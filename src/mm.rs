use axhal::{
    mem::VirtAddr,
    paging::MappingFlags,
    trap::{PAGE_FAULT, register_trap_handler},
};
use axtask::{TaskExtRef, current};
use starry_api::do_exit;
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
    if !curr
        .task_ext()
        .process_data()
        .aspace
        .lock()
        .handle_page_fault(vaddr, access_flags)
    {
        warn!(
            "{} ({:?}): segmentation fault at {:#x}, exit!",
            curr.id_name(),
            curr.task_ext().thread,
            vaddr
        );
        // FIXME: change to SIGSEGV
        do_exit(11, true);
    }
    true
}
