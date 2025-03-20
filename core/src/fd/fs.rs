use core::{any::Any, ffi::c_int};

use alloc::{string::String, sync::Arc};
use axerrno::{LinuxError, LinuxResult};
use axio::PollState;
use axsync::{Mutex, MutexGuard};

use crate::ctypes::stat;

use super::{FileLike, get_file_like};

/// File wrapper for `axfs::fops::File`.
pub struct File {
    inner: Mutex<axfs::fops::File>,
    path: String,
}

impl File {
    pub fn new(inner: axfs::fops::File, path: String) -> Self {
        Self {
            inner: Mutex::new(inner),
            path,
        }
    }

    /// Get the path of the file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get the inner node of the file.
    pub fn inner(&self) -> MutexGuard<axfs::fops::File> {
        self.inner.lock()
    }
}

impl FileLike for File {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        Ok(self.inner().read(buf)?)
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        Ok(self.inner().write(buf)?)
    }

    fn stat(&self) -> LinuxResult<stat> {
        let metadata = self.inner().get_attr()?;
        let ty = metadata.file_type() as u8;
        let perm = metadata.perm().bits() as u32;
        let st_mode = ((ty as u32) << 12) | perm;
        Ok(stat {
            st_ino: 1,
            st_nlink: 1,
            st_mode,
            st_uid: 1000,
            st_gid: 1000,
            st_size: metadata.size() as _,
            st_blocks: metadata.blocks() as _,
            st_blksize: 512,
            ..Default::default()
        })
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        Ok(PollState {
            readable: true,
            writable: true,
        })
    }

    fn set_nonblocking(&self, _nonblocking: bool) -> LinuxResult {
        Ok(())
    }
}

/// Directory wrapper for `axfs::fops::Directory`.
pub struct Directory {
    inner: Mutex<axfs::fops::Directory>,
    path: String,
}

impl Directory {
    pub fn new(inner: axfs::fops::Directory, path: String) -> Self {
        Self {
            inner: Mutex::new(inner),
            path,
        }
    }

    /// Get the path of the directory.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get the inner node of the directory.
    pub fn inner(&self) -> MutexGuard<axfs::fops::Directory> {
        self.inner.lock()
    }
}

impl FileLike for Directory {
    fn read(&self, _buf: &mut [u8]) -> LinuxResult<usize> {
        Err(LinuxError::EBADF)
    }

    fn write(&self, _buf: &[u8]) -> LinuxResult<usize> {
        Err(LinuxError::EBADF)
    }

    fn stat(&self) -> LinuxResult<stat> {
        Err(LinuxError::EBADF)
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn poll(&self) -> LinuxResult<PollState> {
        Ok(PollState {
            readable: true,
            writable: false,
        })
    }

    fn set_nonblocking(&self, _nonblocking: bool) -> LinuxResult {
        Ok(())
    }

    fn from_fd(fd: c_int) -> LinuxResult<Arc<Self>> {
        get_file_like(fd)?
            .into_any()
            .downcast::<Self>()
            .map_err(|_| LinuxError::ENOTDIR)
    }
}
