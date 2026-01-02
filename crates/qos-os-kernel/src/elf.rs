#![allow(dead_code)]

use x86_64::VirtAddr;

#[derive(Debug)]
pub enum ElfError {
    BadMagic,
    Unsupported,
    Truncated,
    BadPhdr,
}

const EI_NIDENT: usize = 16;
const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;

const EM_X86_64: u16 = 62;

const PT_LOAD: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Ehdr {
    e_ident: [u8; EI_NIDENT],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub struct ElfInfo {
    pub entry: VirtAddr,
    pub phnum: u16,
}

pub fn parse_elf64(bytes: &[u8]) -> Result<ElfInfo, ElfError> {
    if bytes.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(ElfError::Truncated);
    }
    let eh = unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const Elf64Ehdr) };

    if eh.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if eh.e_ident[4] != ELFCLASS64 || eh.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::Unsupported);
    }
    if eh.e_machine != EM_X86_64 {
        return Err(ElfError::Unsupported);
    }
    if eh.e_phentsize as usize != core::mem::size_of::<Elf64Phdr>() {
        return Err(ElfError::Unsupported);
    }

    let phoff = eh.e_phoff as usize;
    let phnum = eh.e_phnum as usize;
    let total_ph = phoff
        .checked_add(phnum.checked_mul(core::mem::size_of::<Elf64Phdr>()).ok_or(ElfError::BadPhdr)?)
        .ok_or(ElfError::BadPhdr)?;
    if total_ph > bytes.len() {
        return Err(ElfError::Truncated);
    }

    Ok(ElfInfo {
        entry: VirtAddr::new(eh.e_entry),
        phnum: eh.e_phnum,
    })
}

pub(crate) fn iter_load_segments<'a>(bytes: &'a [u8]) -> Result<impl Iterator<Item = Elf64Phdr> + 'a, ElfError> {
    if bytes.len() < core::mem::size_of::<Elf64Ehdr>() {
        return Err(ElfError::Truncated);
    }
    let eh = unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const Elf64Ehdr) };

    if eh.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::BadMagic);
    }
    if eh.e_ident[4] != ELFCLASS64 || eh.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::Unsupported);
    }
    if eh.e_machine != EM_X86_64 {
        return Err(ElfError::Unsupported);
    }
    if eh.e_phentsize as usize != core::mem::size_of::<Elf64Phdr>() {
        return Err(ElfError::Unsupported);
    }

    let phoff = eh.e_phoff as usize;
    let phnum = eh.e_phnum as usize;
    let phdr_bytes = phnum
        .checked_mul(core::mem::size_of::<Elf64Phdr>())
        .ok_or(ElfError::BadPhdr)?;
    let end = phoff.checked_add(phdr_bytes).ok_or(ElfError::BadPhdr)?;
    if end > bytes.len() {
        return Err(ElfError::Truncated);
    }

    Ok((0..phnum).filter_map(move |i| {
        let off = phoff + i * core::mem::size_of::<Elf64Phdr>();
        let ph = unsafe { core::ptr::read_unaligned(bytes[off..].as_ptr() as *const Elf64Phdr) };
        if ph.p_type == PT_LOAD {
            Some(ph)
        } else {
            None
        }
    }))
}
