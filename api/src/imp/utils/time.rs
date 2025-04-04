use arceos_posix_api::{self as api, ctypes::timeval};
use axerrno::LinuxResult;
use axhal::time::{monotonic_time_nanos, nanos_to_ticks};
use axptr::UserPtr;
use axtask::{TaskExtRef, current};

use starry_core::{ctypes::Tms, task::time_stat_output};

pub fn sys_clock_gettime(
    clock_id: i32,
    mut tp: UserPtr<api::ctypes::timespec>,
) -> LinuxResult<isize> {
    unsafe { Ok(api::sys_clock_gettime(clock_id, tp.get(current().task_ext())?) as _) }
}

pub fn sys_get_time_of_day(mut ts: UserPtr<timeval>) -> LinuxResult<isize> {
    unsafe { Ok(api::sys_get_time_of_day(ts.get(current().task_ext())?) as _) }
}

pub fn sys_times(mut tms: UserPtr<Tms>) -> LinuxResult<isize> {
    let (_, utime_us, _, stime_us) = time_stat_output();
    *tms.get(current().task_ext())? = Tms {
        tms_utime: utime_us,
        tms_stime: stime_us,
        tms_cutime: utime_us,
        tms_cstime: stime_us,
    };
    Ok(nanos_to_ticks(monotonic_time_nanos()) as _)
}
