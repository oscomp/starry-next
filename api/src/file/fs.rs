use core::{
    any::Any,
    ffi::c_int,
    sync::atomic::{AtomicUsize, Ordering},
};

use alloc::{
    string::String,
    sync::{Arc, Weak},
};
use axerrno::{LinuxError, LinuxResult};
use axfs::fops::DirEntry;
use axio::PollState;
use axio::SeekFrom;
use axsync::{Mutex, MutexGuard};
use linux_raw_sys::general::S_IFDIR;

use super::{
    FileLike, Kstat, get_file_like,
    page_cache::{PageCache, page_cache_manager},
};

/// File wrapper for `axfs::fops::File`.
pub struct File {
    inner: Option<Arc<Mutex<axfs::fops::File>>>,
    path: String,
    is_direct: bool,
    cache: Weak<PageCache>,
    offset: AtomicUsize,
    size: AtomicUsize,
}

impl File {
    pub fn new(
        inner: Option<Arc<Mutex<axfs::fops::File>>>,
        path: String,
        is_direct: bool,
        cache: Weak<PageCache>,
    ) -> Self {
        debug!("Starry-api open file {}", path);
        let size = {
            if is_direct {
                let inner = inner.clone().unwrap();
                let inner = inner.lock();
                let metadata = inner.get_attr().unwrap();
                metadata.size() as usize
            } else {
                let cache = cache.upgrade().unwrap();
                cache.get_file_size()
            }
        };
        Self {
            inner,
            path,
            is_direct,
            cache,
            offset: AtomicUsize::new(0),
            size: AtomicUsize::new(size),
        }
    }

    pub fn get_cache(&self) -> Arc<PageCache> {
        self.cache.upgrade().unwrap()
    }

    pub fn get_size(&self) -> usize {
        self.size.load(Ordering::SeqCst)
    }

    /// Get the path of the file.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get the inner node of the file.
    fn inner(&self) -> MutexGuard<axfs::fops::File> {
        self.inner.as_ref().unwrap().lock()
    }

    pub fn seek(&self, pos: SeekFrom) -> LinuxResult<isize> {
        if self.is_direct {
            match self.inner().seek(pos) {
                Ok(pos) => return Ok(pos as isize),
                Err(e) => {
                    error!("Seek failed: {}", e);
                    return Err(LinuxError::EINVAL);
                }
            }
        }
        let size = self.size.load(Ordering::SeqCst) as u64;
        let offset = self.offset.load(Ordering::SeqCst) as u64;
        let new_offset = match pos {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(off) => offset.checked_add_signed(off),
            SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .unwrap();
        self.offset.store(new_offset as usize, Ordering::SeqCst);
        Ok(new_offset as isize)
    }

    pub fn fsync(&self) -> LinuxResult<usize> {
        if self.is_direct {
            return Ok(0);
        }
        let cache = self.get_cache();
        Ok(cache.sync()?)
    }

    pub fn read_at(&self, buf: &mut [u8], offset: usize) -> LinuxResult<usize> {
        if self.is_direct {
            return Ok(self.inner().read_at(offset as u64, buf)?);
        }

        let cache = self.get_cache();
        Ok(cache.read_at(offset, buf))
    }

    pub fn write_at(&self, buf: &[u8], offset: usize) -> LinuxResult<usize> {
        if self.is_direct {
            return Ok(self.inner().write_at(offset as u64, buf)?);
        }

        let cache = self.get_cache();
        Ok(cache.write_at(offset, buf))
    }

    pub fn truncate(&self, offset: usize) -> LinuxResult<usize> {
        if self.is_direct {
            self.inner().truncate(offset as u64)?;
            return Ok(offset);
        }
        let cache = self.get_cache();
        Ok(cache.truncate(offset) as usize)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        debug!("Starry-api drop file {}", self.path);
        if !self.is_direct {
            let cache_manager = page_cache_manager();
            cache_manager.close_page_cache(&self.path);
        }
    }
}

impl FileLike for File {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        if self.is_direct {
            return Ok(self.inner().read(buf)?);
        }

        let cache = self.get_cache();
        let offset = self.offset.load(Ordering::SeqCst);
        let len = cache.write_at(offset, buf);
        self.offset.fetch_add(len, Ordering::SeqCst);
        Ok(len)
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        if self.is_direct {
            return Ok(self.inner().write(buf)?);
        }

        let cache = self.get_cache();
        let offset = self.offset.load(Ordering::SeqCst);
        let len = cache.write_at(offset, buf);
        self.offset.fetch_add(len, Ordering::SeqCst);
        self.size.fetch_max(offset + len, Ordering::SeqCst);
        Ok(len)
    }

    fn stat(&self) -> LinuxResult<Kstat> {
        if !self.is_direct {
            let cache = self.get_cache();
            return cache.stat();
        }

        let metadata = self.inner().get_attr()?;
        let ty = metadata.file_type() as u8;
        let perm = metadata.perm().bits() as u32;
        let size = self.size.load(Ordering::SeqCst);
        Ok(Kstat {
            mode: ((ty as u32) << 12) | perm,
            size: size as u64,
            blocks: metadata.blocks(),
            blksize: 512,
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
