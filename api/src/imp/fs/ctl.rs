use core::ffi::{c_char, c_int, c_void};

use alloc::ffi::CString;
use axerrno::{LinuxError, LinuxResult};
use linux_raw_sys::general::{__kernel_ino_t, __kernel_off_t, AT_REMOVEDIR};
use macro_rules_attribute::apply;

use crate::{
    file::{Directory, FileLike},
    path::{HARDLINK_MANAGER, handle_file_path},
    ptr::{UserConstPtr, UserPtr, nullable},
    syscall_instrument,
};

/// The ioctl() system call manipulates the underlying device parameters
/// of special files.
///
/// # Arguments
/// * `fd` - The file descriptor
/// * `op` - The request code. It is of type unsigned long in glibc and BSD,
///   and of type int in musl and other UNIX systems.
/// * `argp` - The argument to the request. It is a pointer to a memory location
#[apply(syscall_instrument)]
pub fn sys_ioctl(_fd: i32, _op: usize, _argp: UserPtr<c_void>) -> LinuxResult<isize> {
    warn!("Unimplemented syscall: SYS_IOCTL");
    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_chdir(path: UserConstPtr<c_char>) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    debug!("sys_chdir <= {:?}", path);

    axfs::api::set_current_dir(path)?;
    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_mkdirat(dirfd: i32, path: UserConstPtr<c_char>, mode: u32) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    debug!(
        "sys_mkdirat <= dirfd: {}, path: {}, mode: {}",
        dirfd, path, mode
    );

    if mode != 0 {
        warn!("directory mode not supported.");
    }

    let path = handle_file_path(dirfd, path)?;
    axfs::api::create_dir(path.as_str())?;

    Ok(0)
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct DirEnt {
    d_ino: __kernel_ino_t,
    d_off: __kernel_off_t,
    d_reclen: u16,
    d_type: u8,
    d_name: [u8; 0],
}

#[allow(dead_code)]
#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum FileType {
    Unknown = 0,
    Fifo = 1,
    Chr = 2,
    Dir = 4,
    Blk = 6,
    Reg = 8,
    Lnk = 10,
    Socket = 12,
    Wht = 14,
}

impl From<axfs::api::FileType> for FileType {
    fn from(ft: axfs::api::FileType) -> Self {
        match ft {
            ft if ft.is_dir() => FileType::Dir,
            ft if ft.is_file() => FileType::Reg,
            _ => FileType::Unknown,
        }
    }
}

impl DirEnt {
    const FIXED_SIZE: usize = core::mem::size_of::<u64>()
        + core::mem::size_of::<i64>()
        + core::mem::size_of::<u16>()
        + core::mem::size_of::<u8>();

    fn new(ino: u64, off: i64, reclen: usize, file_type: FileType) -> Self {
        Self {
            d_ino: ino,
            d_off: off,
            d_reclen: reclen as u16,
            d_type: file_type as u8,
            d_name: [],
        }
    }

    unsafe fn write_name(&mut self, name: &[u8]) {
        unsafe {
            core::ptr::copy_nonoverlapping(name.as_ptr(), self.d_name.as_mut_ptr(), name.len());
        }
    }
}

// Directory buffer for getdents64 syscall
struct DirBuffer<'a> {
    buf: &'a mut [u8],
    offset: usize,
}

impl<'a> DirBuffer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    fn remaining_space(&self) -> usize {
        self.buf.len().saturating_sub(self.offset)
    }

    fn can_fit_entry(&self, entry_size: usize) -> bool {
        self.remaining_space() >= entry_size
    }

    fn write_entry(&mut self, dirent: DirEnt, name: &[u8]) -> Result<(), ()> {
        if !self.can_fit_entry(dirent.d_reclen as usize) {
            return Err(());
        }
        unsafe {
            let entry_ptr = self.buf.as_mut_ptr().add(self.offset) as *mut DirEnt;
            entry_ptr.write(dirent);
            (*entry_ptr).write_name(name);
        }

        self.offset += dirent.d_reclen as usize;
        Ok(())
    }

    fn read_entry(&self, offset: usize) -> Option<&DirEnt> {
        if offset + DirEnt::FIXED_SIZE > self.buf.len() {
            return None;
        }
        let entry_ptr = unsafe { self.buf.as_ptr().add(offset) as *const DirEnt };
        Some(unsafe { &*entry_ptr })
    }
}

#[apply(syscall_instrument)]
pub fn sys_getdents64(fd: i32, buf: UserPtr<u8>, len: usize) -> LinuxResult<isize> {
    let buf = buf.get_as_mut_slice(len)?;
    debug!(
        "sys_getdents64 <= fd: {}, buf: {:p}, len: {}",
        fd,
        buf.as_ptr(),
        buf.len()
    );

    if len < DirEnt::FIXED_SIZE {
        warn!("Buffer size too small: {len}");
        return Err(LinuxError::EINVAL);
    }

    let mut buffer = DirBuffer::new(buf);

    let (initial_offset, count) = {
        let mut buf_offset = 0;
        let mut count = 0;
        while let Some(dir_ent) = buffer.read_entry(buf_offset) {
            if dir_ent.d_reclen == 0 {
                break;
            }

            buf_offset += dir_ent.d_reclen as usize;
            assert_eq!(dir_ent.d_off, buf_offset as i64);
            count += 1;
        }
        (buf_offset as i64, count)
    };

    let dir = Directory::from_fd(fd)?;
    let path = dir.path();

    let mut total_size = initial_offset as usize;
    let mut current_offset = initial_offset;

    for entry in axfs::api::read_dir(path)?.flatten().skip(count) {
        let mut name = entry.file_name();
        name.push('\0');
        let name_bytes = name.as_bytes();

        let entry_size = DirEnt::FIXED_SIZE + name_bytes.len();
        current_offset += entry_size as i64;

        let dirent = DirEnt::new(
            1,
            current_offset,
            entry_size,
            FileType::from(entry.file_type()),
        );

        if buffer.write_entry(dirent, name_bytes).is_err() {
            break;
        }

        total_size += entry_size;
    }

    if total_size > 0 && buffer.can_fit_entry(DirEnt::FIXED_SIZE) {
        let terminal = DirEnt::new(1, current_offset, 0, FileType::Reg);
        let _ = buffer.write_entry(terminal, &[]);
    }

    Ok(total_size as isize)
}

/// create a link from new_path to old_path
/// old_path: old file path
/// new_path: new file path
/// flags: link flags
/// return value: return 0 when success, else return -1.
#[apply(syscall_instrument)]
pub fn sys_linkat(
    old_dirfd: c_int,
    old_path: UserConstPtr<c_char>,
    new_dirfd: c_int,
    new_path: UserConstPtr<c_char>,
    flags: i32,
) -> LinuxResult<isize> {
    let old_path = old_path.get_as_str()?;
    let new_path = new_path.get_as_str()?;
    debug!(
        "sys_linkat <= old_dirfd: {}, old_path: {}, new_dirfd: {}, new_path: {}, flags: {}",
        old_dirfd, old_path, new_dirfd, new_path, flags
    );

    if flags != 0 {
        warn!("Unsupported flags: {flags}");
    }

    // handle old path
    let old_path = handle_file_path(old_dirfd, old_path)?;
    // handle new path
    let new_path = handle_file_path(new_dirfd, new_path)?;

    HARDLINK_MANAGER.create_link(&new_path, &old_path)?;

    Ok(0)
}

/// remove link of specific file (can be used to delete file)
/// dir_fd: the directory of link to be removed
/// path: the name of link to be removed
/// flags: can be 0 or AT_REMOVEDIR
/// return 0 when success, else return -1
#[apply(syscall_instrument)]
pub fn sys_unlinkat(dirfd: c_int, path: UserConstPtr<c_char>, flags: u32) -> LinuxResult<isize> {
    let path = path.get_as_str()?;
    debug!(
        "sys_unlinkat <= dirfd: {}, path: {}, flags: {}",
        dirfd, path, flags
    );

    let path = handle_file_path(dirfd, path)?;

    if flags == AT_REMOVEDIR {
        axfs::api::remove_dir(path.as_str())?;
    } else {
        let metadata = axfs::api::metadata(path.as_str())?;
        if metadata.is_dir() {
            return Err(LinuxError::EISDIR);
        } else {
            debug!("unlink file: {:?}", path);
            HARDLINK_MANAGER
                .remove_link(&path)
                .ok_or(LinuxError::ENOENT)?;
        }
    }
    Ok(0)
}

#[apply(syscall_instrument)]
pub fn sys_getcwd(buf: UserPtr<u8>, size: usize) -> LinuxResult<isize> {
    let buf = nullable!(buf.get_as_mut_slice(size))?;

    let Some(buf) = buf else {
        return Ok(0);
    };

    let cwd = CString::new(axfs::api::current_dir()?).map_err(|_| LinuxError::EINVAL)?;
    let cwd = cwd.as_bytes_with_nul();

    if cwd.len() <= buf.len() {
        buf[..cwd.len()].copy_from_slice(cwd);
        Ok(buf.as_ptr() as _)
    } else {
        Err(LinuxError::ERANGE)
    }
}
