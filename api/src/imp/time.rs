use axerrno::{LinuxError, LinuxResult};
use axhal::time::{monotonic_time, monotonic_time_nanos, nanos_to_ticks, wall_time};
use macro_rules_attribute::apply;
use starry_core::{
    ctypes::{CLOCK_MONOTONIC, CLOCK_REALTIME, clockid_t, timespec, timeval},
    task::time_stat_output,
};

use crate::{ptr::UserPtr, syscall_instrument};

#[apply(syscall_instrument)]
pub fn sys_clock_gettime(clock_id: clockid_t, ts: UserPtr<timespec>) -> LinuxResult<isize> {
    let now = match clock_id as u32 {
        CLOCK_REALTIME => wall_time(),
        CLOCK_MONOTONIC => monotonic_time(),
        _ => {
            warn!(
                "Called sys_clock_gettime for unsupported clock {}",
                clock_id
            );
            return Err(LinuxError::EINVAL);
        }
    };
    *ts.get_as_mut()? = now.into();
    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_get_time_of_day(ts: UserPtr<timeval>) -> LinuxResult<isize> {
    *ts.get_as_mut()? = monotonic_time().into();
    Ok(0)
}

#[repr(C)]
pub struct Tms {
    /// 进程用户态执行时间，单位为us
    pub utime: usize,
    /// 进程内核态执行时间，单位为us
    pub stime: usize,
    /// 子进程用户态执行时间和，单位为us
    pub cutime: usize,
    /// 子进程内核态执行时间和，单位为us
    pub cstime: usize,
}

#[apply(syscall_instrument)]
pub fn sys_times(tms: UserPtr<Tms>) -> LinuxResult<isize> {
    let (_, utime_us, _, stime_us) = time_stat_output();
    *tms.get_as_mut()? = Tms {
        utime: utime_us,
        stime: stime_us,
        cutime: utime_us,
        cstime: stime_us,
    };
    Ok(nanos_to_ticks(monotonic_time_nanos()) as _)
}
