//! Extended System Call Interface for QOS
//!
//! POSIX-like file operations: open, read, write, close, lseek

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Maximum open files per process
const MAX_OPEN_FILES: usize = 64;

/// File descriptor entry
#[derive(Clone)]
pub struct FileDescriptor {
    pub path: String,
    pub position: u64,
    pub flags: u32,
    pub is_open: bool,
}

impl FileDescriptor {
    pub const fn empty() -> Self {
        Self {
            path: String::new(),
            position: 0,
            flags: 0,
            is_open: false,
        }
    }
}

/// File descriptor table
pub struct FdTable {
    fds: [Option<FileDescriptor>; MAX_OPEN_FILES],
    next_fd: usize,
}

impl FdTable {
    pub fn new() -> Self {
        const NONE: Option<FileDescriptor> = None;
        let mut table = Self {
            fds: [NONE; MAX_OPEN_FILES],
            next_fd: 3,
        };
        
        // Standard file descriptors
        table.fds[0] = Some(FileDescriptor {
            path: String::from("/dev/stdin"),
            position: 0,
            flags: O_RDONLY,
            is_open: true,
        });
        table.fds[1] = Some(FileDescriptor {
            path: String::from("/dev/stdout"),
            position: 0,
            flags: O_WRONLY,
            is_open: true,
        });
        table.fds[2] = Some(FileDescriptor {
            path: String::from("/dev/stderr"),
            position: 0,
            flags: O_WRONLY,
            is_open: true,
        });
        
        table
    }
    
    pub fn alloc(&mut self) -> Option<usize> {
        for i in self.next_fd..MAX_OPEN_FILES {
            if self.fds[i].is_none() {
                self.next_fd = i + 1;
                return Some(i);
            }
        }
        for i in 3..self.next_fd {
            if self.fds[i].is_none() {
                self.next_fd = i + 1;
                return Some(i);
            }
        }
        None
    }
    
    pub fn get(&self, fd: usize) -> Option<&FileDescriptor> {
        self.fds.get(fd).and_then(|f| f.as_ref())
    }
    
    pub fn get_mut(&mut self, fd: usize) -> Option<&mut FileDescriptor> {
        self.fds.get_mut(fd).and_then(|f| f.as_mut())
    }
    
    pub fn set(&mut self, fd: usize, desc: FileDescriptor) {
        if fd < MAX_OPEN_FILES {
            self.fds[fd] = Some(desc);
        }
    }
    
    pub fn close(&mut self, fd: usize) -> bool {
        if fd >= 3 && fd < MAX_OPEN_FILES {
            if self.fds[fd].is_some() {
                self.fds[fd] = None;
                return true;
            }
        }
        false
    }
}

/// Global FD table
static FD_TABLE: Mutex<Option<FdTable>> = Mutex::new(None);

/// Syscall numbers
pub mod syscall_nr {
    pub const SYS_READ: u64 = 0;
    pub const SYS_WRITE: u64 = 1;
    pub const SYS_OPEN: u64 = 2;
    pub const SYS_CLOSE: u64 = 3;
    pub const SYS_LSEEK: u64 = 8;
    pub const SYS_GETPID: u64 = 39;
    pub const SYS_EXIT: u64 = 60;
    pub const SYS_GETCWD: u64 = 79;
    pub const SYS_CHDIR: u64 = 80;
    pub const SYS_TIME: u64 = 201;
    
    // QOS quantum syscalls
    pub const SYS_QUANTUM_SUBMIT: u64 = 500;
    pub const SYS_QUANTUM_STATUS: u64 = 501;
    pub const SYS_QUANTUM_RESULT: u64 = 502;
}

/// Open flags
pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0x40;
pub const O_TRUNC: u32 = 0x200;
pub const O_APPEND: u32 = 0x400;

/// Seek whence
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

/// Error codes
pub const ENOENT: i64 = -2;
pub const EBADF: i64 = -9;
pub const EACCES: i64 = -13;
pub const EINVAL: i64 = -22;
pub const EMFILE: i64 = -24;
pub const ENOSPC: i64 = -28;
pub const ENOSYS: i64 = -38;

/// Initialize syscall subsystem
pub fn init() {
    let mut table = FD_TABLE.lock();
    *table = Some(FdTable::new());
    crate::serial_println!("[SYSCALL] Initialized");
}

/// Check if file exists using VFS
fn file_exists(path: &str) -> bool {
    crate::vfs::read(path.as_bytes()).is_ok()
}

/// Open a file
pub fn sys_open(path: &str, flags: u32, _mode: u32) -> i64 {
    let mut table_guard = FD_TABLE.lock();
    let table = match table_guard.as_mut() {
        Some(t) => t,
        None => return ENOSYS,
    };
    
    let exists = file_exists(path);
    
    if flags & O_CREAT != 0 && !exists {
        // Create empty file
        if crate::vfs::write(path.as_bytes(), &[]).is_err() {
            return ENOSPC;
        }
    } else if !exists {
        return ENOENT;
    }
    
    // Truncate if requested
    if flags & O_TRUNC != 0 {
        let _ = crate::vfs::write(path.as_bytes(), &[]);
    }
    
    // Allocate FD
    let fd = match table.alloc() {
        Some(f) => f,
        None => return EMFILE,
    };
    
    // Get size for append mode
    let size = crate::vfs::read(path.as_bytes())
        .map(|d| d.len() as u64)
        .unwrap_or(0);
    
    table.set(fd, FileDescriptor {
        path: String::from(path),
        position: if flags & O_APPEND != 0 { size } else { 0 },
        flags,
        is_open: true,
    });
    
    fd as i64
}

/// Close file
pub fn sys_close(fd: usize) -> i64 {
    let mut table_guard = FD_TABLE.lock();
    let table = match table_guard.as_mut() {
        Some(t) => t,
        None => return ENOSYS,
    };
    
    if table.close(fd) { 0 } else { EBADF }
}

/// Read from file
pub fn sys_read(fd: usize, buf: &mut [u8]) -> i64 {
    let mut table_guard = FD_TABLE.lock();
    let table = match table_guard.as_mut() {
        Some(t) => t,
        None => return ENOSYS,
    };
    
    let desc = match table.get_mut(fd) {
        Some(d) if d.is_open => d,
        _ => return EBADF,
    };
    
    // stdin - return 0 (no blocking read)
    if fd == 0 {
        return 0;
    }
    
    // Read from VFS
    let data = match crate::vfs::read(desc.path.as_bytes()) {
        Ok(d) => d,
        Err(_) => return ENOENT,
    };
    
    let pos = desc.position as usize;
    if pos >= data.len() {
        return 0; // EOF
    }
    
    let available = data.len() - pos;
    let to_read = core::cmp::min(buf.len(), available);
    buf[..to_read].copy_from_slice(&data[pos..pos + to_read]);
    desc.position += to_read as u64;
    
    to_read as i64
}

/// Write to file
pub fn sys_write(fd: usize, buf: &[u8]) -> i64 {
    let mut table_guard = FD_TABLE.lock();
    let table = match table_guard.as_mut() {
        Some(t) => t,
        None => return ENOSYS,
    };
    
    let desc = match table.get_mut(fd) {
        Some(d) if d.is_open => d,
        _ => return EBADF,
    };
    
    // stdout/stderr
    if fd == 1 || fd == 2 {
        for &b in buf {
            crate::print!("{}", b as char);
        }
        return buf.len() as i64;
    }
    
    // Check write permission
    if desc.flags & O_WRONLY == 0 && desc.flags & O_RDWR == 0 {
        return EACCES;
    }
    
    // Read existing data
    let mut data = crate::vfs::read(desc.path.as_bytes()).unwrap_or_default();
    
    let pos = if desc.flags & O_APPEND != 0 {
        data.len()
    } else {
        desc.position as usize
    };
    
    // Extend if necessary
    if pos + buf.len() > data.len() {
        data.resize(pos + buf.len(), 0);
    }
    
    data[pos..pos + buf.len()].copy_from_slice(buf);
    
    if crate::vfs::write(desc.path.as_bytes(), &data).is_err() {
        return ENOSPC;
    }
    
    desc.position = (pos + buf.len()) as u64;
    buf.len() as i64
}

/// Seek in file
pub fn sys_lseek(fd: usize, offset: i64, whence: i32) -> i64 {
    let mut table_guard = FD_TABLE.lock();
    let table = match table_guard.as_mut() {
        Some(t) => t,
        None => return ENOSYS,
    };
    
    let desc = match table.get_mut(fd) {
        Some(d) if d.is_open => d,
        _ => return EBADF,
    };
    
    let size = crate::vfs::read(desc.path.as_bytes())
        .map(|d| d.len() as i64)
        .unwrap_or(0);
    
    let new_pos = match whence {
        SEEK_SET => offset,
        SEEK_CUR => desc.position as i64 + offset,
        SEEK_END => size + offset,
        _ => return EINVAL,
    };
    
    if new_pos < 0 {
        return EINVAL;
    }
    
    desc.position = new_pos as u64;
    new_pos
}

/// Get time
pub fn sys_time() -> i64 {
    crate::rtc::unix_time() as i64
}

/// Get PID (always 1 for kernel)
pub fn sys_getpid() -> i64 {
    1
}

/// Current working directory
static CWD: Mutex<String> = Mutex::new(String::new());

/// Get current directory
pub fn sys_getcwd(buf: &mut [u8]) -> i64 {
    let cwd = CWD.lock();
    let path = if cwd.is_empty() { "/" } else { cwd.as_str() };
    
    if buf.len() < path.len() + 1 {
        return EINVAL;
    }
    
    buf[..path.len()].copy_from_slice(path.as_bytes());
    buf[path.len()] = 0;
    
    path.len() as i64
}

/// Change directory
pub fn sys_chdir(path: &str) -> i64 {
    let mut cwd = CWD.lock();
    *cwd = String::from(path);
    0
}

/// Unlink (delete) file
pub fn sys_unlink(path: &str) -> i64 {
    match crate::vfs::remove(path.as_bytes()) {
        Ok(_) => 0,
        Err(_) => ENOENT,
    }
}

/// Handle syscall by number
pub fn handle_syscall(nr: u64, arg1: u64, arg2: u64, arg3: u64) -> i64 {
    match nr {
        syscall_nr::SYS_READ => {
            let fd = arg1 as usize;
            let buf_ptr = arg2 as *mut u8;
            let count = arg3 as usize;
            if buf_ptr.is_null() {
                return EINVAL;
            }
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr, count) };
            sys_read(fd, buf)
        }
        syscall_nr::SYS_WRITE => {
            let fd = arg1 as usize;
            let buf_ptr = arg2 as *const u8;
            let count = arg3 as usize;
            if buf_ptr.is_null() {
                return EINVAL;
            }
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr, count) };
            sys_write(fd, buf)
        }
        syscall_nr::SYS_CLOSE => sys_close(arg1 as usize),
        syscall_nr::SYS_LSEEK => sys_lseek(arg1 as usize, arg2 as i64, arg3 as i32),
        syscall_nr::SYS_GETPID => sys_getpid(),
        syscall_nr::SYS_TIME => sys_time(),
        _ => ENOSYS,
    }
}
