use core::ffi::{c_char, c_int};

use axerrno::{LinuxError, LinuxResult};
use axfs::fops::OpenOptions;
use macro_rules_attribute::apply;
use starry_core::{
    ctypes::{AT_EMPTY_PATH, stat},
    fd::{File, FileLike, get_file_like},
    path::handle_file_path,
};

use crate::{
    ptr::{UserConstPtr, UserPtr, nullable},
    syscall_instrument,
};

fn stat_at_path(path: &str) -> LinuxResult<stat> {
    let opts = OpenOptions::new().set_read(true);
    let file = axfs::fops::File::open(path, &opts)?;
    File::new(file, path.into()).stat()
}

/// Get the file metadata by `path` and write into `statbuf`.
///
/// Return 0 if success.
#[apply(syscall_instrument)]
pub fn sys_stat(path: UserConstPtr<c_char>, statbuf: UserPtr<stat>) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    debug!("sys_stat <= path: {}", path);

    *statbuf.get_as_mut()? = stat_at_path(path)?;

    Ok(0)
}

/// Get file metadata by `fd` and write into `statbuf`.
///
/// Return 0 if success.
#[apply(syscall_instrument)]
pub fn sys_fstat(fd: i32, statbuf: UserPtr<stat>) -> LinuxResult<isize> {
    debug!("sys_fstat <= fd: {}", fd);
    *statbuf.get_as_mut()? = get_file_like(fd)?.stat()?;
    Ok(0)
}

/// Get the metadata of the symbolic link and write into `buf`.
///
/// Return 0 if success.
#[apply(syscall_instrument)]
pub fn sys_lstat(path: UserConstPtr<c_char>, statbuf: UserPtr<stat>) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    debug!("sys_lstat <= path: {}", path);

    // TODO: symlink
    *statbuf.get_as_mut()? = stat::default();

    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_fstatat(
    dirfd: c_int,
    path: UserConstPtr<c_char>,
    statbuf: UserPtr<stat>,
    flags: u32,
) -> LinuxResult<isize> {
    let path = nullable!(path.get_as_str())?;
    debug!(
        "sys_fstatat <= dirfd: {}, path: {:?}, flags: {}",
        dirfd, path, flags
    );

    if path.is_none_or(|s| s.is_empty()) && (flags & AT_EMPTY_PATH) == 0 {
        return Err(LinuxError::ENOENT);
    }

    let path = handle_file_path(dirfd, path.unwrap_or_default())?;
    *statbuf.get_as_mut()? = stat_at_path(path.as_str())?;

    Ok(0)
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct FsStatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
}

/// statx - get file status (extended)
/// Standard C library (libc, -lc)
/// <https://man7.org/linux/man-pages/man2/statx.2.html>
#[repr(C)]
#[derive(Debug, Default)]
pub struct StatX {
    /// Bitmask of what information to get.
    pub stx_mask: u32,
    /// Block size for filesystem I/O.
    pub stx_blksize: u32,
    /// File attributes.
    pub stx_attributes: u64,
    /// Number of hard links.
    pub stx_nlink: u32,
    /// User ID of owner.
    pub stx_uid: u32,
    /// Group ID of owner.
    pub stx_gid: u32,
    /// File mode (permissions).
    pub stx_mode: u16,
    /// Inode number.
    pub stx_ino: u64,
    /// Total size, in bytes.
    pub stx_size: u64,
    /// Number of 512B blocks allocated.
    pub stx_blocks: u64,
    /// Mask to show what's supported in stx_attributes.
    pub stx_attributes_mask: u64,
    /// Last access timestamp.
    pub stx_atime: FsStatxTimestamp,
    /// Birth (creation) timestamp.
    pub stx_btime: FsStatxTimestamp,
    /// Last status change timestamp.
    pub stx_ctime: FsStatxTimestamp,
    /// Last modification timestamp.
    pub stx_mtime: FsStatxTimestamp,
    /// Major device ID (if special file).
    pub stx_rdev_major: u32,
    /// Minor device ID (if special file).
    pub stx_rdev_minor: u32,
    /// Major device ID of file system.
    pub stx_dev_major: u32,
    /// Minor device ID of file system.
    pub stx_dev_minor: u32,
    /// Mount ID.
    pub stx_mnt_id: u64,
    /// Memory alignment for direct I/O.
    pub stx_dio_mem_align: u32,
    /// Offset alignment for direct I/O.
    pub stx_dio_offset_align: u32,
}

#[apply(syscall_instrument)]
pub fn sys_statx(
    dirfd: c_int,
    path: UserConstPtr<c_char>,
    flags: u32,
    _mask: u32,
    statxbuf: UserPtr<StatX>,
) -> LinuxResult<isize> {
    // `statx()` uses pathname, dirfd, and flags to identify the target
    // file in one of the following ways:

    // An absolute pathname(situation 1)
    //        If pathname begins with a slash, then it is an absolute
    //        pathname that identifies the target file.  In this case,
    //        dirfd is ignored.

    // A relative pathname(situation 2)
    //        If pathname is a string that begins with a character other
    //        than a slash and dirfd is AT_FDCWD, then pathname is a
    //        relative pathname that is interpreted relative to the
    //        process's current working directory.

    // A directory-relative pathname(situation 3)
    //        If pathname is a string that begins with a character other
    //        than a slash and dirfd is a file descriptor that refers to
    //        a directory, then pathname is a relative pathname that is
    //        interpreted relative to the directory referred to by dirfd.
    //        (See openat(2) for an explanation of why this is useful.)

    // By file descriptor(situation 4)
    //        If pathname is an empty string (or NULL since Linux 6.11)
    //        and the AT_EMPTY_PATH flag is specified in flags (see
    //        below), then the target file is the one referred to by the
    //        file descriptor dirfd.

    let path = nullable!(path.get_as_str())?;
    debug!(
        "sys_statx <= dirfd: {}, path: {:?}, flags: {}",
        dirfd, path, flags
    );

    if path.is_none_or(|s| s.is_empty()) && (flags & AT_EMPTY_PATH) == 0 {
        return Err(LinuxError::ENOENT);
    }

    let path = handle_file_path(dirfd, path.unwrap_or_default())?;
    let stat = stat_at_path(path.as_str())?;

    let statx = statxbuf.get_as_mut()?;
    statx.stx_blksize = stat.st_blksize as u32;
    statx.stx_attributes = stat.st_mode as u64;
    statx.stx_nlink = stat.st_nlink;
    statx.stx_uid = stat.st_uid;
    statx.stx_gid = stat.st_gid;
    statx.stx_mode = stat.st_mode as u16;
    statx.stx_ino = stat.st_ino;
    statx.stx_size = stat.st_size as u64;
    statx.stx_blocks = stat.st_blocks as u64;
    statx.stx_attributes_mask = 0x7FF;
    statx.stx_atime.tv_sec = stat.st_atime.tv_sec;
    statx.stx_atime.tv_nsec = stat.st_atime.tv_nsec as u32;
    statx.stx_ctime.tv_sec = stat.st_ctime.tv_sec;
    statx.stx_ctime.tv_nsec = stat.st_ctime.tv_nsec as u32;
    statx.stx_mtime.tv_sec = stat.st_mtime.tv_sec;
    statx.stx_mtime.tv_nsec = stat.st_mtime.tv_nsec as u32;

    Ok(0)
}
