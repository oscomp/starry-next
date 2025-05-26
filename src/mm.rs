use axhal::{
    mem::{PhysAddr, VirtAddr},
    paging::MappingFlags,
    trap::{register_trap_handler, PAGE_FAULT},
};
use axtask::{TaskExtRef, current};
use linux_raw_sys::general::SIGSEGV;
use starry_api::do_exit;
use starry_api::file::{File, FileLike};
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

    if aspace.handle_page_fault(vaddr, access_flags) {
        return true;
    }
    
    if let Some((fd, start)) = aspace.get_file_metadata(vaddr) {
        let file = File::from_fd(fd).unwrap();
        let tmp_addr = file.cache().get_vaddr(vaddr - start);
        let paddr = axhal::mem::virt_to_phys(tmp_addr);
        warn!("Mmap page fault: fd = {}, start = {:#x}, vaddr = {:#x}, paddr = {:#x}", fd, start, vaddr, paddr);
        aspace.force_map_page(vaddr, paddr, access_flags);
        return true;
    }
    
    warn!(
        "{} ({:?}): segmentation fault at {:#x}, exit!",
        curr.id_name(),
        curr.task_ext().thread,
        vaddr
    );
    do_exit(SIGSEGV as _, true);
    false
}
