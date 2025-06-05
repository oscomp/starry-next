use axhal::{
    mem::VirtAddr,
    paging::MappingFlags,
    trap::{register_trap_handler, PAGE_FAULT},
};
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::SIGSEGV;
use starry_api::{do_exit, file::page_cache_manager};
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

    if is_user {
        let cache_manager = page_cache_manager();
        if let Some(paddr) = cache_manager.try_mmap_lazy_init(vaddr) {
            let curr = current();
            let mut aspace = curr.task_ext().process_data().aspace.lock();
            aspace.force_map_page(vaddr, paddr, access_flags);
            return true;
        }
    }

    let curr = current();
    let mut aspace = curr.task_ext().process_data().aspace.lock();
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
