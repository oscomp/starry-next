use alloc::{string::String, sync::Arc};
use axhal::arch::UspaceContext;
use axsync::Mutex;
use memory_addr::VirtAddr;

use crate::{mm::load_user_app, path::FilePath, task::spawn_user_task};

pub fn run_user_app(args: &[String], envs: &[String]) -> Option<i32> {
    let mut uspace = axmm::new_user_aspace(
        VirtAddr::from_usize(axconfig::plat::USER_SPACE_BASE),
        axconfig::plat::USER_SPACE_SIZE,
    )
    .expect("Failed to create user address space");

    let path = FilePath::new(&args[0]).expect("Invalid file path");
    axfs::api::set_current_dir(path.parent().unwrap()).expect("Failed to set current dir");

    let (entry_vaddr, ustack_top) = load_user_app(&mut uspace, args, envs)
        .unwrap_or_else(|e| panic!("Failed to load user app: {}", e));
    let user_task = spawn_user_task(
        Arc::new(Mutex::new(uspace)),
        UspaceContext::new(entry_vaddr.into(), ustack_top, 2333),
        axconfig::plat::USER_HEAP_BASE as _,
    );
    user_task.join()
}
