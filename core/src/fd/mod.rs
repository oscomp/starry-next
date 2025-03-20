mod fs;
mod net;
mod pipe;
mod stdio;

use core::{any::Any, ffi::c_int};

use alloc::sync::Arc;
use axerrno::{LinuxError, LinuxResult};
use axio::PollState;
use axns::{ResArc, def_resource};
use flatten_objects::FlattenObjects;
use spin::RwLock;

use crate::ctypes::stat;

pub use self::{
    fs::{Directory, File},
    net::Socket,
    pipe::Pipe,
};

pub const AX_FILE_LIMIT: usize = 1024;

#[allow(dead_code)]
pub trait FileLike: Send + Sync {
    fn read(&self, buf: &mut [u8]) -> LinuxResult<usize>;
    fn write(&self, buf: &[u8]) -> LinuxResult<usize>;
    fn stat(&self) -> LinuxResult<stat>;
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
    fn poll(&self) -> LinuxResult<PollState>;
    fn set_nonblocking(&self, nonblocking: bool) -> LinuxResult;

    fn from_fd(fd: c_int) -> LinuxResult<Arc<Self>>
    where
        Self: Sized + 'static,
    {
        get_file_like(fd)?
            .into_any()
            .downcast::<Self>()
            .map_err(|_| LinuxError::EINVAL)
    }

    fn add_to_fd_table(self) -> LinuxResult<c_int>
    where
        Self: Sized + 'static,
    {
        add_file_like(Arc::new(self))
    }
}

def_resource! {
    pub static FD_TABLE: ResArc<RwLock<FlattenObjects<Arc<dyn FileLike>, AX_FILE_LIMIT>>> = ResArc::new();
}

impl FD_TABLE {
    /// Return a copy of the inner table.
    pub fn copy_inner(&self) -> RwLock<FlattenObjects<Arc<dyn FileLike>, AX_FILE_LIMIT>> {
        let table = self.read();
        let mut new_table = FlattenObjects::new();
        for id in table.ids() {
            let _ = new_table.add_at(id, table.get(id).unwrap().clone());
        }
        RwLock::new(new_table)
    }
}

/// Get a file-like object by `fd`.
pub fn get_file_like(fd: c_int) -> LinuxResult<Arc<dyn FileLike>> {
    FD_TABLE
        .read()
        .get(fd as usize)
        .cloned()
        .ok_or(LinuxError::EBADF)
}

/// Add a file to the file descriptor table.
pub fn add_file_like(f: Arc<dyn FileLike>) -> LinuxResult<c_int> {
    Ok(FD_TABLE.write().add(f).map_err(|_| LinuxError::EMFILE)? as c_int)
}

/// Close a file by `fd`.
pub fn close_file_like(fd: c_int) -> LinuxResult {
    let f = FD_TABLE
        .write()
        .remove(fd as usize)
        .ok_or(LinuxError::EBADF)?;
    drop(f);
    Ok(())
}

#[ctor_bare::register_ctor]
fn init_stdio() {
    let mut fd_table = flatten_objects::FlattenObjects::new();
    fd_table
        .add_at(0, Arc::new(stdio::stdin()) as _)
        .unwrap_or_else(|_| panic!()); // stdin
    fd_table
        .add_at(1, Arc::new(stdio::stdout()) as _)
        .unwrap_or_else(|_| panic!()); // stdout
    fd_table
        .add_at(2, Arc::new(stdio::stdout()) as _)
        .unwrap_or_else(|_| panic!()); // stderr
    FD_TABLE.init_new(spin::RwLock::new(fd_table));
}
