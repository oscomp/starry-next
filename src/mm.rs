use axhal::{
    mem::VirtAddr,
    mem::PhysAddr,
    paging::MappingFlags,
    trap::{register_trap_handler, PAGE_FAULT},
};
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::SIGSEGV;
use starry_api::{do_exit, file::page_cache_manager, lazy_map_file};
use starry_core::mm::is_accessing_user_memory;


#[register_trap_handler(PAGE_FAULT)]
fn handle_page_fault(vaddr: VirtAddr, access_flags: MappingFlags, is_user: bool) -> bool {
    warn!(
        "Page fault at {:#x}, {}, access_flags: {:#x?}, is_user: {}",
        vaddr, current().id_name(), access_flags, is_user
    );
    if !is_user && !is_accessing_user_memory() {
        return false;
    }
    if is_user && lazy_map_file(vaddr, access_flags) {
        return true;
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
