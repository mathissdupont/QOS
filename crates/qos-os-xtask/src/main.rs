use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use xshell::{cmd, Shell};

const KERNEL_PKG: &str = "os";
const TARGET: &str = "x86_64-unknown-none";

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let sub = args.next().unwrap_or_else(|| "help".to_string());

    match sub.as_str() {
        "run" => run(),
        "run-fs" => run_fs(),
        "fs-mkfs" => fs_mkfs(),
        "fs-put" => {
            let host_path = args.next().context("usage: fs-put <host_path> <disk_name>")?;
            let disk_name = args.next().context("usage: fs-put <host_path> <disk_name>")?;
            fs_put(Path::new(&host_path), disk_name.as_bytes())
        }
        "build" => build(),
        "iso" => iso(),
        "verify" => verify(),
        "help" | "-h" | "--help" => {
            eprintln!(
                "qos-os-xtask\n\nCommands:\n  run      Run kernel in QEMU (via bootimage runner)\n  run-fs   Run kernel in QEMU with a second disk (target/qos-fs.img)\n  fs-mkfs  Format target/qos-fs.img with QOS diskfs\n  fs-put   Copy a host file into target/qos-fs.img (diskfs)\n  build    Build kernel\n  iso      Build a bootable disk image (and print ISO guidance)\n  verify   Headless QEMU run + serial log assertion\n"
            );
            Ok(())
        }
        other => bail!("unknown command: {other}"),
    }
}

// --- Host-side helpers for kernel diskfs format (target/qos-fs.img) ---

const SECTOR: usize = 512;

const MAGIC: &[u8; 8] = b"QOSFS1\0\0";
const VERSION: u32 = 1;

const DIR_SECTORS: u32 = 8;
const DIR_START: u32 = 1;
const DATA_START: u32 = DIR_START + DIR_SECTORS;

const MAX_FILES: usize = 32;
const NAME_MAX: usize = 32;
const MAX_FILE_BYTES: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Superblock {
    magic: [u8; 8],
    version: u32,
    dir_start: u32,
    dir_sectors: u32,
    data_start: u32,
    next_free_lba: u32,
    _pad: [u8; 512 - 8 - 5 * 4],
}

impl Superblock {
    fn new() -> Self {
        Self {
            magic: *MAGIC,
            version: VERSION,
            dir_start: DIR_START,
            dir_sectors: DIR_SECTORS,
            data_start: DATA_START,
            next_free_lba: DATA_START,
            _pad: [0; 512 - 8 - 5 * 4],
        }
    }

    fn is_valid(&self) -> bool {
        self.magic == *MAGIC
            && self.version == VERSION
            && self.dir_start == DIR_START
            && self.dir_sectors == DIR_SECTORS
            && self.data_start == DATA_START
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct DirEntry {
    used: u8,
    name_len: u8,
    _rsv: [u8; 2],
    name: [u8; NAME_MAX],
    start_lba: u32,
    size: u32,
    _pad: [u8; 64 - 1 - 1 - 2 - NAME_MAX - 4 - 4],
}

impl DirEntry {
    const fn empty() -> Self {
        Self {
            used: 0,
            name_len: 0,
            _rsv: [0; 2],
            name: [0; NAME_MAX],
            start_lba: 0,
            size: 0,
            _pad: [0; 64 - 1 - 1 - 2 - NAME_MAX - 4 - 4],
        }
    }

    fn name_bytes(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
}

fn fs_img_path() -> PathBuf {
    PathBuf::from("target").join("qos-fs.img")
}

fn read_sector_file(img: &mut fs::File, lba: u32, out: &mut [u8; 512]) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    img.seek(SeekFrom::Start(lba as u64 * SECTOR as u64))?;
    img.read_exact(out)?;
    Ok(())
}

fn write_sector_file(img: &mut fs::File, lba: u32, data: &[u8; 512]) -> Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    img.seek(SeekFrom::Start(lba as u64 * SECTOR as u64))?;
    img.write_all(data)?;
    Ok(())
}

fn read_superblock(img: &mut fs::File) -> Result<Option<Superblock>> {
    let mut sec = [0u8; 512];
    read_sector_file(img, 0, &mut sec)?;
    let sb = unsafe { core::ptr::read_unaligned(sec.as_ptr() as *const Superblock) };
    if sb.is_valid() {
        Ok(Some(sb))
    } else {
        Ok(None)
    }
}

fn write_superblock(img: &mut fs::File, sb: &Superblock) -> Result<()> {
    let mut sec = [0u8; 512];
    unsafe {
        core::ptr::write_unaligned(sec.as_mut_ptr() as *mut Superblock, *sb);
    }
    write_sector_file(img, 0, &sec)
}

fn read_dir_table(img: &mut fs::File, sb: &Superblock) -> Result<[DirEntry; MAX_FILES]> {
    let mut buf = [0u8; (DIR_SECTORS as usize) * 512];
    for i in 0..(DIR_SECTORS as usize) {
        let mut sec = [0u8; 512];
        read_sector_file(img, sb.dir_start + i as u32, &mut sec)?;
        buf[i * 512..(i + 1) * 512].copy_from_slice(&sec);
    }
    let mut table = [DirEntry::empty(); MAX_FILES];
    for i in 0..MAX_FILES {
        let off = i * 64;
        table[i] = unsafe { core::ptr::read_unaligned(buf[off..].as_ptr() as *const DirEntry) };
    }
    Ok(table)
}

fn write_dir_table(img: &mut fs::File, sb: &Superblock, table: &[DirEntry; MAX_FILES]) -> Result<()> {
    let mut buf = [0u8; (DIR_SECTORS as usize) * 512];
    for i in 0..MAX_FILES {
        let off = i * 64;
        unsafe {
            core::ptr::write_unaligned(buf[off..].as_mut_ptr() as *mut DirEntry, table[i]);
        }
    }
    for i in 0..(DIR_SECTORS as usize) {
        let mut sec = [0u8; 512];
        sec.copy_from_slice(&buf[i * 512..(i + 1) * 512]);
        write_sector_file(img, sb.dir_start + i as u32, &sec)?;
    }
    Ok(())
}

fn find_entry(table: &[DirEntry; MAX_FILES], name: &[u8]) -> Option<usize> {
    table.iter().position(|e| e.used != 0 && e.name_bytes() == name)
}

fn find_free(table: &[DirEntry; MAX_FILES]) -> Option<usize> {
    table.iter().position(|e| e.used == 0)
}

fn fs_mkfs() -> Result<()> {
    let img_path = fs_img_path();
    ensure_min_len(&img_path, 16 * 1024 * 1024).context("failed to create/pad qos-fs.img")?;

    let mut img = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&img_path)
        .with_context(|| format!("open {}", img_path.display()))?;

    let sb = Superblock::new();
    write_superblock(&mut img, &sb)?;
    let table = [DirEntry::empty(); MAX_FILES];
    write_dir_table(&mut img, &sb, &table)?;
    eprintln!("diskfs formatted: {}", img_path.display());
    Ok(())
}

fn fs_put(host_path: &Path, disk_name: &[u8]) -> Result<()> {
    if disk_name.is_empty() || disk_name.len() > NAME_MAX {
        bail!("disk_name must be 1..={NAME_MAX} bytes");
    }
    let data = fs::read(host_path).with_context(|| format!("read {}", host_path.display()))?;
    if data.len() > MAX_FILE_BYTES {
        bail!("file too large for diskfs ({} > {})", data.len(), MAX_FILE_BYTES);
    }

    let img_path = fs_img_path();
    ensure_min_len(&img_path, 16 * 1024 * 1024).context("failed to create/pad qos-fs.img")?;
    let mut img = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&img_path)
        .with_context(|| format!("open {}", img_path.display()))?;

    let mut sb = read_superblock(&mut img)?.unwrap_or_else(Superblock::new);
    if !sb.is_valid() {
        // If it wasn't formatted, format it now.
        sb = Superblock::new();
        write_superblock(&mut img, &sb)?;
        let table = [DirEntry::empty(); MAX_FILES];
        write_dir_table(&mut img, &sb, &table)?;
    }

    let mut table = read_dir_table(&mut img, &sb)?;
    let idx = if let Some(i) = find_entry(&table, disk_name) {
        i
    } else {
        find_free(&table).context("no free directory slots")?
    };

    // Append-only allocator: write at next_free_lba.
    let start_lba = sb.next_free_lba;
    let sectors = ((data.len() + 511) / 512) as u32;
    for i in 0..sectors {
        let mut sec = [0u8; 512];
        let off = (i as usize) * 512;
        let take = std::cmp::min(512, data.len().saturating_sub(off));
        sec[..take].copy_from_slice(&data[off..off + take]);
        write_sector_file(&mut img, start_lba + i, &sec)?;
    }

    let mut e = DirEntry::empty();
    e.used = 1;
    e.name_len = disk_name.len() as u8;
    e.name[..disk_name.len()].copy_from_slice(disk_name);
    e.start_lba = start_lba;
    e.size = data.len() as u32;
    table[idx] = e;

    sb.next_free_lba = start_lba + sectors;
    write_dir_table(&mut img, &sb, &table)?;
    write_superblock(&mut img, &sb)?;

    eprintln!(
        "fs-put ok: {} -> {}/{} ({} bytes)",
        host_path.display(),
        img_path.display(),
        String::from_utf8_lossy(disk_name),
        data.len()
    );
    Ok(())
}

fn run_fs() -> Result<()> {
    let sh = Shell::new()?;

    // Build an interactive bootimage that supports the Ring3 shared-memory syscall ABI.
    // NOTE: Do not enable `userdemo` here (it auto-enters Ring3 at boot and never shows the shell).
    cmd!(sh, "cargo bootimage -p {KERNEL_PKG} --target {TARGET} --features userabi")
        .run()
        .context("cargo bootimage failed")?;

    let bootimage = bootimage_path();
    if !bootimage.exists() {
        bail!("bootimage not found at {}", bootimage.display());
    }

    // Ensure an FS disk image exists inside the workspace (safe to write).
    let fs_img = PathBuf::from("target").join("qos-fs.img");
    ensure_min_len(&fs_img, 16 * 1024 * 1024).context("failed to create/pad qos-fs.img")?;

    let qemu = qemu_exe();
    if !qemu.exists() {
        bail!(
            "QEMU not found at {} (edit qos-os-kernel Cargo.toml or set QEMU_EXE)",
            qemu.display()
        );
    }

    let bootimage = qemu_rel_path(&bootimage);
    let fs_img = qemu_rel_path(&fs_img);

    // Interactive windowed QEMU: COM1 to stdio so you still see serial.
    let status = Command::new(qemu)
        .arg("-drive")
        .arg(format!("if=ide,index=0,media=disk,format=raw,file={bootimage}"))
        .arg("-drive")
        .arg(format!("if=ide,index=1,media=disk,format=raw,file={fs_img}"))
        .arg("-device")
        .arg("isa-debug-exit,iobase=0xf4,iosize=0x04")
        .arg("-boot")
        .arg("order=c")
        .arg("-serial")
        .arg("stdio")
        .status()
        .context("qemu run failed")?;

    // When the guest exits via `isa-debug-exit`, QEMU returns code 33 for our exit value (0x10).
    // Treat that as a normal exit so interactive runs don't look like failures.
    let ok = status.success() || status.code() == Some(33);
    if !ok {
        bail!("qemu exited with {status}");
    }
    Ok(())
}

fn build() -> Result<()> {
    let sh = Shell::new()?;
    cmd!(sh, "cargo build -p {KERNEL_PKG} --target {TARGET}")
        .run()
        .context("kernel build failed")?;
    Ok(())
}

fn run() -> Result<()> {
    let sh = Shell::new()?;
    cmd!(sh, "cargo run -p {KERNEL_PKG} --target {TARGET}")
        .run()
        .context("kernel run failed")?;
    Ok(())
}

fn iso() -> Result<()> {
    let sh = Shell::new()?;

    eprintln!("Building bootable disk image using cargo-bootimage...");
    cmd!(sh, "cargo bootimage -p {KERNEL_PKG} --target {TARGET}")
        .run()
        .context("cargo bootimage failed")?;

    eprintln!(
        "\nOutput: target/{TARGET}/debug/bootimage-{KERNEL_PKG}.bin\n\nRun in QEMU:\n  cargo run -p {KERNEL_PKG} --target {TARGET}\n\nVirtualBox (Windows) quick path (no ISO yet):\n  VBoxManage convertfromraw --format VDI target\\{TARGET}\\debug\\bootimage-{KERNEL_PKG}.bin qos-os.vdi\n\nISO note:\n- Milestone 0 delivers a bootable *raw disk image*.\n- Milestone 1 will add a real BIOS+UEFI ISO pipeline (Limine) so VirtualBox can boot directly from ISO.\n"
    );

    Ok(())
}

fn verify() -> Result<()> {
    let sh = Shell::new()?;

    // Verify should exercise the Ring3 syscall path too.
    cmd!(sh, "cargo bootimage -p {KERNEL_PKG} --target {TARGET} --features verify,userdemo")
        .run()
        .context("cargo bootimage failed")?;

    let bootimage = bootimage_path();
    if !bootimage.exists() {
        bail!("bootimage not found at {}", bootimage.display());
    }

    // Some BIOS implementations (and QEMU/SeaBIOS combos) behave poorly with very small raw disks
    // (often reporting them as 0 MiB). Padding to a small minimum makes boot behavior more stable.
    ensure_min_len(&bootimage, 2 * 1024 * 1024).context("failed to pad bootimage")?;

    let qemu = qemu_exe();
    if !qemu.exists() {
        bail!(
            "QEMU not found at {} (edit qos-os-kernel Cargo.toml or set QEMU_EXE)",
            qemu.display()
        );
    }

    let log_path = PathBuf::from("target").join("qos-os-serial.log");
    let _ = fs::remove_file(&log_path);

    let debugcon_path = PathBuf::from("target").join("qos-os-debugcon.log");
    let _ = fs::remove_file(&debugcon_path);

    let bios_debugcon_path = PathBuf::from("target").join("qos-os-bios-debugcon.log");
    let _ = fs::remove_file(&bios_debugcon_path);

    let qemu_log_path = PathBuf::from("target").join("qemu-verify.log");
    let _ = fs::remove_file(&qemu_log_path);

    // Run QEMU headless and capture COM1 to a file for deterministic verification.
    let mut child = spawn_qemu_headless(
        &qemu,
        &bootimage,
        &log_path,
        &debugcon_path,
        &bios_debugcon_path,
        &qemu_log_path,
    )
        .context("failed to start QEMU")?;

    // Wait for the kernel to exit QEMU via isa-debug-exit (flushes log files).
    let mut elapsed = Duration::from_millis(0);
    let timeout = Duration::from_secs(20);
    while elapsed < timeout {
        if let Some(_status) = child.try_wait()? {
            break;
        }
        thread::sleep(Duration::from_millis(100));
        elapsed += Duration::from_millis(100);
    }

    let exited = child.try_wait()?.is_some();
    if !exited {
        // Fallback: kill so dev shells don't hang.
        let _ = child.kill();
        let _ = child.wait();
    }

    let qemu_stdout = read_child_pipe(&mut child, Pipe::Stdout).unwrap_or_default();
    let qemu_stderr = read_child_pipe(&mut child, Pipe::Stderr).unwrap_or_default();

    let qemu_log = fs::read_to_string(&qemu_log_path).unwrap_or_default();

    let bios_debugcon_log = fs::read_to_string(&bios_debugcon_path).unwrap_or_default();

    let qemu_diag = format!(
        "QEMU exited_gracefully={exited}\n\n--- qemu stdout ---\n{}\n--- end ---\n\n--- qemu stderr ---\n{}\n--- end ---\n\n--- qemu log ({}) ---\n{}\n--- end ---\n",
        qemu_stdout,
        qemu_stderr
        ,qemu_log_path.display()
        ,qemu_log
    );

    let serial_log = fs::read_to_string(&log_path).unwrap_or_default();
    let debugcon_log = fs::read_to_string(&debugcon_path).unwrap_or_default();

    // Prefer debugcon (0xE9) on Windows; it's much more deterministic than COM1 file flushing.
    let log: &str = if !debugcon_log.trim().is_empty() {
        &debugcon_log
    } else {
        &serial_log
    };

    let must_have = [
        "QOS-OS boot OK",
        "heap initialized",
        "heap test ok",
        "entering user mode",
        "syscall abi: SubmitIr",
        "syscall abi: GetStatus",
        // Note: "-> Running" is not required in verify mode since mock completes instantly
        "-> Done",
        "syscall abi: GetResult",
        "handle=2",
        "VERIFY: quantum demo ok (ring3)",
    ];

    for s in must_have {
        if !log.contains(s) {
            bail!(
                "verification failed: missing '{s}' in captured log (debugcon: {}, serial: {})\n\n--- captured log ---\n{log}\n--- end ---\n\n--- bios debugcon ({}) ---\n{}\n--- end ---\n\n{qemu_diag}",
                debugcon_path.display(),
                log_path.display(),
                bios_debugcon_path.display(),
                bios_debugcon_log
            );
        }
    }

    if !qemu_stdout.trim().is_empty() {
        eprintln!("QEMU stdout:\n{qemu_stdout}");
    }
    if !qemu_stderr.trim().is_empty() {
        eprintln!("QEMU stderr:\n{qemu_stderr}");
    }

    if debugcon_log.trim().is_empty() {
        eprintln!("verify OK (serial): {}", log_path.display());
    } else {
        eprintln!("verify OK (debugcon): {}", debugcon_path.display());
    }
    Ok(())
}

fn ensure_min_len(path: &Path, min_len: u64) -> Result<()> {
    let len = match fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e).with_context(|| format!("stat {}", path.display())),
    };

    if len >= min_len {
        return Ok(());
    }

    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(path)
        .with_context(|| format!("open/create {}", path.display()))?;
    f.set_len(min_len)
        .with_context(|| format!("set_len {} -> {min_len}", path.display()))?;
    Ok(())
}

fn bootimage_path() -> PathBuf {
    PathBuf::from("target")
        .join(TARGET)
        .join("debug")
        .join(format!("bootimage-{KERNEL_PKG}.bin"))
}

fn qemu_exe() -> PathBuf {
    if let Ok(path) = env::var("QEMU_EXE") {
        return PathBuf::from(path);
    }
    PathBuf::from(r"C:/Program Files/qemu/qemu-system-x86_64.exe")
}

fn qemu_uefi_firmware() -> Option<PathBuf> {
    // Opt-in only: BIOS boot is the default.
    if env::var("QOS_USE_UEFI").ok().as_deref() != Some("1") {
        return None;
    }

    if let Ok(path) = env::var("QEMU_UEFI_CODE") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
        return None;
    }

    let p = PathBuf::from(r"C:/Program Files/qemu/share/edk2-x86_64-code.fd");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn qemu_uefi_vars_template() -> Option<PathBuf> {
    if let Ok(path) = env::var("QEMU_UEFI_VARS") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
        return None;
    }

    let p = PathBuf::from(r"C:/Program Files/qemu/share/edk2-i386-vars.fd");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn ensure_uefi_vars_image() -> Result<Option<PathBuf>> {
    let code = match qemu_uefi_firmware() {
        Some(c) => c,
        None => return Ok(None),
    };

    let vars_path = PathBuf::from("target").join("edk2-vars.fd");
    if !vars_path.exists() {
        if let Some(template) = qemu_uefi_vars_template() {
            fs::create_dir_all("target").ok();
            fs::copy(&template, &vars_path).with_context(|| {
                format!(
                    "copy UEFI vars template {} -> {}",
                    template.display(),
                    vars_path.display()
                )
            })?;
        } else {
            // Create a minimal writable vars image if no template exists.
            let f = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&vars_path)
                .with_context(|| format!("create {}", vars_path.display()))?;
            f.set_len(2 * 1024 * 1024)
                .with_context(|| format!("set_len {}", vars_path.display()))?;
        }
    }

    // Return both code and vars paths (code existence already checked).
    let _ = code; // keep code check for symmetry; code is re-read in spawn.
    Ok(Some(vars_path))
}

fn spawn_qemu_headless(
    qemu: &Path,
    bootimage: &Path,
    log_path: &Path,
    debugcon_path: &Path,
    bios_debugcon_path: &Path,
    qemu_log_path: &Path,
) -> Result<Child> {
    let bootimage = qemu_rel_path(bootimage);
    let log_path = qemu_rel_path(log_path);
    let debugcon_path = qemu_rel_path(debugcon_path);
    let bios_debugcon_path = qemu_rel_path(bios_debugcon_path);
    let qemu_log_path = qemu_rel_path(qemu_log_path);

    let mut cmd = Command::new(qemu);

    // If EDK2 firmware exists, boot via pflash (UEFI).
    if let Some(code) = qemu_uefi_firmware() {
        let _vars = ensure_uefi_vars_image().ok().flatten();
        let vars = qemu_rel_path(&PathBuf::from("target").join("edk2-vars.fd"));
        let code = qemu_rel_path(&code);
        cmd.arg("-drive")
            .arg(format!("if=pflash,format=raw,readonly=on,file={code}"))
            .arg("-drive")
            .arg(format!("if=pflash,format=raw,file={vars}"));
    }

    cmd.arg("-drive")
        .arg(format!("if=ide,index=0,media=disk,format=raw,file={bootimage}"))
        .arg("-boot")
        .arg("order=c")
        .arg("-nographic")
        .arg("-d")
        .arg("cpu_reset,int,guest_errors")
        .arg("-D")
        .arg(qemu_log_path)
        .arg("-device")
        .arg("isa-debug-exit,iobase=0xf4,iosize=0x04")
        // Capture BIOS/SeaBIOS (often uses 0x402) to separate file for diagnostics.
        .arg("-chardev")
        .arg(format!("file,id=bioscon,path={bios_debugcon_path}"))
        .arg("-device")
        .arg("isa-debugcon,iobase=0x402,chardev=bioscon")
        // Capture kernel debugcon (0xE9) for deterministic verification signal.
        .arg("-chardev")
        .arg(format!("file,id=debugcon,path={debugcon_path}"))
        .arg("-device")
        .arg("isa-debugcon,iobase=0xE9,chardev=debugcon")
        .arg("-serial")
        .arg(format!("file:{log_path}"))
        .arg("-no-reboot")
        ;

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd.spawn().context("qemu spawn failed")
}

enum Pipe {
    Stdout,
    Stderr,
}

fn read_child_pipe(child: &mut Child, which: Pipe) -> Result<String> {
    use std::io::Read;

    let mut buf = String::new();
    match which {
        Pipe::Stdout => {
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_string(&mut buf);
            }
        }
        Pipe::Stderr => {
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_string(&mut buf);
            }
        }
    }
    Ok(buf)
}

fn qemu_rel_path(p: &Path) -> String {
    // Intentionally avoid canonical/absolute paths here:
    // - canonical paths on Windows often include a `\\?\` prefix
    // - our workspace path may contain non-ASCII characters
    // Both can break QEMU path parsing for `-drive ... file=...`.
    p.to_string_lossy().replace('\\', "/")
}
