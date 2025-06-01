use core::{any::Any, ffi::c_int};

use alloc::{string::String, sync::{Arc, Weak}};
use axerrno::{LinuxError, LinuxResult};
use axfs::fops::DirEntry;
use axio::PollState;
use axsync::{Mutex, MutexGuard};
use linux_raw_sys::general::S_IFDIR;
use axio::SeekFrom;


use super::{get_file_like, page_cache::{PageCache, page_cache_manager}, FileLike, Kstat};

/// File wrapper for `axfs::fops::File`.
pub struct File {
    inner: Arc<Mutex<axfs::fops::File>>,
    path: String,
    is_direct: bool,
    cache: Weak<PageCache>,
}

impl File {
    pub fn new(inner: Arc<Mutex<axfs::fops::File>>, path: String, cache: Weak<PageCache>) -> Self {
        let direct = {
            let file = inner.clone();
            let file = file.lock();
            file.is_direct()
        };
        Self {
            inner: inner,
            path: path.clone(),
            is_direct: direct,
            cache,
        }
    }

    pub fn set_cache(&mut self, cache: Weak<PageCache>) {
        self.cache = cache;
    }

    /// Get the path of the file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get the inner node of the file.
    pub fn inner(&self) -> MutexGuard<axfs::fops::File> {
        self.inner.lock()
    }

    // 主要问题在于 fstat，在没有 open 的情况下仍然会以 direct 创建 struct File，此时并没有 cache
    fn try_get_cache(&self) -> Option<Arc<PageCache>> {
        if let Some(cache) = self.cache.upgrade() {
            return Some(cache);
        }
        let manager = page_cache_manager();
        if let Some(weak_cache) = manager.get_page_cache(&self.path) {
            if let Some(cache) = weak_cache.upgrade() {
                return Some(cache);
            }
        }
        return None;
    }


    fn get_cache(&self) -> Arc<PageCache> {
        self.cache.upgrade().unwrap()
    }

    pub fn seek(&self, pos: SeekFrom) -> LinuxResult<isize> {
        if !self.is_direct {
            let cache = self.get_cache();
            return cache.seek(pos);
        }
        self.inner().seek(pos)?;
        Ok(0)
    }

    pub fn fsync(&self) -> LinuxResult<isize> {
        if !self.is_direct {
            let cache = self.get_cache();
            cache.sync(false)?;
        }
        return Ok(0);
    }
}

impl FileLike for File {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        if !self.is_direct {
            let cache = self.get_cache();
            return cache.read(buf);
        }
        return Ok(self.inner().read(buf)?);
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        if !self.is_direct {
            let cache = self.get_cache();
            return cache.write(buf);
        }
        return Ok(self.inner().write(buf)?);
    }

    fn stat(&self) -> LinuxResult<Kstat> {
        // 先尝试从缓存中获取文件属性，如果没有缓存则直接获取文件属性
        if !self.is_direct {
            if let Some(cache) = self.try_get_cache() {
                return cache.stat();
            }
        }

        let metadata = self.inner().get_attr()?;
        let ty = metadata.file_type() as u8;
        let perm = metadata.perm().bits() as u32;

        return Ok(Kstat {
            mode: ((ty as u32) << 12) | perm,
            size: metadata.size(),
            blocks: metadata.blocks(),
            blksize: 512,
            ..Default::default()
        });
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
    last_dirent: Mutex<Option<DirEntry>>,
}

impl Directory {
    pub fn new(inner: axfs::fops::Directory, path: String) -> Self {
        Self {
            inner: Mutex::new(inner),
            path,
            last_dirent: Mutex::new(None),
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

    /// Get the last directory entry.
    pub fn last_dirent(&self) -> MutexGuard<Option<DirEntry>> {
        self.last_dirent.lock()
    }
}

impl FileLike for Directory {
    fn read(&self, _buf: &mut [u8]) -> LinuxResult<usize> {
        Err(LinuxError::EBADF)
    }

    fn write(&self, _buf: &[u8]) -> LinuxResult<usize> {
        Err(LinuxError::EBADF)
    }

    fn stat(&self) -> LinuxResult<Kstat> {
        Ok(Kstat {
            mode: S_IFDIR | 0o755u32, // rwxr-xr-x
            ..Default::default()
        })
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
