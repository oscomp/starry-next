use axhal::{
    mem::VirtAddr,
    paging::MappingFlags,
    trap::{register_trap_handler, PAGE_FAULT},
};
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::SIGSEGV;
use starry_api::{do_exit, file::mmap_area_manager};
use starry_core::mm::is_accessing_user_memory;

#[register_trap_handler(PAGE_FAULT)]
fn handle_page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    warn!(
        "Page fault at {:#x}, access_flags: {:#x?}",
        vaddr, access_flags
    );
    if !is_user && !is_accessing_user_memory() {
        return false;
    }

    let curr = current();
    let mut aspace = curr.task_ext().process_data().aspace.lock();

    // 检查是否为文件映射
    let pid = curr.task_ext().thread.process().pid();
    let mmap_manager = mmap_area_manager();
    if let Some(paddr) = mmap_manager.find_paddr(pid, &vaddr) {
        aspace.force_map_page(vaddr, paddr, access_flags);
        return true;
    }

    if aspace.handle_page_fault(vaddr, access_flags) {
        return true;
    }

    warn!(
        "{} ({:?}): segmentation fault at {:#x}, exit!",
        curr.id_name(),
        curr.task_ext().thread,
        vaddr
    );
    do_exit(SIGSEGV as _, true);
}
