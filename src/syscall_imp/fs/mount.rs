use alloc::vec::Vec;
use arceos_posix_api::{AT_FDCWD, FilePath, char_ptr_to_str, handle_file_path};
use axerrno::{LinuxError, LinuxResult};
use axsync::Mutex;
use core::ffi::{c_char, c_void};

pub(crate) fn sys_mount(
    source: *const c_char,
    target: *const c_char,
    fs_type: *const c_char,
    _flags: i32,
    _data: *const c_void,
) -> LinuxResult<isize> {
    info!("sys_mount");
    let device_path = handle_file_path(AT_FDCWD, Some(source as _), false)?;
    let mount_path = handle_file_path(AT_FDCWD, Some(target as _), true)?;
    let fs_type = char_ptr_to_str(fs_type)?;
    info!(
        "mount {:?} to {:?} with fs_type={:?}",
        device_path, mount_path, fs_type
    );

    if fs_type != "vfat" {
        debug!("fs_type can only be vfat.");
        return Err(LinuxError::EPERM);
    }
    // 检查挂载点路径是否存在
    if !mount_path.exists() {
        debug!("mount path not exist");
        return Err(LinuxError::EPERM);
    }
    // 查挂载点是否已经被挂载
    if check_mounted(&mount_path) {
        debug!("mount path includes mounted fs");
        return Err(LinuxError::EPERM);
    }
    // 挂载
    if !mount_fat_fs(&device_path, &mount_path) {
        debug!("mount error");
        return Err(LinuxError::EPERM);
    }
    Ok(0)
}

pub(crate) fn sys_umount2(target: *const c_char, flags: i32) -> LinuxResult<isize> {
    info!("sys_mount");
    let mount_path = handle_file_path(AT_FDCWD, Some(target as _), true)?;
    if flags != 0 {
        debug!("flags unimplemented");
        return Err(LinuxError::EPERM);
    }

    // 检查挂载点路径是否存在
    if !mount_path.exists() {
        debug!("mount path not exist");
        return Err(LinuxError::EPERM);
    }
    // 从挂载点中删除
    if !umount_fat_fs(&mount_path) {
        debug!("umount error");
        return Err(LinuxError::EPERM);
    }
    Ok(0)
}

/// 挂载的文件系统。
/// 目前"挂载"的语义是，把一个文件当作文件系统读写
pub struct MountedFs {
    //pub inner: Arc<Mutex<FATFileSystem>>,
    pub device: FilePath,
    pub mnt_dir: FilePath,
}

impl MountedFs {
    pub fn new(device: &FilePath, mnt_dir: &FilePath) -> Self {
        assert!(
            device.is_file() && mnt_dir.is_dir(),
            "device must be a file and mnt_dir must be a dir"
        );
        Self {
            device: device.clone(),
            mnt_dir: mnt_dir.clone(),
        }
    }
    #[allow(unused)]
    pub fn device(&self) -> FilePath {
        self.device.clone()
    }

    pub fn mnt_dir(&self) -> FilePath {
        self.mnt_dir.clone()
    }
}

/// 已挂载的文件系统(设备)。
/// 注意启动时的文件系统不在这个 vec 里，它在 mod.rs 里。
static MOUNTED: Mutex<Vec<MountedFs>> = Mutex::new(Vec::new());

/// 挂载一个fatfs类型的设备
pub fn mount_fat_fs(device_path: &FilePath, mount_path: &FilePath) -> bool {
    // // device_path需要链接转换, mount_path不需要, 因为目前目录没有链接  // 暂时只有Open过的文件会加入到链接表，所以这里先不转换
    // debug!("mounting {} to {}", device_path.path(), mount_path.path());
    // if let Some(true_device_path) = real_path(device_path) {
    if mount_path.exists() {
        MOUNTED.lock().push(MountedFs::new(device_path, mount_path));
        info!(
            "mounted {} to {}",
            device_path.as_str(),
            mount_path.as_str()
        );
        return true;
    }
    // }
    info!(
        "mount failed: {} to {}",
        device_path.as_str(),
        mount_path.as_str()
    );
    false
}

/// 卸载一个fatfs类型的设备
pub fn umount_fat_fs(mount_path: &FilePath) -> bool {
    let mut mounted = MOUNTED.lock();
    let mut i = 0;
    while i < mounted.len() {
        if mounted[i].mnt_dir() == *mount_path {
            mounted.remove(i);
            info!("umounted {}", mount_path.as_str());
            return true;
        }
        i += 1;
    }
    info!("umount failed: {}", mount_path.as_str());
    false
}

/// 检查一个路径是否已经被挂载
pub fn check_mounted(path: &FilePath) -> bool {
    let mounted = MOUNTED.lock();
    for m in mounted.iter() {
        if path.starts_with(&m.mnt_dir()) {
            debug!("{} is mounted", path.as_str());
            return true;
        }
    }
    false
}
