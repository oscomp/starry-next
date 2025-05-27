use axerrno::{LinuxError, LinuxResult};
use axhal::paging::MappingFlags;
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::{
    MAP_ANONYMOUS, MAP_FIXED, MAP_NORESERVE, MAP_PRIVATE, MAP_SHARED, MAP_STACK, MAP_POPULATE, PROT_EXEC,
    PROT_GROWSDOWN, PROT_GROWSUP, PROT_READ, PROT_WRITE,
};
use memory_addr::{VirtAddr, VirtAddrRange, PAGE_SIZE_4K};

use crate::file::{File, FileLike};

bitflags::bitflags! {
    /// `PROT_*` flags for use with [`sys_mmap`].
    ///
    /// For `PROT_NONE`, use `ProtFlags::empty()`.
    #[derive(Debug)]
    struct MmapProt: u32 {
        /// Page can be read.
        const READ = PROT_READ;
        /// Page can be written.
        const WRITE = PROT_WRITE;
        /// Page can be executed.
        const EXEC = PROT_EXEC;
        /// Extend change to start of growsdown vma (mprotect only).
        const GROWDOWN = PROT_GROWSDOWN;
        /// Extend change to start of growsup vma (mprotect only).
        const GROWSUP = PROT_GROWSUP;
    }
}

impl From<MmapProt> for MappingFlags {
    fn from(value: MmapProt) -> Self {
        let mut flags = MappingFlags::USER;
        if value.contains(MmapProt::READ) {
            flags |= MappingFlags::READ;
        }
        if value.contains(MmapProt::WRITE) {
            flags |= MappingFlags::WRITE;
        }
        if value.contains(MmapProt::EXEC) {
            flags |= MappingFlags::EXECUTE;
        }
        flags
    }
}

bitflags::bitflags! {
    /// flags for sys_mmap
    ///
    /// See <https://github.com/bminor/glibc/blob/master/bits/mman.h>
    #[derive(Debug)]
    struct MmapFlags: u32 {
        /// Share changes
        const SHARED = MAP_SHARED;
        /// Changes private; copy pages on write.
        const PRIVATE = MAP_PRIVATE;
        /// Map address must be exactly as requested, no matter whether it is available.
        const FIXED = MAP_FIXED;
        /// Don't use a file.
        const ANONYMOUS = MAP_ANONYMOUS;
        /// Don't check for reservations.
        const NORESERVE = MAP_NORESERVE;
        /// Allocation is for a stack.
        const STACK = MAP_STACK;

        const POPULATE = MAP_POPULATE;
    }
}

pub fn sys_mmap(
    addr: usize,
    length: usize,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: isize,
) -> LinuxResult<isize> {
    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mut aspace = process_data.aspace.lock();
    let permission_flags = MmapProt::from_bits_truncate(prot);
    
    // TODO: check illegal flags for mmap
    // An example is the flags contained none of MAP_PRIVATE, MAP_SHARED, or MAP_SHARED_VALIDATE.
    let map_flags = MmapFlags::from_bits_truncate(flags);
    
    let offset = offset as usize;
    if offset % PAGE_SIZE_4K != 0 {
        error!("MAP_FAILED: offset must aligned to 4K");
        return Err(LinuxError::EINVAL);
    }


    let start = memory_addr::align_down_4k(addr);
    let end = memory_addr::align_up_4k(addr + length);
    let aligned_length = end - start;
    let aligned_offset = memory_addr::align_down_4k(offset);
    debug!(
        "start: {:x?}, end: {:x?}, aligned_length: {:x?}",
        start, end, aligned_length
    );

    
    let start_addr = if map_flags.contains(MmapFlags::FIXED) {
        if start == 0 {
            return Err(LinuxError::EINVAL);
        }
        let dst_addr = VirtAddr::from(start);
        aspace.unmap(dst_addr, aligned_length)?;
        dst_addr
    } else {
        aspace
            .find_free_area(
                VirtAddr::from(start),
                aligned_length,
                VirtAddrRange::new(aspace.base(), aspace.end()),
            )
            .or(aspace.find_free_area(
                aspace.base(),
                aligned_length,
                VirtAddrRange::new(aspace.base(), aspace.end()),
            ))
            .ok_or(LinuxError::ENOMEM)?
    };

    let populate = map_flags.contains(MmapFlags::POPULATE);
    let anonymous = map_flags.contains(MmapFlags::ANONYMOUS);
    let shared = map_flags.contains(MmapFlags::SHARED);

    if anonymous {
        if shared {
            // TODO: 匿名共享 => 共享内存
            aspace.map_shm(start_addr, aligned_length, permission_flags.into(), populate)?;
        } else {
            // 匿名私有 => 功能等同 malloc
            aspace.map_alloc(start_addr, aligned_length, permission_flags.into(), populate)?;
        }
    } else {
        // 文件读写
        let file = File::from_fd(fd)?;
        aspace.map_file(start_addr, aligned_length, permission_flags.into(), fd, offset as usize, shared, populate)?;
    }

    info!("mmap: start_addr = {:#x}, length = {:#x}, fd = {}, offset = {:#x}",
        start_addr, aligned_length, fd, offset);

    return Ok(start_addr.as_usize() as _);
}

pub fn sys_munmap(addr: usize, length: usize) -> LinuxResult<isize> {
    sys_msync(addr, length, 0)?;

    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mut aspace = process_data.aspace.lock();
    let length = memory_addr::align_up_4k(length);
    let start_addr = VirtAddr::from(addr);

    aspace.unmap(start_addr, length)?;
    axhal::arch::flush_tlb(None);
    Ok(0)
}

// TODO
pub fn sys_mprotect(addr: usize, length: usize, prot: u32) -> LinuxResult<isize> {
    // TODO: implement PROT_GROWSUP & PROT_GROWSDOWN
    let Some(permission_flags) = MmapProt::from_bits(prot) else {
        return Err(LinuxError::EINVAL);
    };
    if permission_flags.contains(MmapProt::GROWDOWN | MmapProt::GROWSUP) {
        return Err(LinuxError::EINVAL);
    }

    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mut aspace = process_data.aspace.lock();
    let length = memory_addr::align_up_4k(length);
    let start_addr = VirtAddr::from(addr);
    aspace.protect(start_addr, length, permission_flags.into())?;

    Ok(0)
}

pub fn sys_msync(addr: usize, length: usize, flags: isize) -> LinuxResult<isize> {
    if addr % PAGE_SIZE_4K != 0 {
        return Err(LinuxError::EINVAL);
    }
    
    let length = memory_addr::align_up_4k(length);
    let curr = current();
    
    let mut aspace = curr.task_ext().process_data().aspace.lock();
    if let Some((fd, .. )) = aspace.get_file_metadata(VirtAddr::from_usize(addr)) {
        let file = File::from_fd(fd)?;
        let mut page_cache = file.cache();
        page_cache.msync()?;
        return Ok(0);
    }
    
    // 找不到文件不一定是错误，可能是匿名映射，似乎 mallco 也会调用 msync
    Ok(0)
}