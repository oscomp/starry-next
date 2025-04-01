use core::ffi::c_char;

use arceos_posix_api::{self as api, ctypes::mode_t};
use axerrno::LinuxResult;
use axptr::{UserConstPtr, UserPtr};
use axtask::{TaskExtRef, current};

pub(crate) fn sys_read(fd: i32, mut buf: UserPtr<u8>, count: usize) -> LinuxResult<isize> {
    let buf = buf.get_as_slice(current().task_ext(), count)?;
    Ok(api::sys_read(fd, buf.as_mut_ptr().cast(), count))
}

pub(crate) fn sys_write(fd: i32, buf: UserConstPtr<u8>, count: usize) -> LinuxResult<isize> {
    let buf = buf.get_as_slice(current().task_ext(), count)?;
    Ok(api::sys_write(fd, buf.as_ptr().cast(), count))
}

pub(crate) fn sys_writev(
    fd: i32,
    iov: UserConstPtr<api::ctypes::iovec>,
    iocnt: i32,
) -> LinuxResult<isize> {
    let iov = iov.get_as_slice(current().task_ext(), iocnt as _)?;
    unsafe { Ok(api::sys_writev(fd, iov.as_ptr().cast(), iocnt)) }
}

pub(crate) fn sys_openat(
    dirfd: i32,
    path: UserConstPtr<c_char>,
    flags: i32,
    modes: mode_t,
) -> LinuxResult<isize> {
    let path = path.get_as_str(current().task_ext())?;
    Ok(api::sys_openat(dirfd, path.as_ptr(), flags, modes) as _)
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn sys_open(
    path: UserConstPtr<c_char>,
    flags: i32,
    modes: mode_t,
) -> LinuxResult<isize> {
    use arceos_posix_api::AT_FDCWD;
    sys_openat(AT_FDCWD as _, path, flags, modes)
}
