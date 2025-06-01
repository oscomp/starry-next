use alloc::{collections::{btree_map::BTreeMap, btree_set::BTreeSet, linked_list::LinkedList}, 
            string::String, sync::{Arc, Weak}, vec, vec::Vec};
use axio::SeekFrom;
use axsync::Mutex;
use spin::rwlock::RwLock;
use axalloc::GlobalPage;
use axerrno::{LinuxError, LinuxResult};
use memory_addr::{PhysAddr, VirtAddr, PAGE_SIZE_4K};
use super::Kstat;
use hashbrown::hash_map::HashMap;
use axerrno::ax_err_type;
use lazy_static::lazy_static;
use axtask::{TaskExtRef, current};


// TODO 现在的数据结构不好，需要实现 struct FileAddressSpace

#[derive(Clone)]
struct MmapArea {
    start: VirtAddr,
    length: usize,
    fd: i32,
    offset: usize,
}

struct ProcessMmapAreaManager {
    areas: Mutex<BTreeMap<VirtAddr, MmapArea>>,
}

impl ProcessMmapAreaManager {
    pub fn new() -> Self {
        Self {
            areas: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn add_area(&mut self, start: VirtAddr, length: usize, fd: i32, offset: usize) -> LinuxResult<isize> {
        // TODO 识别与左右区间相交的错误
        let mut areas = self.areas.lock();
        if length == 0 || start.as_usize() % PAGE_SIZE_4K != 0 || offset % PAGE_SIZE_4K != 0 {
            return Err(LinuxError::EINVAL);
        }
        areas.insert(start, MmapArea { start: start, length, fd, offset });
        Ok(0)
    }
    
    pub fn remove_area(&mut self, start: VirtAddr) -> LinuxResult<isize> {
        let mut areas = self.areas.lock();
        areas.remove(&start);
        Ok(0)
    }
    
    pub fn query(&mut self, vaddr: &VirtAddr) -> Option<MmapArea> {
        let areas = self.areas.lock();
        if let Some((start, area)) = areas.range(..=vaddr).next_back() {
            if vaddr >= start && vaddr.as_usize() < start.as_usize() + area.length {
                return Some(area.clone());
            }
        }
        None
    }
}

pub struct MmapAreaManager {
    areas: Mutex<HashMap<u32, ProcessMmapAreaManager>>
}

impl MmapAreaManager {
    pub fn new() -> Self {
        Self {
            areas: Mutex::new(HashMap::new()),
        }
    }

    pub fn mmap(&self, pid: u32, start: VirtAddr, length: usize, fd: i32, offset: usize) -> LinuxResult<isize> {
        let mut areas = self.areas.lock();
        let process_areas = areas.entry(pid)
                                                              .or_insert_with(ProcessMmapAreaManager::new);
        process_areas.add_area(start, length, fd, offset)
    }

    pub fn munmap(&self, pid: u32, start: VirtAddr) -> LinuxResult<isize> {
        let mut areas = self.areas.lock();
        if let Some(process_areas) = areas.get_mut(&pid) {
            return process_areas.remove_area(start);
        } 
        Ok(0)
    }

    pub fn find_paddr(&self, pid: u32, vaddr: &VirtAddr) -> Option<PhysAddr> {
        let mut areas = self.areas.lock();
        if let Some(process_areas) = areas.get_mut(&pid) {
            if let Some(area) = process_areas.query(&vaddr) {
                let offset = vaddr.as_usize() - area.start.as_usize();
                let manager = page_cache_manager();
                let path = manager.find_path(area.fd).unwrap();
                let cache = manager.get_page_cache(&path).unwrap();
                let cache = cache.upgrade().unwrap();

                let vaddr = cache.get_vaddr(offset);
                let paddr = axhal::mem::virt_to_phys(vaddr);
                return Some(paddr);
            }
        }
        None
    }

    pub fn msync(&self, pid: u32, start: VirtAddr, length: usize) -> LinuxResult<isize> {
        let mut areas = self.areas.lock();
        if let Some(process_areas) = areas.get_mut(&pid) {
            if let Some(area) = process_areas.query(&start) {
                let offset = start.as_usize() - area.start.as_usize();
                let manager = page_cache_manager();
                let path = manager.find_path(area.fd).unwrap();
                let cache = manager.get_page_cache(&path).unwrap();
                let cache = cache.upgrade().unwrap();

                let start_page_id = offset / PAGE_SIZE_4K;
                let end_page_id = (offset + length) / PAGE_SIZE_4K;
                for page_id in start_page_id..end_page_id {
                    cache.flush_page(page_id)?;
                }
                return Ok(0);
            }
        }
        Ok(0)
    }
}

lazy_static! {
    static ref MMAP_AREA_MANAGER: Arc<MmapAreaManager> = Arc::new(
        MmapAreaManager::new()
    );
}

pub fn mmap_area_manager() -> Arc<MmapAreaManager> {
    MMAP_AREA_MANAGER.clone()
}

/// 检查一个页是否为脏页
fn check_dirty(vaddr: VirtAddr) -> bool {
    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mut aspace = process_data.aspace.lock();
    aspace.check_dirty(vaddr)
}

// 清除一个页的脏页标记
fn clear_dirty(vaddr: VirtAddr) {
    let curr = current();
    let process_data = curr.task_ext().process_data();
    let mut aspace = process_data.aspace.lock();
    aspace.set_dirty(vaddr, false);
}

// 文件页，在创建的时候自动加载文件内容
// 并发不安全，需要上层加读写锁
// TODO: 
struct Page {
    inner: GlobalPage,                  // 16 bytes
    file: Weak<Mutex<axfs::fops::File>>, // 16 bytes
    page_id: usize,                     // 8 bytes
}

impl Page {
    fn new(file: Weak<Mutex<axfs::fops::File>>, page_id: usize) -> Self {
        let inner =  GlobalPage::alloc().expect("GlobalPage alloc failed");
        let mut page = Page { inner, file, page_id };
        page.load();
        clear_dirty(page.inner.start_vaddr());
        page
    }
    
    /// 只有 Page::load 涉及 axfs 层的文件 read 操作
    /// 仅由 Page::new 调用，上层没有使用权限
    fn load(&mut self) {
        let file_arc = { self.file.upgrade().unwrap() };
        let offset = (self.page_id * PAGE_SIZE_4K) as u64;
        let mut file = file_arc.lock();
        file.seek(SeekFrom::Start(offset)).unwrap();

        let buf = self.inner.as_slice_mut();
        let mut bytes_read = 0;
        loop {
            match file.read(&mut buf[bytes_read..]) {
                Ok(0) => break,
                Ok(n) => bytes_read += n,
                _ => break,
            }
        }
    }

    /// 只有 Page::flush 涉及 axfs 层的文件 write 操作，并清空脏位标记
    /// 可以由上层主动调用
    /// 把整个 4K 都写入磁盘，如果超额写入则交给上层 PagePool 调用 axfs 层的 truncate
    fn flush(&mut self) -> LinuxResult<isize> {
        if !check_dirty(self.inner.start_vaddr()) {
            return Ok(0);
        }

        let file_arc = self.file.upgrade().expect("File has been dropped before flush");
        let mut file = file_arc.lock();
        let offset = self.page_id * PAGE_SIZE_4K;
        file.seek(SeekFrom::Start(offset as u64)).expect("Flush page failed to seek");

        let buf = self.inner.as_slice();
        let mut bytes_write = 0;
        loop {
            match file.write(&buf[bytes_write..]) {
                Ok(0) => break,
                Ok(n) => bytes_write += n,
                Err(e) => {
                    error!("Write failed at offset {}: {:?}", offset + bytes_write, e);
                    return Err(e.into());
                }
            }
        }

        clear_dirty(self.inner.start_vaddr());
        debug!("Flush page {} done, length = {}", self.page_id, bytes_write);
        Ok(0)
    }

    /// 从内存中的缓存页面中读取数据到 buf，返回读取的长度
    fn read(&self, offset: usize, buf: &mut [u8]) -> LinuxResult<isize> {
        let start = offset;
        let end = start + buf.len();
        if end > PAGE_SIZE_4K {
            return Err(LinuxError::EINVAL);
        }
        let slice = &self.inner.as_slice()[start..end];
        buf.copy_from_slice(slice);
        Ok((end - start) as isize)
    }

    /// 将 buf 中的数据写入到内存中的缓存页面，返回写入的长度
    fn write(&mut self, offset: usize, buf: &[u8]) -> LinuxResult<isize> {                
        let start = offset;
        let end = start + buf.len();
        if end > PAGE_SIZE_4K {
            return Err(LinuxError::EINVAL);
        }
        let slice = &mut self.inner.as_slice_mut()[start..end];
        slice.copy_from_slice(buf);

        self.flush();
        Ok((end - start) as isize)
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // TODO: Drop 后需要修改 mmap 的页表！
        debug!("Drop page {}", self.page_id);
        self.flush().unwrap_or_else(|_| {
            error!("Failed to Drop page {}", self.page_id);
            0
        });
    }
}

// 并发安全的文件页池
struct PagePool {
    // 注意文件页的虚拟地址是位于内核段（线性映射区），不要跟 mmap 的虚拟地址混淆
    pages: RwLock<BTreeMap::<VirtAddr, RwLock<Page>>>,
    max_size: Mutex<usize>,

    // 没有被修改过的页面
    clean_list: Mutex<LinkedList<VirtAddr>>,
    // 修改过的页面
    dirty_list: Mutex<LinkedList<VirtAddr>>,
    // 导致文件大小增加的页面，按分配顺序排列
    alloc_list: Mutex<LinkedList<VirtAddr>>,
}

impl PagePool {
    fn new(max_size: usize) -> Self {
        Self {
            pages: RwLock::new(BTreeMap::new()),
            max_size: Mutex::new(max_size),
            clean_list: Mutex::new(LinkedList::new()),
            dirty_list: Mutex::new(LinkedList::new()),
            alloc_list: Mutex::new(LinkedList::new()),
        }
    }
    
    // 暂时没用到，给以后留作扩展：根据内存使用情况，自动调整 PagePool 的大小
    fn resize(&self, size: usize) -> bool {
        // TODO: 检查 size 的合法范围
        let mut max_size = self.max_size.lock();
        *max_size = size;
        let cnt = {
            let pages = self.pages.read();
            (pages.len() - size).max(0)
        };
        for _ in 0..cnt {
            self.auto_drop_page();
        }
        true
    }

    // append 参数的含义：这个页面是否拓展了文件的页面数量
    fn alloc_page(&self, file: Weak<Mutex<axfs::fops::File>>, page_id: usize, append: bool) -> VirtAddr {
        {
            let max_size = self.max_size.lock();
            while self.pages.read().len() >= *max_size {
                self.auto_drop_page();
            }
        }
        
        let mut pages = self.pages.write();
        let page = RwLock::new(Page::new(file, page_id));

        let addr = {
            let page = page.read();
            page.inner.start_vaddr()
        };
        pages.insert(addr, page);
        
        if append {
            let mut list = self.alloc_list.lock();
            list.push_back(addr);
        } else {
            let mut list = self.clean_list.lock();
            list.push_back(addr);
        }
        
        debug!("Page Pool alloc page: {:#x}, append: {}", addr, append);
        addr
    }

    fn read_page(&self, vaddr: VirtAddr, offset: usize, buf: &mut [u8]) -> LinuxResult<isize> {
        let pages = self.pages.read();
        let page = pages.get(&vaddr).unwrap();
        let page = page.read();
        page.read(offset, buf)
    }
    
    fn write_page(&self, vaddr: VirtAddr, offset: usize, buf: &[u8]) -> LinuxResult<isize> {
        let pages = self.pages.read();
        let page = pages.get(&vaddr).unwrap();
        let mut page = page.write();
        page.write(offset, buf)
    }

    /// 自动页面置换
    fn auto_drop_page(&self) {        
        // 选择需要被丢弃的页面
        let drop_vaddr = {
            let mut clean_list = self.clean_list.lock();
            let mut dirty_list = self.dirty_list.lock();
            
            let mut drop_vaddr = VirtAddr::from(0);
            while !clean_list.is_empty() {
                let vaddr = clean_list.pop_front().unwrap();
                if check_dirty(vaddr) {
                    dirty_list.push_back(vaddr);
                } else {
                    drop_vaddr = vaddr;
                    break;
                }
            }

            if drop_vaddr.as_usize() != 0 {
                drop_vaddr
            } else if !dirty_list.is_empty() {
                dirty_list.pop_front().unwrap()
            } else {
                let mut alloc_list = self.alloc_list.lock();
                alloc_list.pop_front().unwrap()
            }
        };

        let mut pages = self.pages.write();
        pages.remove(&drop_vaddr);
        warn!("Auto drop page: vaddr {:#x}", drop_vaddr);
    }

    /// 将一个页面刷新回磁盘，保留原缓存页
    fn flush_page(&self, vaddr: VirtAddr) -> LinuxResult<isize> {
        let pages = self.pages.read();
        if let Some(page) = pages.get(&vaddr) {
            let mut page = page.write();
            page.flush()?;
        };
        // 这里不需要 clear dirty 标记，因为 page.flush 会自己清除脏位
        // 这里没有将该页从 dirty_list 或 clean_list 中清除
        // 因为即便仍留在 dirty_list，之后 drop_page 时检测到该页干净不会重复落盘，无不良影响
        Ok(0)
    }
 
    // 将 alloc_list 中的页面全都写回，目的是保证 fstat 能正确获取文件大小
    fn flush_alloced_list(&self) -> LinuxResult<isize> {
        let mut alloc_list = self.alloc_list.lock();
        let mut clean_list = self.clean_list.lock();
        while !alloc_list.is_empty() {
            let vaddr = alloc_list.pop_front().unwrap();
            self.flush_page(vaddr)?;
            clean_list.push_back(vaddr);
        }
        Ok(0)
    }
    
    /// 外部调用的页面置换
    fn drop_page(&self, vaddr: VirtAddr) {
        // 由于 rust 的 RAII 机制，这里会自动调用 Page 的 drop 方法，将脏页写回内存
        if check_dirty(vaddr) {
            let mut pages = self.pages.write();
            pages.remove(&vaddr);
        }
    }
}

const DEFAULT_PAGE_POOL_SIZE: usize = 100;
lazy_static! {
    static ref PAGE_POOL: Arc<PagePool> = Arc::new(
        PagePool::new(DEFAULT_PAGE_POOL_SIZE)
    );
}

fn page_pool() -> Arc<PagePool> {
    PAGE_POOL.clone()
}

pub struct PageCache {
    file: Weak<Mutex<axfs::fops::File>>,
    offset: RwLock<usize>,  // 文件读写的位置指针
    stat: RwLock<Kstat>,    // 文件属性

    page_pool: Arc<PagePool>,
    pages: RwLock<BTreeMap<usize, VirtAddr>>,

    // 这个 dirty_pages 只管理 write 产生的脏页，无法维护 mmap 后通过地址映射修改的脏页
    dirty_pages: RwLock<BTreeSet<usize>>,
}

impl PageCache {
    pub fn new(file: Weak<Mutex<axfs::fops::File>>) -> Self {        
        // stat 的初始化照抄 api/src/file/fs.rs
        let (offset, stat) = {
            let file = file.upgrade().expect("File has been dropped before PageCache creation");
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
            file: file.clone(),
            offset: RwLock::new(offset),
            stat: RwLock::new(stat),
            page_pool: page_pool(),
            pages: RwLock::new(BTreeMap::new()),
            dirty_pages: RwLock::new(BTreeSet::new()),
        }
    }

    pub fn get_vaddr(&self, offset: usize) -> VirtAddr {
        let aligned_offset = memory_addr::align_down_4k(offset);
        let page_id = aligned_offset / PAGE_SIZE_4K;
        {
            let mut pages = self.pages.write();
            if let Some(vaddr) = pages.get(&page_id) {
                return VirtAddr::from_usize(offset - aligned_offset + vaddr.as_usize());
            }
        }
        let append = {
            let stat = self.stat.read();
            let size = stat.size as usize;
            if size == 0 {
                true
            } else {
                // 当前页号 > 文件最大页号
                let max_page_id = (size - 1) / PAGE_SIZE_4K;
                page_id > max_page_id
            }
        };
        let append = false;
        let vaddr = self.page_pool.alloc_page(self.file.clone(), page_id, append);
        {
            let mut pages = self.pages.write();
            pages.insert(page_id, vaddr);
        }
        VirtAddr::from_usize(offset - aligned_offset + vaddr.as_usize())
    }

    fn read_slice_from_page(&self, page_id: usize, page_start: usize, buf: &mut [u8]) {
        let vaddr = self.get_vaddr(page_id * PAGE_SIZE_4K);
        self.page_pool.read_page(vaddr, page_start, buf);
    }

    fn write_slice_into_page(&self, page_id: usize, page_start: usize, buf: &[u8]) -> LinuxResult<isize> {
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.insert(page_id);
        }
        // TODO 问题出在这里，但不理解为什么会这样
        let vaddr = self.get_vaddr(page_id * PAGE_SIZE_4K);
        let len = self.page_pool.write_page(vaddr, page_start, buf)?;
        Ok(len as isize)
    }
    
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> LinuxResult<usize> {
        debug!("PageCache::read_at: offset = {}, buf.len() = {}", offset, buf.len());

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
            self.read_slice_from_page(page_id, page_offset_start, slice);
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
        debug!("PageCache::write_at: offset = {}, buf.len() = {}", offset, buf.len());
        // 把间隙填 0
        {
            let orig_size = { self.stat.read().size } as usize;
            if offset > orig_size {
                error!("offset = {}, size = {}", offset, orig_size);
                let gap_length = (offset - orig_size) as usize;
                let buf = vec![0 as u8; gap_length];
                self.write_at(orig_size as usize, &buf).expect("Failed to write gap");
            }
        }
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
            let len = self.write_slice_into_page(page_id, page_offset_start, slice).expect("写入失败");
            ret += len as usize;
        }

        // 更新 stat
        let mut stat = self.stat.write();
        if offset + ret > stat.size as usize {
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
            *offset_lock += bytes_written;
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
        // gap 补齐操作推迟到 write_at 执行
        Ok(new_offset as isize)
    }

    fn flush_page(&self, page_id: usize) -> LinuxResult<isize> {        
        let vaddr = self.get_vaddr(page_id * PAGE_SIZE_4K);
        self.page_pool.flush_page(vaddr)
    }

    /// 这里主要更新文件大小，同步文件指针
    fn update_metadata(&self) -> LinuxResult<isize> {
        // 先刷新所有新分配的页面（会导致文件页面数量增加）
        self.page_pool.flush_alloced_list().unwrap_or_else(|e| {
            error!("Failed to flush alloced list: {:?}", e);
            0
        });
        
        let stat = { self.stat.read() };
        let arc_file = self.file.upgrade().expect("File has been dropped before update_metadata");
        let mut file = arc_file.lock();
        
        // 更新文件大小
        if stat.size != file.get_attr()?.size() {
            debug!("File Metadata size {}, real size {}", stat.size, file.get_attr()?.size());
            assert!(stat.size as usize <= file.get_attr()?.size() as usize);
            file.truncate(stat.size).unwrap_or_else(|e| {
                error!("Failed to truncate file to size {}: {}", stat.size, e);
            });
        }
        
        // 更新文件指针
        let offset = { self.offset.read().clone() };
        let pos = SeekFrom::Start(offset as u64);
        file.seek(pos).unwrap_or_else(|e| {
            error!("Failed to seek file at offset {}: {}", offset, e);
            0
        });

        debug!("Update File Metadata size {}, offset {}", file.get_attr()?.size(), offset);
        Ok(0)
    }
    
    /// 参数 all: 
    ///     true => msync，刷新所有 page，原因是 mmap 后的页面读写无法在内核中维护脏页
    ///     => fsync，只刷新 dirty_pages 队列中的 page
    pub fn sync(&self, all: bool) -> LinuxResult<isize> {
        let page_ids : Vec<usize> = {
            if all {
                let pages = self.pages.read();
                pages.keys().copied().collect()   
            } else {
                let dirty_pages_lock = self.dirty_pages.read();
                dirty_pages_lock.iter().copied().collect()
            }
        };

        for page_id in page_ids {
            self.flush_page(page_id)
                .expect("PageCache::sync failed to flush page");
        }

        let mut dirty_pages = self.dirty_pages.write();
        dirty_pages.clear();
        drop(dirty_pages);
        
        self.update_metadata()?;
        Ok(0)
    }

    pub fn clear(&self) -> LinuxResult<isize> {
        self.sync(true)?;
        let page_addrs: Vec<VirtAddr> = {
            let pages = self.pages.read();
            pages.values().copied().collect()
        };
        for vaddr in page_addrs {
            self.page_pool.drop_page(vaddr);
        }
        self.update_metadata()?;
        let mut pages = self.pages.write();
        pages.clear();
        Ok(0)
    }
}

impl Drop for PageCache {
    fn drop(&mut self) {
        self.clear().expect("PageCache::drop failed to clear");
    }
}

/// 仅有 PageCacheManager 长久持有 Arc<PageCache>，其他地方只允许临时持有 Arc<PageCache>
/// 当 PageCache 中删掉 Arc<PageCache>，即会 RAII 析构 PageCache
/// 可以根据 fd 或者 path 找到 Arc<PageCache>
/// sys_open, sys_close, sys_stat 需要根据 path 找到 page_cache
pub struct PageCacheManager {
    path_fd: Mutex<BTreeMap<String, BTreeSet<i32>>>,
    fd_path: Mutex<BTreeMap<i32, String>>,
    path_cache: Mutex<BTreeMap<String, Arc<PageCache>>>,
}

impl PageCacheManager {
    const fn new() -> Self {
        Self { 
            path_fd: Mutex::new(BTreeMap::new()),
            fd_path: Mutex::new(BTreeMap::new()),
            path_cache: Mutex::new(BTreeMap::new()),
        }
    }
    
    // 由 sys_open 调用
    pub fn open_page_cache(&self, path: &String, file: Weak<Mutex<axfs::fops::File>>) -> Weak<PageCache> {
        let mut path_fd = self.path_fd.lock();
        let mut path_cache = self.path_cache.lock();
        let mut fd_path = self.fd_path.lock();

        if let Some(cache) = path_cache.get(path) {
            return Arc::downgrade(cache);
        }

        let cache = Arc::new(PageCache::new(file));
        path_cache.insert(path.clone(), cache.clone());
        return Arc::downgrade(&cache);
    }

    pub fn insert_fd(&self, fd: i32, path: &String) {
        let mut fd_path = self.fd_path.lock();
        let mut path_fd = self.path_fd.lock();
        let mut path_cache = self.path_cache.lock();
        
        fd_path.insert(fd, path.clone());
        if let Some(fd_set) = path_fd.get_mut(path) {
            fd_set.insert(fd);
        } else {
            path_fd.insert(path.clone(), BTreeSet::<i32>::from([fd]));
        }
    }

    // 由 sys_fstat 调用
    pub fn get_page_cache(&self, path: &String) -> Option<Weak<PageCache>> {
        let path_cache = self.path_cache.lock();
        if let Some(arc_cache) = path_cache.get(path) {
            return Some(Arc::downgrade(arc_cache));
        } else {
            return None;
        }
    }

    // 由 sys_close 调用
    pub fn try_close_page_cache(&self, fd: i32) -> LinuxResult<isize> {
        let mut path_fd = self.path_fd.lock();
        let mut path_cache = self.path_cache.lock();
        let mut fd_path = self.fd_path.lock();

        if let Some(path) = fd_path.remove(&fd) {
            let empty = {
                let fd_set = path_fd.get_mut(&path).unwrap();
                fd_set.remove(&fd);
                fd_set.is_empty()
            };
            if empty {
                path_fd.remove(&path).unwrap();
                path_cache.remove(&path).unwrap();
            }
        }
        Ok(0)
    } 

    // 辅助函数，用于查找 fd 对应的文件名
    pub fn find_path(&self, fd: i32) -> Option<String> {
        let fd_path = self.fd_path.lock();
        fd_path.get(&fd).cloned()
    }
}

lazy_static! {
    static ref PAGE_CACHE_MANAGER: Arc<PageCacheManager> = Arc::new(
        PageCacheManager::new()
    );
}

pub fn page_cache_manager() -> Arc<PageCacheManager> {
    PAGE_CACHE_MANAGER.clone()
}

