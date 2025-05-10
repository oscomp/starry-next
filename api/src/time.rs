use axhal::time::TimeValue;
use linux_raw_sys::general::{timespec, timeval};

pub fn timevalue_to_timespec(tv: TimeValue) -> timespec {
    timespec {
        tv_sec: tv.as_secs() as _,
        tv_nsec: tv.subsec_nanos() as _,
    }
}

pub fn timespec_to_timevalue(ts: timespec) -> TimeValue {
    TimeValue::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub fn timevalue_to_timeval(tv: TimeValue) -> timeval {
    timeval {
        tv_sec: tv.as_secs() as _,
        tv_usec: tv.subsec_micros() as _,
    }
}

pub fn timeval_to_timevalue(tv: timeval) -> TimeValue {
    TimeValue::new(tv.tv_sec as u64, tv.tv_usec as u32 * 1_000)
}
