use axerrno::{LinuxError, LinuxResult};
use axhal::time::TimeValue;
use macro_rules_attribute::apply;
use starry_core::ctypes::timespec;

use crate::{
    ptr::{UserConstPtr, UserPtr, nullable},
    syscall_instrument,
};

pub fn sys_sched_yield() -> LinuxResult<isize> {
    axtask::yield_now();
    Ok(0)
}

/// Sleep some nanoseconds
///
/// TODO: should be woken by signals, and set errno
#[apply(syscall_instrument)]
pub fn sys_nanosleep(req: UserConstPtr<timespec>, rem: UserPtr<timespec>) -> LinuxResult<isize> {
    let req = req.get_as_ref()?;

    if req.tv_nsec < 0 || req.tv_nsec > 999_999_999 || req.tv_sec < 0 {
        return Err(LinuxError::EINVAL);
    }

    let dur = TimeValue::from(*req);
    debug!("sys_nanosleep <= {:?}", dur);

    let now = axhal::time::monotonic_time();

    axtask::sleep(dur);

    let after = axhal::time::monotonic_time();
    let actual = after - now;

    if let Some(diff) = dur.checked_sub(actual) {
        if let Some(rem) = nullable!(rem.get_as_mut())? {
            *rem = diff.into();
        }
        Err(LinuxError::EINTR)
    } else {
        Ok(0)
    }
}
