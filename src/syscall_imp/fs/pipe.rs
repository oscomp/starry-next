use core::ffi::c_int;

use arceos_posix_api as api;
use axerrno::LinuxResult;
use axptr::UserPtr;
use axtask::{current, TaskExtRef};

pub fn sys_pipe2(mut fds: UserPtr<c_int>) -> LinuxResult<isize> {
    let fds = fds.get_as_slice(current().task_ext(), 2)?;
    Ok(api::sys_pipe(fds) as _)
}
