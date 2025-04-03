use core::ffi::c_void;

use axerrno::LinuxResult;

use crate::ptr::{PtrWrapper, UserConstPtr, UserPtr};

pub fn sys_getuid() -> LinuxResult<isize> {
    Ok(0)
}

#[repr(C)]
pub struct UtsName {
    /// sysname
    pub sysname: [u8; 65],
    /// nodename
    pub nodename: [u8; 65],
    /// release
    pub release: [u8; 65],
    /// version
    pub version: [u8; 65],
    /// machine
    pub machine: [u8; 65],
    /// domainname
    pub domainname: [u8; 65],
}

impl Default for UtsName {
    fn default() -> Self {
        Self {
            sysname: Self::from_str("Starry"),
            nodename: Self::from_str("Starry - machine[0]"),
            release: Self::from_str("10.0.0"),
            version: Self::from_str("10.0.0"),
            machine: Self::from_str("10.0.0"),
            domainname: Self::from_str("https://github.com/BattiestStone4/Starry-On-ArceOS"),
        }
    }
}

impl UtsName {
    fn from_str(info: &str) -> [u8; 65] {
        let mut data: [u8; 65] = [0; 65];
        data[..info.len()].copy_from_slice(info.as_bytes());
        data
    }
}

pub fn sys_uname(name: UserPtr<UtsName>) -> LinuxResult<isize> {
    unsafe { *name.get()? = UtsName::default() };
    Ok(0)
}

pub fn sys_prlimit64(
    _pid: i32,
    _resource: i32,
    _new_limit: UserConstPtr<c_void>,
    _old_limit: UserPtr<c_void>,
) -> LinuxResult<isize> {
    warn!("sys_prlimit64: not implemented");
    Ok(0)
}

pub fn sys_rseq(
    _rseq: UserPtr<c_void>,
    _rseq_len: u32,
    _flags: u32,
    _sig: UserPtr<c_void>,
) -> LinuxResult<isize> {
    warn!("sys_rseq: not implemented");
    Ok(0)
}

pub fn sys_getrandom(_buf: UserPtr<c_void>, _buflen: usize, _flags: u32) -> LinuxResult<isize> {
    warn!("sys_getrandom: not implemented");
    Ok(0)
}
