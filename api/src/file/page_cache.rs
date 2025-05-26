use core::clone;

use alloc::{collections::{btree_map::BTreeMap, btree_set::BTreeSet, linked_list::LinkedList}, string::String, sync::{Arc, Weak}, vec::{self, Vec}};
use axfs::api::File;
use axio::SeekFrom;
use axsync::{Mutex, MutexGuard};
use linux_raw_sys::general::FD_CLOEXEC;
use spin::{rwlock::RwLock, Lazy, RwLockReadGuard};
use axalloc::GlobalPage;
use axerrno::LinuxResult;
use memory_addr::{PhysAddr, VirtAddr, PAGE_SIZE_4K};
use super::Kstat;
use hashbrown::{hash_map::HashMap, HashSet};
use axerrno::ax_err_type;

struct FilePage {
    inner: GlobalPage,
    page_cache_id: usize,
    page_id: usize,
}

impl FilePage {
    pub fn new(page_cache_id: usize, page_id: usize) -> Self {
        FilePage { 
            inner: GlobalPage::alloc().expect("GlobalPage alloc failed"),
            page_cache_id,
            page_id,
        }
    }
}

/// 功能：
///     1. 主要目的是限制内核中 FilePage 的总数
///     2. 维护页面置换，并在 drop 页面时自动写回文件
///     3. 服务 FilePageCache 对文件页面的申请、释放
/// 注意：
///     1. 并发不安全，只能通过 page_cache_manager 访问（自动加锁）
///     2. 当 page_pool 扔掉 FilePage 的 Arc 指针，即说明这个页面被置换掉。因此
///     只有这里会长久存有 FilePage 的 Arc 指针，其他地方要么是 Weak，要么只是临时使用 Arc
pub struct PageCacheManager {
    open_cnt: BTreeMap<String, usize>,
    paths: HashMap<i32, String>,
    page_cache_id: HashMap<String, usize>,
    page_cache_ptr: HashMap<usize, Arc<FilePageCache>>,
    pages: BTreeMap::<VirtAddr, Arc<RwLock<FilePage>>>,
    id_cnt: usize,
    max_size: usize,
}

pub fn open_page_cache(path: &String, fd: i32, file: Arc<Mutex<axfs::fops::File>>) {
    let mut manager = page_cache_manager();
    manager.open_page_cache(path, fd, file);

}

pub fn get_page_cache(path: &String) -> Option<Arc<FilePageCache>> {
    let mut manager = page_cache_manager();
    manager.get_page_cache(path)
}

pub fn try_close_page_cache(fd: i32) {
    let mut manager = page_cache_manager();
    manager.try_close_page_cache(fd);
}

impl PageCacheManager {
    pub fn new(max_size: usize) -> Self {
        Self { 
            open_cnt: BTreeMap::new(),
            paths: HashMap::new(),
            page_cache_id: HashMap::new(),
            page_cache_ptr: HashMap::new(),
            pages: BTreeMap::new(),
            id_cnt: 0,
            max_size, 
        }
    }
    
    pub fn open_page_cache(&mut self, path: &String, fd: i32, file: Arc<Mutex<axfs::fops::File>>) {
        self.paths.insert(fd, path.clone());

        if !self.open_cnt.contains_key(path) {
            self.open_cnt.insert(path.clone(), 1);
        } else {
            self.open_cnt.entry(path.clone()).or_insert(1);
            return;
        }
    
        self.id_cnt += 1; 
        let page_cache = Arc::new(FilePageCache::new(self.id_cnt, file));
        self.page_cache_id.insert(path.clone(), self.id_cnt);
        self.page_cache_ptr.insert(self.id_cnt, page_cache.clone());
    }

    pub fn get_page_cache(&self, path: &String) -> Option<Arc<FilePageCache>> {
        if let Some(id) = self.page_cache_id.get(path) {
            return self.page_cache_ptr.get(id).cloned();
        }
        None
    }

    pub fn try_close_page_cache(&mut self, fd: i32) {
        if let Some(path) = self.paths.remove(&fd) {
            let cnt = self.open_cnt.get_mut(&path).unwrap();
            *cnt -= 1;
    
            if *cnt == 0 {
                let id = {
                    let page_cache_id = &self.page_cache_id;  // 不可变借用 self.page_cache_id
                    *page_cache_id.get(&path).unwrap()
                };
                self.open_cnt.remove(&path);
                self.page_cache_id.remove(&path);
                self.page_cache_ptr.remove(&id);
            }
        }
    }

    // pub fn try_close_page_cache_by_path(&mut self, path: &String) {
    //     if !self.open_cnt.contains_key(path) {
    //         return;
    //     }
    //     let cnt = self.open_cnt.get_mut(path).unwrap();
    //     *cnt -= 1;
    //     if cnt == 0 {

    //     }
    //     if let Some(id) = self.page_cache_id.remove(path) {
    //         self.page_cache_ptr.remove(&id);
    //         self.fds.remove(path);
    //     }
    // }

    pub fn resize(&mut self, size: usize) {
        // TODO: 检查 size 的合法范围
        self.max_size = size;
        while self.pages.len() > self.max_size {
            self.drop_page();
        }
    }

    pub fn alloc_page(&mut self, page_cache_id: usize, page_id: usize) -> VirtAddr {
        if self.pages.len() == self.max_size {
            self.drop_page();
        }
        
        let page = Arc::new(RwLock::new(FilePage::new(page_cache_id, page_id)));
        let addr = {
            let page = page.read();
            page.inner.start_vaddr()
        };
        warn!("Kernel Page Pool alloc page: {:#x}", addr);
        self.pages.insert(addr, page);
        addr
    }

    pub fn dealloc_page(&mut self, start_addr: VirtAddr) {
        {
            // 这里的目的是确保没有其他进程正在使用这个 Arc<RwLock<PageFile>> 指针
            // 即避免并发场景下，KernelPagePool 扔掉 FilePage，但是临时的 FilePage Arc 指针的失效问题
            let page =  self.pages.get(&start_addr).unwrap();
            let _page_write = page.write();
            unsafe {page.force_write_unlock();}
        }
        self.pages.remove(&start_addr);
    }

    /// 获取一个页面
    pub fn get_page(&mut self, start_addr: VirtAddr) -> Option<Arc<RwLock<FilePage>>> {
        let page = self.pages.get(&start_addr);
        if page.is_none() {
            return None;
        }
        let page = page.unwrap().clone();
        Some(page)
    }

    // TODO: 完善页面置换算法，目前只是把虚拟地址最低的页面给扔掉
    fn find_page_drop(&mut self) -> VirtAddr {
        let (&vaddr, _) = self.pages.first_key_value().unwrap();
        vaddr
    }
    
    /// TODO: 通过页表检查是否为脏页，只有脏页才需写回
    fn drop_page(&mut self) {
        let vaddr = self.find_page_drop();
        let page = self.get_page(vaddr).unwrap();
        let (page_cache_id, page_id) = {
            let page = page.read();
            (page.page_cache_id, page.page_id)
        };
        warn!("KernelPagePool drop page: vaddr {:#x}, page_cache_id {}, page_id {}",
            vaddr, page_cache_id, page_id);
        let page_cache = self.page_cache_ptr.get(&page_cache_id).unwrap();
        page_cache.drop_page(page_id, page.read());
        self.dealloc_page(vaddr);
        warn!("KernelPagePool drop page: vaddr {:#x}", vaddr);
    }
}

const DEFAULT_PAGE_POOL_SIZE: usize = 5;
pub static PAGE_CACHE_MANAGER: Lazy<Mutex<PageCacheManager>> = Lazy::new(|| {Mutex::new(
    PageCacheManager::new(DEFAULT_PAGE_POOL_SIZE)
)});

pub fn page_cache_manager() -> MutexGuard<'static, PageCacheManager> {
    PAGE_CACHE_MANAGER.lock()
}
/// 接管 syscall 层对 axfs::fops::File 的操作
/// 包括 read, write, seek, stat，还需要实现同步落盘机制
pub struct FilePageCache {
    id: RwLock<usize>,
    file: Arc<Mutex<axfs::fops::File>>,
    offset: RwLock<usize>,
    stat: RwLock<Kstat>,
    pages: RwLock<BTreeMap<usize, VirtAddr>>,
    dirty_pages: RwLock<BTreeSet<usize>>,
}

impl FilePageCache {
    pub fn new(id: usize, file: Arc<Mutex<axfs::fops::File>>) -> Self {
        // stat 的初始化照抄 api/src/file/fs.rs
        let (offset, stat) = {
            let file_lock = file.lock(); // 获取文件锁
            let offset = file_lock.get_offset() as usize;
            
            let metadata = file_lock.get_attr().unwrap();
            let ty = metadata.file_type() as u8;
            let perm = metadata.perm().bits() as u32;
            let stat = Kstat {
                mode: ((ty as u32) << 12) | perm,
                size: metadata.size(),
                blocks: metadata.blocks(),
                blksize: 512, 
                ..Default::default()
            };
            
            (offset, stat)
        };

        Self {
            id: RwLock::new(id),
            file: Arc::clone(&file), 
            offset: RwLock::new(offset),
            stat: RwLock::new(stat),
            pages: RwLock::new(BTreeMap::new()),
            dirty_pages: RwLock::new(BTreeSet::new()),
        }
    }

    /// 根据 page_id 取出一个缓存页面，首次取出会从文件加载
    /// 上层会检查超出文件范围的问题
    fn get_page(&self, page_id: usize) -> Arc<RwLock<FilePage>> {
        let mut manager = page_cache_manager();
        
        // 从页缓存里面找，为了提升性能先尝试在不获取写锁的情况下查找页面
        {
            let pages = self.pages.read();
            if let Some(&start_addr) = pages.get(&page_id) {
                let page = manager.get_page(start_addr).unwrap();
                return page;
            }
        }

        let vaddr = {
            // 从 manager 里面申请一个页面，并插入自己的数据结构
            /// 这一条调用链极其容易发生死锁：
            ///     FilePageCache::get_page 
            ///  => PageCacheManager::alloc_page 
            ///  => PageCacheManager::drop_page
            ///  => FilePageCache::flush_page

            let vaddr = {
                let id = self.id.read();
                manager.alloc_page(*id, page_id)
            };

            /// 这样的写法是同时兼顾两个目的：
            /// 1. 避免一个隐蔽的并发 bug：防止在等待写锁期间其他进程已经创建了该页面
            /// 2. 避免死锁：若先上写锁，再尝试 manager.get_page，有概率在页面置换时发生死锁
            
            let mut pages = self.pages.write();
            if let Some(&start_addr) = pages.get(&page_id) {
                let page = manager.get_page(start_addr).unwrap();
                manager.dealloc_page(vaddr);
                return page;
            }
            pages.insert(page_id, vaddr);
            vaddr
        };

        // 从文件加载
        let page = manager.get_page(vaddr).unwrap();
        {
            let mut page = page.write();
            let mut file = self.file.lock();
            let mut len = 0;
            file.seek(SeekFrom::Start((page_id * PAGE_SIZE_4K) as u64)).unwrap();
            loop {
                let offset = (page_id * PAGE_SIZE_4K) as u64;
                let buf = &mut page.inner.as_slice_mut()[len..];
                let add = match file.read(buf) {
                    Ok(add) => add,
                    Err(e) => {
                        break;
                    }
                };
                if add == 0 {
                    break;
                }
                len = len + add;
            }
        }
        page
    }

    pub fn get_vaddr(&self, offset: usize) -> VirtAddr {
        let aligned_offset = memory_addr::align_down_4k(offset);
        let page_id = aligned_offset / PAGE_SIZE_4K;
        let page = self.get_page(page_id);
        let page = page.read();
        assert!(self.pages.read().contains_key(&page_id), "Page not found in cache");
        VirtAddr::from_usize(offset - aligned_offset + page.inner.start_vaddr().as_usize())
    }
    
    fn read_slice_from_page(&self, page_id: usize, page_start: usize, page_end: usize, buf: &mut [u8]) {
        let page = self.get_page(page_id);
        let page = page.read();
        let slice = page.inner.as_slice();
        buf.copy_from_slice(&slice[page_start..page_end])
    }

    fn write_slice_into_page(&self, page_id: usize, page_start: usize, page_end: usize, buf: &[u8]) {
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.insert(page_id);
        }
        let page = self.get_page(page_id);
        let mut page = page.write();
        let slice = page.inner.as_slice_mut();
        slice[page_start..page_end].copy_from_slice(&buf);
    }
    
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> LinuxResult<usize> {
        let file_len = {
            let stat = self.stat.read();
            stat.size as usize
        };
        let read_len =  buf.len().min(file_len - offset);
        
        if offset >= file_len || read_len == 0 {
            return Ok(0);
        }
        let mut ret = 0 as usize;
        while ret < read_len {
            let current_pos = offset + ret;
            let page_id = current_pos / PAGE_SIZE_4K;
            let page_offset_start = current_pos % PAGE_SIZE_4K;
            // 计算当前页剩余空间
            let bytes_left_in_page = PAGE_SIZE_4K - page_offset_start;
            // 计算本次可读取的最大长度（不超过页尾和总读取长度）
            let cur_len = bytes_left_in_page.min(read_len - ret);
            let slice = &mut buf[ret..ret + cur_len];
            self.read_slice_from_page(page_id, page_offset_start, page_offset_start + cur_len, slice);
            ret += cur_len;
        }
        Ok(ret)
    }

    pub fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        let offset = {
            let offset_lock = self.offset.read();
            *offset_lock
        };
        
        let result = self.read_at(offset, buf);
        if let Ok(bytes_read) = result {
            let mut offset_lock = self.offset.write();
            *offset_lock += bytes_read;
        }
        result
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> LinuxResult<usize> {
        error!("FilePageCache::write_at: offset = {}, buf.len() = {}", offset, buf.len());
        
        let mut ret = 0;
        let write_len = buf.len();

        while ret < write_len {
            let current_pos = offset + ret;
            let page_id = current_pos / PAGE_SIZE_4K;
            let page_offset_start = current_pos % PAGE_SIZE_4K;
            // 计算当前页剩余空间
            let bytes_left_in_page = PAGE_SIZE_4K - page_offset_start;
            // 计算本次可写入的最大长度
            let cur_len = bytes_left_in_page.min(write_len - ret);
            
            //将 buf 的数据写入 pagecache，自动标记脏页
            let slice = &buf[ret..ret + cur_len];
            self.write_slice_into_page(page_id, page_offset_start, page_offset_start + cur_len, slice);
            ret += cur_len;
        }

        // 更新 stat
        if offset + ret > { self.stat.read().size as usize } {
            let mut stat = self.stat.write();
            stat.size = (offset + ret) as u64;
        }
        Ok(ret)
    }

    pub fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        let offset = {
            let offset_lock = self.offset.read();
            *offset_lock
        };

        let result = self.write_at(offset, buf);

        if let Ok(bytes_written) = result {
            let mut offset_lock = self.offset.write();
            let mut stat_lock = self.stat.write();
            *offset_lock += bytes_written;
        
            if *offset_lock > stat_lock.size as usize {
                error!("write: updating file size from {} to {}", stat_lock.size, *offset_lock);
                stat_lock.size = *offset_lock as u64;
            }
        }
        result
    }

    pub fn stat(&self) -> LinuxResult<Kstat> {
        let stat = self.stat.read();
        Ok(*stat)
    }

    pub fn seek(&self, pos: SeekFrom) -> LinuxResult<isize> {
        let size = { self.stat.read().size };
        let mut offset = self.offset.write();
        let new_offset = match pos {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(off) => (*offset as u64).checked_add_signed(off),
            SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .ok_or_else(|| ax_err_type!(InvalidInput))?;

        *offset = new_offset as usize;
        Ok(new_offset as isize)
    }

    fn flush_page(&self, page_id: usize) -> LinuxResult<isize> {        
        let page_exists = {
            let pages = self.pages.read();
            pages.contains_key(&page_id)
        };
        
        if !page_exists {
            return Ok(0);
        }
        let page = self.get_page(page_id);
        let page = page.read();
        let page_start = page_id * PAGE_SIZE_4K;
        
        let length = {
            let stat = self.stat.read();
            if page_start >= stat.size as usize {
                0 // 页面起始位置已超出文件大小
            } else {
                let remaining = stat.size as usize - page_start;
                remaining.min(PAGE_SIZE_4K)
            }
        };
        
        if length == 0 {
            return Ok(0);
        }
        
        let mut file = self.file.lock();
        file.seek(SeekFrom::Start((page_id * PAGE_SIZE_4K) as u64))?;
        let mut cur = 0;
        loop {
            let slice = &page.inner.as_slice()[cur..length];
            let write_length = file.write(slice)?;
            cur += write_length;
            if cur == length {
                break;
            }
        }
        Ok(0)
    }

    /// ! 只允许被 PageCacheManager 调用，用于页面置换
    pub fn drop_page(&self, page_id: usize, page: RwLockReadGuard<FilePage>) -> LinuxResult<isize> {        
        {
            let pages = self.pages.read();
            if !pages.contains_key(&page_id) {
                return Ok(0);
            }
        };
        // 确定写回范围
        let page_start = page_id * PAGE_SIZE_4K;
        let length = {
            let stat = self.stat.read();
            if page_start >= stat.size as usize {
                0 // 页面起始位置已超出文件大小
            } else {
                let remaining = stat.size as usize - page_start;
                remaining.min(PAGE_SIZE_4K)
            }
        };
        // 写入文件
        let mut file = self.file.lock();
        file.seek(SeekFrom::Start((page_id * PAGE_SIZE_4K) as u64))?;
        let mut cur = 0;
        loop {
            let slice = &page.inner.as_slice()[cur..length];
            let write_length = file.write(slice)?;
            cur += write_length;
            if cur == length {
                break;
            }
        }
        // 在本 struct 中删除记录
        let mut pages = self.pages.write();
        pages.remove(&page_id);
        let mut dirty_pages = self.dirty_pages.write();
        dirty_pages.remove(&page_id);
        Ok(0)
    }

    fn update_metadata(&self) {
        // 更新文件大小
        let stat = { self.stat.read() };
        let mut file = self.file.lock();
        file.truncate(stat.size);
        error!("FilePageCache: update file size to {}", stat.size);

        // 更新文件指针
        let offset = { self.offset.read().clone() };
        let pos = SeekFrom::Start(offset as u64);
        file.seek(pos);
        // TODO: 不知道还有没有其他信息需要更新
    }
    
    pub fn fsync(&self) -> LinuxResult<isize> {
        let dirty_pages : Vec<usize> = {
            let dirty_pages_lock = self.dirty_pages.read();
            dirty_pages_lock.iter().copied().collect()
        };
        for page_id in dirty_pages {
            self.flush_page(page_id)?;
        }
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.clear();
        }
        self.update_metadata();
        Ok(0)
    }

    pub fn msync(&self) -> LinuxResult<isize> {
        let page_ids: Vec<usize> = {
            let pages = self.pages.read();
            pages.keys().copied().collect()
        };
        for page_id in page_ids {
            self.flush_page(page_id)?;
        }
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.clear();
        }
        self.update_metadata();
        Ok(0)
    }

    pub fn clear(&self) -> LinuxResult<isize> {
        self.msync()?;
        
        let mut pool = page_cache_manager();
        let page_addrs: Vec<VirtAddr> = {
            let pages = self.pages.read();
            pages.values().copied().collect()
        };
        for vaddr in page_addrs {
            pool.dealloc_page(vaddr);
        }
        {
            let mut pages = self.pages.write();
            pages.clear();
        }
        Ok(0)
    }
}