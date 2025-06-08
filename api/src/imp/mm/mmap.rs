use axerrno::{LinuxError, LinuxResult};
use axhal::paging::MappingFlags;
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::{
    MAP_ANONYMOUS, MAP_FIXED, MAP_NORESERVE, MAP_POPULATE, MAP_PRIVATE, MAP_SHARED, MAP_STACK, PROT_EXEC, PROT_GROWSDOWN, PROT_GROWSUP, PROT_READ, PROT_WRITE, QNX4_SUPER_MAGIC
};
use memory_addr::{PhysAddr, VirtAddr, VirtAddrRange, PAGE_SIZE_4K};
use alloc::vec;

use crate::file::{page_cache_manager, File, FileLike};

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


/// ### 需要在维护的信息：
/// 1. process_data 里的 aspace，即 axmm 层的 AddrSpace。它维护已分配的虚拟地址段，并分配新地址。
/// 插入在 sys_mmap 时执行，删除在 sys_munmap 时执行。
/// 2. process_data 里的 process_mmap_manager。它的主要作用是在 page fault 时找到相应的 VMA 信息，
/// 包括 fd/start_addr/shared 等等。为了维持底层 Unikernel 的简洁性，将这些内容放在 Starry 层维护。
/// 插入在 sys_mmap 时执行，删除在 sys_munmap 时执行。
/// 3. Page 里的 virt_pages。它的作用是实现物理页的反向映射，用于检查页面是否为脏页，在页面置换的时候
/// 取消所有相关的页表映射。
/// 插入在 page fault 时由 lazy_map_file 执行，删除在 sys_munmap 中执行。
/// 
/// ### 页表的修改：
/// 1. 页表的删除：一律在 AddrSpace::unmap 中执行，取消整个 VMA 的页表映射。
/// 2. 对于非 populate 文件映射，在 page fault 时由 lazy_map_file 建立页表映射。
/// 3. 对于非 populate 匿名映射，在 page fault 时由 Addrspace::handle_page_fault 建立页表映射。
/// 4. 对于 populate 映射，在sys_mmmap 时由 AddrSpace::map_alloc 直接建立页表映射。
/// 
/// ### 根据匿名/文件、私有/共享，主要有 4 种 mmap：
/// 1. 匿名私有：功能等同 malloc，事实上 malloc 底层即调用这种类型的 mmap。
/// 2. 匿名共享：功能等同于 Private 的共享内存，只能在父子进程之间共享。 TODO: 尚未实现。
/// 3. 文件私有：仅将文件内容加载进内存，但是修改不会同步到文件。
/// 4. 文件共享：对文件的修改会被同步，并且允许多个进程并发读写文件。底层会将不同进程的虚拟页面映射到同一个页缓存物理页面。
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

    // 分配虚拟地址段
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
    info!("mmap: start_addr = {:#x}, length = {:#x}, fd = {}, offset = {:#x}",
        start_addr, aligned_length, fd, offset);

    let anonymous = map_flags.contains(MmapFlags::ANONYMOUS) || fd == -1;
    let shared = map_flags.contains(MmapFlags::SHARED);
    let fd = { if anonymous { -1 } else { fd } };
    // 目前只有匿名映射能做到 populate
    let populate =  anonymous && map_flags.contains(MmapFlags::POPULATE);

    // 添加 VMA 信息到 process_mmap_manager 和 aspace，等访问页面时触发 page fault 后建立页表映射
    let curr = current();
    let manager = curr.task_ext().process_data().process_mmap_mnager();
    manager.add_area(start_addr, length, fd, offset, shared)?;
    aspace.map_alloc(start_addr, aligned_length, permission_flags.into(), populate)?;     
    
    // 私有文件映射：
    // - 仅从文件读取数据到内存，修改不会同步到文件，即之前版本的 starry-next mmap 实现。
    // - 这里默认 populate，不会触发 page fault
    if !anonymous && !shared {
        let file = File::from_fd(fd)?;
        let file = file.inner();
        let file_size = file.get_attr()?.size() as usize;
        if offset as usize >= file_size {
            return Err(LinuxError::EINVAL);
        }
        let offset = offset as usize;
        let length = core::cmp::min(length, file_size - offset);
        let mut buf = vec![0u8; length];
        file.read_at(offset as u64, &mut buf)?;
        aspace.write(start_addr, &buf)?;
    }       
    
    return Ok(start_addr.as_usize() as _);
}

pub fn sys_munmap(addr: usize, length: usize) -> LinuxResult<isize> {
    // 同步文件
    sys_msync(addr, length, 0)?;

    // 从 process_mmap_manager 中移除 VMA
    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mmap_manager = process_data.process_mmap_mnager();
    let area = mmap_manager.query(VirtAddr::from(addr));
    if area.is_none() {
        error!("Invalid munmap area!");
        return Err(LinuxError::EINVAL);
    }
    mmap_manager.remove_area(VirtAddr::from(addr))?;
    
    // 对于文件mmap，在缓存页中取消反向映射，但是这一步没有修改页表
    let area = area.unwrap();
    if area.fd != -1 {
        assert!(addr % PAGE_SIZE_4K == 0);
        let cache_manager = page_cache_manager();
        let cache = cache_manager.fd_cache(area.fd);
        let start_page_id = (addr - area.start.as_usize()) / PAGE_SIZE_4K;
        let end_page_id = (addr - area.start.as_usize() + length) / PAGE_SIZE_4K;
        for page_id in start_page_id..end_page_id {
            cache.unmap_virt_page(page_id, VirtAddr::from(addr + page_id * PAGE_SIZE_4K));
        }
    }
    
    // 在 aspace 中移除 VMA 并取消页表映射
    let mut aspace = process_data.aspace.lock();
    let length = memory_addr::align_up_4k(length);
    // error!("before unmap \n{:#?}", aspace);
    aspace.unmap(VirtAddr::from(addr), length).unwrap();
    error!("after unmap \n{:#?}", aspace);
    axhal::arch::flush_tlb(None);
    Ok(0)
}

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

pub fn sys_msync(addr: usize, length: usize, _flags: isize) -> LinuxResult<isize> {
    // TODO: implement flags
    if addr % PAGE_SIZE_4K != 0 {
        error!("Msync addr must aligned to 4K!");
        return Err(LinuxError::EINVAL);
    }
    
    let curr = current();
    let mmap_manager = curr.task_ext().process_data().process_mmap_mnager();
    let area = mmap_manager.query(VirtAddr::from(addr));
    if area.is_none() {
        error!("Invalid msync area");
        return Err(LinuxError::EINVAL);
    }
    let area = area.unwrap();
    if area.fd == -1 {
        return Ok(0);
    }

    let cache_manager = page_cache_manager();
    let cache = cache_manager.fd_cache(area.fd);
    let start_page_id = (addr - area.start.as_usize()) / PAGE_SIZE_4K;
    let end_page_id = (addr - area.start.as_usize() + length) / PAGE_SIZE_4K;
    for page_id in start_page_id..end_page_id {
        cache.flush_page(page_id)?;
    }
    Ok(0)
}

/// 在 page fatul 时，判断是否由 mmap 文件映射引起，并建立建立虚拟页 => 文件缓存页的映射。
pub fn lazy_map_file(vaddr: VirtAddr, access_flags: MappingFlags) -> bool {
    let curr = current();
    let mmap_manager = curr.task_ext().process_data().process_mmap_mnager();
    let area = mmap_manager.query(vaddr);
    
    // 说明 page fatul 并非由 mmap lazy-alloc 引起。
    // 开销仅有 mmap_manager 的一次 query。
    if area.is_none() {
        return false;
    }

    // 匿名映射，应该会交给 AddrSpace::handle_page_fault 处理
    let area= area.unwrap();
    if area.fd == -1 {
        return false;
    }
    
    let offset = vaddr.as_usize() - area.start.as_usize();
    let page_id = offset / PAGE_SIZE_4K;
    let aligned_vaddr = VirtAddr::from(memory_addr::align_down_4k(vaddr.as_usize()));
    
    let page_cache_manager = page_cache_manager();
    let cache = page_cache_manager.fd_cache(area.fd);
    // 这里面直接修改了页表，并在 Page 中加入 virt_page 信息。
    return cache.map_virt_page(page_id, aligned_vaddr, access_flags);
}