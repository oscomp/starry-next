use axerrno::{LinuxError, LinuxResult};
use axhal::time::{monotonic_time, monotonic_time_nanos, nanos_to_ticks, wall_time};
use linux_raw_sys::general::{
    __kernel_clockid_t, CLOCK_MONOTONIC, CLOCK_REALTIME, timespec, timeval,
};
use macro_rules_attribute::apply;
use starry_core::task::time_stat_output;

use crate::{
    ptr::UserPtr,
    syscall_instrument,
    time::{timevalue_to_timespec, timevalue_to_timeval},
};

#[apply(syscall_instrument)]
pub fn sys_clock_gettime(
    clock_id: __kernel_clockid_t,
    ts: UserPtr<timespec>,
) -> LinuxResult<isize> {
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
    *ts.get_as_mut()? = timevalue_to_timespec(now);
    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_get_time_of_day(ts: UserPtr<timeval>) -> LinuxResult<isize> {
    *ts.get_as_mut()? = timevalue_to_timeval(monotonic_time());
    Ok(0)
}

#[repr(C)]
pub struct Tms {
    /// 进程用户态执行时间，单位为us
    tms_utime: usize,
    /// 进程内核态执行时间，单位为us
    tms_stime: usize,
    /// 子进程用户态执行时间和，单位为us
    tms_cutime: usize,
    /// 子进程内核态执行时间和，单位为us
    tms_cstime: usize,
}

#[apply(syscall_instrument)]
pub fn sys_times(tms: UserPtr<Tms>) -> LinuxResult<isize> {
    let (_, utime_us, _, stime_us) = time_stat_output();
    *tms.get_as_mut()? = Tms {
        tms_utime: utime_us,
        tms_stime: stime_us,
        tms_cutime: utime_us,
        tms_cstime: stime_us,
    };
    Ok(nanos_to_ticks(monotonic_time_nanos()) as _)
}
