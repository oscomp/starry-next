use arceos_posix_api as api;
use axerrno::LinuxResult;
use axptr::{UserConstPtr, UserPtr};
use axtask::{TaskExtRef, current};

pub fn sys_sched_yield() -> LinuxResult<isize> {
    Ok(api::sys_sched_yield() as _)
}

pub fn sys_nanosleep(
    req: UserConstPtr<api::ctypes::timespec>,
    mut rem: UserPtr<api::ctypes::timespec>,
) -> LinuxResult<isize> {
    let req = req.get(current().task_ext())?;
    let rem = rem.get(current().task_ext())?;
    unsafe { Ok(api::sys_nanosleep(req, rem) as _) }
}
