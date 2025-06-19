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
use axio::{PollState, SeekFrom};
use axsync::{Mutex, MutexGuard};
use linux_raw_sys::general::S_IFDIR;

use super::{
    FileLike, Kstat, get_file_like,
    page_cache::{PageCache, page_cache_manager},
};

/// File wrapper for `axfs::fops::File` with two variants.
pub enum File {
    Direct {
        path: String,
        inner: Arc<Mutex<axfs::fops::File>>,
    },
    Cached {
        path: String,
        size: AtomicUsize,
        offset: AtomicUsize,
        cache: Weak<PageCache>,
    },
}

impl File {
    pub fn new_file_direct(path: String, inner: Arc<Mutex<axfs::fops::File>>) -> Self {
        let size = {
            let inner = inner.clone();
            let inner = inner.lock();
            let metadata = inner.get_attr().unwrap();
            metadata.size() as usize
        };
        File::Direct {
            path,
            inner,
        }
    }

    pub fn new_file_cached(path: String, cache: Weak<PageCache>) -> Self {
        let size = {
            let cache = cache.upgrade().unwrap();
            cache.file_size()
        };
        File::Cached {
            path,
            size: AtomicUsize::new(size),
            offset: AtomicUsize::new(0),
            cache,
        }
    }

    pub fn size(&self) -> usize {
        match self {
            File::Direct { inner, .. } => {
                let inner = inner.lock();
                inner.get_attr().unwrap().size() as usize
            },
            File::Cached { size, .. } => {
                size.load(Ordering::Relaxed)
            }
        }
    }

    pub fn offset(&self) -> usize {
        match self {
            File::Direct { .. } => {
                panic!("Please use seek for File::Direc");
            },
            File::Cached { offset, .. } => {
                offset.load(Ordering::Relaxed)
            }
        }
    }

    /// Get the path of the file.
    pub fn path(&self) -> &String {
        match self {
            File::Direct { path, .. } | File::Cached { path, .. } => path,
        }
    }

    /// Get the cache of the file. Only available in File::Cached.
    pub fn cache(&self) -> Arc<PageCache> {
        match self {
            File::Direct { .. } => {
                panic!("File::Direct doesn't have cache.");
            }
            File::Cached { cache, .. } => cache.upgrade().unwrap(),
        }
    }

    /// Get the inner node of the file. Only available in File::Direct.
    fn inner(&self) -> MutexGuard<axfs::fops::File> {
        match self {
            File::Direct { inner, .. } => inner.lock(),
            File::Cached {  .. } => {
                panic!("File::Cached doesn't have inner.");
            }
        }
    }

    pub fn seek(&self, pos: SeekFrom) -> LinuxResult<isize> {
        match self {
            File::Direct { .. } => {
                let mut inner = self.inner();
                match inner.seek(pos) {
                    Ok(pos) => Ok(pos as isize),
                    Err(e) => {
                        error!("Seek failed: {}", e);
                        Err(LinuxError::EINVAL)
                    }
                }
            }
            File::Cached { offset, .. } => {
                let size = self.size() as u64;
                let offset_val = offset.load(Ordering::Relaxed) as u64;
                let new_offset = match pos {
                    SeekFrom::Start(pos) => Some(pos),
                    SeekFrom::Current(off) => offset_val.checked_add_signed(off),
                    SeekFrom::End(off) => size.checked_add_signed(off),
                }
                .unwrap_or(0);
                offset.store(new_offset as usize, Ordering::SeqCst);
                Ok(new_offset as isize)
            }
        }
    }

    pub fn fsync(&self) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => Ok(0),
            File::Cached { .. } => {
                let cache = self.cache();
                Ok(cache.sync()?)
            }
        }
    }

    pub fn read_at(&self, buf: &mut [u8], offset: usize) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => {
                let inner = self.inner();
                Ok(inner.read_at(offset as u64, buf)?)
            }
            File::Cached { .. } => {
                let cache = self.cache();
                Ok(cache.read_at(offset, buf))
            }
        }
    }

    pub fn write_at(&self, buf: &[u8], offset: usize) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => {
                let inner = self.inner();
                Ok(inner.write_at(offset as u64, buf)?)
            }
            File::Cached { .. } => {
                let cache = self.cache();
                Ok(cache.write_at(offset, buf))
            }
        }
    }

    pub fn truncate(&self, offset: usize) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => {
                let inner = self.inner();
                inner.truncate(offset as u64)?;
                Ok(offset)
            }
            File::Cached { .. } => {
                let cache = self.cache();
                Ok(cache.truncate(offset) as usize)
            }
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        debug!("Starry-api drop file {}", self.path());
        if let File::Cached { .. } = self {
            let cache_manager = page_cache_manager();
            cache_manager.close_page_cache(self.path());
        }
    }
}

impl FileLike for File {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => {
                let mut inner = self.inner();
                Ok(inner.read(buf)?)
            }
            File::Cached { offset, .. } => {
                let cache = self.cache();
                let offset_val = offset.load(Ordering::SeqCst);
                let len = cache.read_at(offset_val, buf);
                offset.fetch_add(len, Ordering::SeqCst);
                Ok(len)
            }
        }
    }

    fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        match self {
            File::Direct { .. } => {
                let mut inner = self.inner();
                Ok(inner.write(buf)?)
            }
            File::Cached { offset, size, .. } => {
                let cache = self.cache();
                let offset_val = offset.load(Ordering::SeqCst);
                let len = cache.write_at(offset_val, buf);
                offset.fetch_add(len, Ordering::SeqCst);
                size.fetch_max(offset_val + len, Ordering::SeqCst);
                Ok(len)
            }
        }
    }

    fn stat(&self) -> LinuxResult<Kstat> {
        match self {
            File::Direct { .. } => {
                let inner = self.inner();
                let metadata = inner.get_attr()?;
                let ty = metadata.file_type() as u8;
                let perm = metadata.perm().bits() as u32;
                let size = self.size();
                Ok(Kstat {
                    mode: ((ty as u32) << 12) | perm,
                    size: size as u64,
                    blocks: metadata.blocks(),
                    blksize: 512,
                    ..Default::default()
                })
            }
            File::Cached { .. } => {
                let cache = self.cache();
                cache.stat()
            }
        }
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
