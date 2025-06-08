use axhal::{console::read_bytes, paging::MappingFlags};
use core::{error, isize, mem::ManuallyDrop, ops::DerefMut, sync::atomic::{AtomicI8, AtomicIsize}};
use axconfig::plat::PHYS_VIRT_OFFSET;
use alloc::{boxed::Box, collections::{btree_map::BTreeMap, btree_set::BTreeSet, linked_list::LinkedList}, string::String, sync::{Arc, Weak}, vec, vec::Vec};
use axhal::arch::flush_tlb;
use axio::SeekFrom;
use axprocess::Pid;
use axsync::Mutex;
use spin::{rwlock::RwLock, RwLockReadGuard, RwLockWriteGuard};
use axalloc::GlobalPage;
use axerrno::{LinuxError, LinuxResult};
use memory_addr::{PhysAddr, VirtAddr, PAGE_SIZE_4K};
use super::Kstat;
use hashbrown::hash_map::HashMap;
use axerrno::ax_err_type;
use lazy_static::lazy_static;
use axtask::{TaskExtRef, current};
use core::sync::atomic::{AtomicUsize, Ordering};
use starry_core::task::{get_process, ProcessData};

// GlobalAllocator 的性能并不高（似乎是 O(n)?），尽可能频繁避免申请释放 GlobalPage
lazy_static! {
    static ref GLOBAL_PAGE_POOL: Arc<Mutex<Vec<GlobalPage>>> = Arc::new(Mutex::new(Vec::new()));
}

/// 文件页，在创建的时候自动加载文件内容。
/// 并发不安全，需要上层加读写锁。
/// 
/// 脏页管理机制：
/// - 由 sys_write 等产生的脏页：特点是会调用 Page::write 操作，直接修改 Page 的 dirty 位即可。
/// - 由 mmap 后的内存访问产生的脏页：由于直接内存读写不会调用 Page::write，此时必须查询对应页表的
/// 页表项脏位。mmap 后访问虚拟页面触发 page fault，此时通过 Page::map_virt_page 建立页表映射，并在
/// virt_pages 里面添加对应的 pid 和 虚拟页号。查询脏位时，检查所有相关进程的页表。
struct Page {
    inner: ManuallyDrop<GlobalPage>,
    file: Weak<Mutex<axfs::fops::File>>, 
    page_id: usize,         
    dirty: bool,
    virt_pages: BTreeSet<(Pid, VirtAddr)>,
}

impl Page {
    /// 构造一个 Page。
    /// 
    /// - 优先从 GLOBAL_PAGE_POOL 里面分配 GlobalPage 作为 inner，
    /// 以尽可能避免 GLOBAL_ALLOCATOR 的分配释放
    fn new() -> Self {
        let mut inner_page =  {
            let mut global_page_pool = GLOBAL_PAGE_POOL.lock();
            global_page_pool.pop().unwrap_or_else(|| GlobalPage::alloc().expect("GlobalPage alloc failed"))
        };
        debug!("New file page: paddr {:#x}", inner_page.start_paddr(axhal::mem::virt_to_phys));
        assert!(
            // 保证该页在线性映射区
            inner_page.start_vaddr().as_usize() >= PHYS_VIRT_OFFSET,
            "Converted address is invalid, check if the virtual address is in kernel space"
        );
        inner_page.zero();

        Self {
            inner: ManuallyDrop::new(inner_page),
            file: Weak::new(),
            page_id: 0,
            dirty: false,
            virt_pages: BTreeSet::new(),
        }
    }
    
    /// 功能
    /// 1. 初始化 file 和 page_id 字段，加载文件内容到 Page。
    /// 2. 清除所有相关页表项的脏位标记。
    /// 
    /// 注意
    /// 1. 只有 Page::load 涉及 axfs 层的文件 read 操作。
    /// 2. 一个 Page 只有 load 之后才能使用；
    fn load(&mut self, file: Weak<Mutex<axfs::fops::File>>, page_id: usize) {
        self.file = file.clone();
        self.page_id = page_id;
        let file_arc = file.upgrade().unwrap();
        let mut file = file_arc.lock();
        let offset = (self.page_id * PAGE_SIZE_4K) as u64;
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

        self.set_clean();
    }

    /// 功能：
    /// 1. 将 Page 的内容刷新回文件，返回本页写回文件的大小
    /// 2. 清除所有相关页表项的脏位标记。
    /// 
    /// 注意：
    /// 1. 只有 Page::flush 涉及 axfs 层的文件 write 操作；
    /// 2. 把整个 4K 都写入磁盘，同步文件属性时上层 PagePool 会调用 truncate 修正文件大小。
    fn flush(&mut self) -> LinuxResult<isize> {        
        let file = self.file.upgrade();
        if file.is_none() {
            return Ok(0);
        }
        if !self.is_dirty() {
            return Ok(0);
        }
        
        let file_arc = file.unwrap();
        let mut file = file_arc.lock();
        let offset = (self.page_id * PAGE_SIZE_4K) as u64;

        // fatfs 调用 file.seek(SeekFrom::Start(offset as u64)) 会奇慢无比，似乎是 O(n) 的链式查找
        // ext4 可以做到 O(1) 随即寻址，性能正常
        file.seek(SeekFrom::Start(offset))?;
        let buf = self.inner.as_slice();
        let mut bytes_write = 0;
        loop {
            match file.write_at(offset + bytes_write, &buf[bytes_write as usize..]) {
                Ok(0) => break,
                Ok(n) => bytes_write += n as u64,
                Err(e) => {
                    warn!("Write failed at offset {}: {:?}", offset + bytes_write, e);
                    return Err(e.into());
                }
            }
        }

        self.set_clean();
        debug!("Flush page {} done, length = {}", self.page_id, bytes_write);
        Ok(bytes_write as isize)
    }

    /// 从内存中的缓存页面中读取数据到 buf，返回读取的长度，返回读取的长度
    fn read(&self, offset: usize, buf: &mut [u8]) -> LinuxResult<isize> {
        let start = offset;
        let end = start + buf.len();
        if end > PAGE_SIZE_4K {
            return Err(LinuxError::EINVAL);
        }
        assert!(start <= end && end <= PAGE_SIZE_4K);
        let slice = &self.inner.as_slice()[start..end];
        buf.copy_from_slice(slice);
        Ok((end - start) as isize)
    }

    /// 将 buf 中的数据写入到内存中的缓存页面，返回写入的长度，返回写入的长度
    /// 
    /// 参数 direct: 是否直接同步到文件
    fn write(&mut self, offset: usize, buf: &[u8], direct: bool) -> LinuxResult<isize> {                
        debug!("write page {}, len {}, direct {}", self.page_id, buf.len(), direct);
        let start = offset;
        let end = start + buf.len();
        if end > PAGE_SIZE_4K {
            return Err(LinuxError::EINVAL);
        }
        assert!(start < end && end <= PAGE_SIZE_4K);
        let slice = &mut self.inner.as_slice_mut()[start..end];
        slice.copy_from_slice(buf);
        self.dirty = true;
        assert!(self.is_dirty());
        if direct {
            self.flush()?;
        }
        Ok((end - start) as isize)
    }

    /// 查询本页面的实际起始物理地址
    fn get_phys_addr(&self) -> PhysAddr {
        let vaddr = self.inner.start_vaddr();
        assert!(
            // 保证该页在线性映射区
            vaddr.as_usize() >= PHYS_VIRT_OFFSET,
            "Converted address is invalid, check if the virtual address is in kernel space"
        );
        axhal::mem::virt_to_phys(vaddr)
    }

    /// 遍历所有映射到这个物理页的页表，查询本页面是否为脏页
    fn is_dirty(&self) -> bool {
        if self.dirty {
            return true;
        }
        
        let mut virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        for (pid, vaddr) in virt_pages {
            match get_process(pid) {
                Ok(process) => {
                    let process_data = process.data::<ProcessData>().unwrap();
                    let mmap_manager = process_data.process_mmap_mnager();

                    // 可能进程 munmap 取消映射，但是 Page 中没有实时更新
                    if mmap_manager.query(vaddr).is_some() {
                        let mut aspace = process_data.aspace.lock();
                        if aspace.check_page_dirty(vaddr) {
                            debug!("check dirty {}, {:#x}", self.page_id, vaddr);
                        return true;
                        }
                    }
                },
                _ => continue,
            }
        }
        return false;
    }
    
    /// 遍历所有映射到这个物理页的页表，清除本页脏位
    fn set_clean(&mut self) {
        self.dirty = false;
        
        let mut virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        for (pid, vaddr) in virt_pages {
            match get_process(pid) {
                Ok(process) => {
                    let process_data = process.data::<ProcessData>().unwrap();
                    let mmap_manager = process_data.process_mmap_mnager();
                    
                    // 可能进程 munmap 取消映射，但是 Page 中没有实时更新
                    if mmap_manager.query(vaddr).is_some() {
                        debug!("page {} set clean vadr = {:#x}", self.page_id, vaddr);
                        let mut aspace = process_data.aspace.lock();
                        aspace.set_page_dirty(vaddr, false);
                    } else {
                        self.virt_pages.remove(&(pid, vaddr));
                    }
                },
                _ => continue,
            }
        }

    }

    /// 将当前进程的一个虚拟页映射到此 Page。具体的：先尝试建立页表映射，成功的就更新 virt_pages
    /// 
    /// **注意：直接修改了页表！**
    fn map_virt_page(&mut self, aligned_virt_page: VirtAddr, access_flags: MappingFlags) -> bool {
        let curr = current();
        let pid = curr.task_ext().thread.process().pid();
        assert!(aligned_virt_page.as_usize() % PAGE_SIZE_4K == 0);
        
        let mut aspace = curr.task_ext().process_data().aspace.lock();
        let paddr = self.inner.start_paddr(axhal::mem::virt_to_phys);
        
        warn!("Try force map page {}, page id {}, {:#x} => {:#x}", current().id_name(), 
            self.page_id, aligned_virt_page, paddr);
        if aspace.force_map_page(aligned_virt_page, paddr, access_flags) {
            let mut virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
            self.virt_pages.insert((pid, aligned_virt_page));
            return true;
        }
        return false;
    }

    /// 功能：
    /// - 取消当前进程的一个虚拟页到此 Page 的映射
    /// 
    /// 注意：
    /// - 这里没有修改页表！
    /// - munmap 修改页表是在 aspace 中自动执行的
    /// - Drop Page 修改也表是在 drop 里面进行的
    fn unmap_virt_page(&mut self, aligned_virt_page: VirtAddr) {
        let mut virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        let pid = {
            let curr = current();
            curr.task_ext().thread.process().pid()
        };
        assert!(aligned_virt_page.as_usize() % PAGE_SIZE_4K == 0);
        self.virt_pages.remove(&(pid, aligned_virt_page));
        debug!("unmap virt page {} ({}, {:#x})", self.page_id, pid, aligned_virt_page);
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // 写回文件
        self.flush().unwrap_or_else(|_| {
            error!("Failed to Drop page {}", self.page_id);
            0
        });
        
        // 取消所有映射到该物理页的映射。
        // 注意：这里直接修改了页表！
        let virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        for (pid, vaddr) in virt_pages {
            match get_process(pid) {
                Ok(process) => {
                    let process_data = process.data::<ProcessData>().unwrap();
                    let mmap_manager = process_data.process_mmap_mnager();

                    if let Some(_) = mmap_manager.query(vaddr) {
                        let mut aspace = process_data.aspace.lock();
                        error!("force unmap {:#x}", vaddr);
                        aspace.force_unmap_page(vaddr);
                    }
                },
                _ => continue,
            }
        }

        let mut global_page_pool = GLOBAL_PAGE_POOL.lock();
        let inner = unsafe {
            ManuallyDrop::take(&mut self.inner)
        };
        global_page_pool.push(inner);
    }
}

type PageKey = isize;
type PagePtr = Arc<RwLock<Page>>;

/// 并发安全的文件页池
struct PagePool {
    // 唯一的数据结构存储 Page，只要在 pages 中删掉 PageKey，就会 Drop 该 Page
    pages: RwLock<BTreeMap::<PageKey, PagePtr>>,
    // 允许同时持有的最多 Page 数量
    max_size: AtomicUsize,
    // 用于分配 page_key
    page_key_cnt: AtomicIsize,  
    // 没有被修改过的页面
    clean_list: Mutex<LinkedList<PageKey>>,
    // 修改过的页面
    dirty_list: Mutex<LinkedList<PageKey>>,
}

impl PagePool {
    fn new(max_size: usize) -> Self {
        Self {
            pages: RwLock::new(BTreeMap::new()),
            max_size: AtomicUsize::new(max_size),
            page_key_cnt: AtomicIsize::new(0),
            clean_list: Mutex::new(LinkedList::new()),
            dirty_list: Mutex::new(LinkedList::new()),
        }
    }
    
    /// 功能：
    /// - 调整 PagePool 的大小
    /// 
    /// 暂时没用到，给以后留作扩展，例如根据内存使用情况，动态调整
    fn resize(&self, size: usize) {
        self.max_size.store(size, Ordering::SeqCst);
        let mut pages = self.pages.write();
        while pages.len() >= self.max_size.load(Ordering::SeqCst) {
            self.auto_drop_page(&mut pages);
        }
        let mut global_page_pool = GLOBAL_PAGE_POOL.lock();
        while pages.len() + global_page_pool.len() > size {
            global_page_pool.pop();
        }
    }

    /// 功能：
    /// - 根据 page_key 尝试获取页面的 Arc 指针；
    /// - 如果 page_key 查找命中，则返回的 PageKey 即使传入的 page_key；
    /// - 如果 page_key 查找失败，说明该页已经被 Drop，那么重新分配一个页面，
    /// 返回的 PageKey != 传入的 page_key，这时上层调用者就需要初始化这个 page。
    fn acquire_page(&self, page_key: PageKey) -> (PagePtr, PageKey) {
        let pages = self.pages.read();
        if let Some(page) = pages.get(&page_key) {
            return (page.clone(), page_key);
        }
        drop(pages);

        let mut pages = self.pages.write();
        while pages.len() >= self.max_size.load(Ordering::SeqCst) {
            self.auto_drop_page(&mut pages);
        }
        let page = Arc::new(RwLock::new(Page::new()));
        let key = self.page_key_cnt.fetch_add(1, Ordering::SeqCst);
        pages.insert(key.clone(), page.clone());
        let mut clean_list = self.clean_list.lock();
        clean_list.push_back(key);
        (page, key)
    }

    /// 功能：
    /// - 根据 page_key 判断该页是否仍在 PagePool 中
    fn check_page_exist(&self, page_key: &PageKey) -> bool {
        let pages = self.pages.read();
        pages.contains_key(page_key)
    }

    /// 功能：
    /// - 自动挑选一个 Page，Drop 它
    fn auto_drop_page(&self, pages: &mut RwLockWriteGuard<BTreeMap<PageKey, Arc<RwLock<Page>>>>) {        
        // 选择需要被丢弃的页面
        let key = {
            let mut clean_list = self.clean_list.lock();
            let mut dirty_list = self.dirty_list.lock();
            // error!("clean: {}, dirty: {}, total: {}", clean_list.len(), dirty_list.len(), pages.len());
            // assert!(clean_list.len() + dirty_list.len() == pages.len());
            
            let mut drop_key: Option<isize> = None;
            while !clean_list.is_empty() {
                let key = clean_list.pop_front().unwrap();
                let _page = pages.get(&key);
                if _page.is_none() {
                    continue;
                }
                let arc_page = _page.unwrap();
                let page = arc_page.read();
                if page.is_dirty() {
                    dirty_list.push_back(key);
                } else {
                    drop_key = Some(key);
                    break;
                }
            }
            if drop_key.is_some() {
                drop_key.unwrap()
            } else {
                dirty_list.pop_front().unwrap() as isize
            } 
        };

        pages.remove(&key);
        debug!("Auto drop page:  {:#x}", key);
    }
 
    /// 将一个页面刷新回文件并 Drop
    /// 1. 外部调用的页面置换，由于 rust 的 RAII 机制，这里会自动调用 Page 的 drop 方法，将脏页写回内存。
    /// 2. 没有修改干净页/脏页队列
    fn drop_page(&self, page_key: PageKey) {
        let mut pages = self.pages.write();
        pages.remove(&page_key);
    }
}


/// 全局的 PagePool，但是可以扩展成灵活地给 PageCache 绑定 PagePool
const DEFAULT_PAGE_POOL_SIZE: usize = 10;
lazy_static! {
    static ref PAGE_POOL: Arc<PagePool> = Arc::new(
        PagePool::new(DEFAULT_PAGE_POOL_SIZE)
    );
}

/// 用于维护一个文件的页缓存，接管文件的基本操作
pub struct PageCache {
    // axfs 层的 File 指针
    file: Weak<Mutex<axfs::fops::File>>,
    // 文件读写的位置指针
    offset: AtomicUsize,  
    // 文件大小
    size: AtomicUsize,
    // 使用的 PagePool
    page_pool: Arc<PagePool>,
    // 维护页面 -> PageKey 的缓存
    pages: RwLock<BTreeMap<usize, PageKey>>,
    // 这个 dirty_pages 只管理 write 产生的脏页，无法维护 mmap 后通过地址映射修改的脏页
    dirty_pages: RwLock<BTreeSet<usize>>,
}

impl PageCache {
    pub fn new(file: Weak<Mutex<axfs::fops::File>>) -> Self {        
        let (offset, size) = {
            let file = file.upgrade().expect("File has been dropped before PageCache creation");
            let file_lock = file.lock(); // 获取文件锁
            let offset = file_lock.get_offset() as usize;
            let size = file_lock.get_attr().unwrap().size();
            (offset, size)
        };

        Self {
            file: file.clone(),
            offset: AtomicUsize::new(offset),
            size: AtomicUsize::new(size as usize),
            page_pool: PAGE_POOL.clone(),
            pages: RwLock::new(BTreeMap::new()),
            dirty_pages: RwLock::new(BTreeSet::new()),
        }
    }

    /// 功能：
    /// - 根据 page_id，获得 Page 的 Arc 指针
    /// - 如果 PagePool 此时不存在，则新建一个 Page 并初始化
    fn acquire_page(&self, page_id: usize) -> PagePtr {
        let mut pages = self.pages.write();
        let orig_key = pages.get(&page_id).cloned().unwrap_or(-1);
        
        let (page, new_key) = self.page_pool.acquire_page(orig_key);
        if orig_key != new_key {
            let mut page = page.write();
            page.load(self.file.clone(), page_id);
            pages.remove(&page_id);
            pages.insert(page_id, new_key);
        }
        
        page
    }

    /// 一种对页面操作的抽象：检查页号为 page_id 的页是否存在于 PagePool，
    /// 若存在则执行操作 f
    fn _with_valid_page<T, F>(&self, page_id: usize, f: F) -> bool
    where
        F: FnOnce(&mut Page) -> T,
    {
        {
            let pages = self.pages.read();
            if !pages.contains_key(&page_id) {
                return false;
            }
            let page_key = pages.get(&page_id).unwrap();
            if !self.page_pool.check_page_exist(page_key) {
                return false;
            }
        }
        
        let rwlock = self.acquire_page(page_id);
        let mut page = rwlock.write();
        f(page.deref_mut());
        true
    }

    /// 将当前进程的 aligned_vaddr 映射到文件的 page_id 页面
    /// 
    /// 注意：直接修改了页表！ 
    pub fn map_virt_page(&self, page_id: usize, aligned_vaddr: VirtAddr, access_flags: MappingFlags) -> bool {
        let rwlock = self.acquire_page(page_id);
        let mut page = rwlock.write();
        page.map_virt_page(aligned_vaddr, access_flags)
    }

    /// 取消当前进程的 aligned_vaddr 到文件的 page_id 页面的映射
    pub fn unmap_virt_page(&self, page_id: usize, aligned_vaddr: VirtAddr) {
        self._with_valid_page(page_id, |page| {
            page.unmap_virt_page(aligned_vaddr);
        });
    }

    /// 将缓存页 page_id 同步到文件
    pub fn flush_page(&self, page_id: usize) -> LinuxResult<isize> {
        self._with_valid_page(page_id, |page| page.flush());
        Ok(0)
    }

    /// 读取页面 page_id，从 page_start 起的内容到 buf
    fn read_slice_from_page(&self, page_id: usize, page_start: usize, buf: &mut [u8]) -> LinuxResult<isize> {
        let rwlock = self.acquire_page(page_id);
        let page = rwlock.read();
        page.read(page_start, buf)
    }

    /// 将 buf 的内容写到页面 page_id，从 page_start 起的位置
    fn write_slice_into_page(&self, page_id: usize, page_start: usize, buf: &[u8]) -> LinuxResult<isize> {
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.insert(page_id);
        }
        let direct = {
            let size = self.size.load(Ordering::SeqCst);
            // 如果当前文件为空，或者 page_id 大于已存在的最大页号
            size == 0 || page_id > (size - 1) / PAGE_SIZE_4K
        };
        let rwlock = self.acquire_page(page_id);
        let mut page = rwlock.write();
        page.write(page_start, buf, direct)
    }

    /// 功能：
    /// - 将文件末尾到 offset 之间的内容全部填成 0，并更新文件大小为 offset
    fn fill_gap(&self, offset: usize) {
        let orig_size = self.size.load(Ordering::SeqCst);
        if offset <= orig_size {
            return;
        }

        let buf = vec![0 as u8; PAGE_SIZE_4K];
        let start_offset = orig_size;
        let start_page_id = start_offset / PAGE_SIZE_4K;
        let end_page_id = offset / PAGE_SIZE_4K;

        for page_id in start_page_id..end_page_id {
            self.write_slice_into_page(page_id, 0, &buf).expect("Failed to write gap");
        }

        self.size.store(offset, Ordering::SeqCst);
    }

    /// 功能：
    /// - 并发安全的读取给定位置的内容，不会改变文件指针；
    /// - 适用于 sys_pread。
    /// 
    /// 返回：
    /// - 读取到的长度
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> LinuxResult<usize> {
        let file_len = self.size.load(Ordering::SeqCst);
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

    /// 功能：
    /// - 并发安全地写入给定位置，不会改变文件指针，但会更新文件大小；
    /// - 适用于 sys_pwrite。
    /// 
    /// 返回：
    /// - 成功写入的长度
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> LinuxResult<usize> {
        // 把间隙填 0
        self.fill_gap(offset);

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

        self.size.fetch_max(offset + ret, Ordering::SeqCst);
        Ok(ret)
    }

    /// 顺序读取，并发不安全，会改变文件指针和文件大小。适用于 sys_read。
    pub fn read(&self, buf: &mut [u8]) -> LinuxResult<usize> {
        let offset = self.offset.load(Ordering::SeqCst);
        let result = self.read_at(offset, buf);
        if let Ok(bytes_read) = result {
            self.offset.fetch_add(bytes_read, Ordering::SeqCst);
        }
        result
    }

    /// 顺序写入，并发不安全，会改变文件指针和文件大小。适用于 sys_write。
    pub fn write(&self, buf: &[u8]) -> LinuxResult<usize> {
        let offset = self.offset.load(Ordering::SeqCst);
        let result = self.write_at(offset, buf);
        if let Ok(bytes_written) = result {
            self.offset.fetch_add(bytes_written, Ordering::SeqCst);
        }
        result
    }

    /// 获取文件属性。只有 size 是从 page cache 里面维护，其他都是从 axfs 层读取地。
    pub fn stat(&self) -> LinuxResult<Kstat> {
        let file = self.file.upgrade().unwrap();
        let file = file.lock();
        let metadata = file.get_attr()?;
        let ty = metadata.file_type() as u8;
        let perm = metadata.perm().bits() as u32;

        return Ok(Kstat {
            mode: ((ty as u32) << 12) | perm,
            size: self.size.load(Ordering::SeqCst) as u64,
            blocks: metadata.blocks(),
            blksize: 512,
            ..Default::default()
        });
    }

    /// 移动文件指针
    pub fn seek(&self, pos: SeekFrom) -> LinuxResult<isize> {
        let mut size = self.size.load(Ordering::SeqCst) as u64;
        let mut offset = self.offset.load(Ordering::SeqCst) as u64;
        let new_offset = match pos {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(off) => offset.checked_add_signed(off),
            SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .ok_or_else(|| ax_err_type!(InvalidInput))?;
    
        self.offset.store(new_offset as usize, Ordering::SeqCst);
        Ok(new_offset as isize)
    }

    /// 这里主要更新文件大小，同步文件指针
    fn update_metadata(&self) -> LinuxResult<isize> {
        let file_lock = self.file.upgrade().unwrap();
        let mut file = file_lock.lock();
        
        // // 更新文件大小
        // let size = self.size.load(Ordering::SeqCst);
        // if size != file.get_attr()?.size() as usize {
        //     assert!(size <= file.get_attr()?.size() as usize);
        //     file.truncate(size as u64).unwrap_or_else(|e| {
        //         error!("Failed to truncate file to size {}: {}", size, e);
        //     });
        // }
        
        // 更新文件指针
        let offset = self.offset.load(Ordering::SeqCst);
        let pos = SeekFrom::Start(offset as u64);
        file.seek(pos).unwrap_or_else(|e| {
            error!("Failed to seek file at offset {}: {}", offset, e);
            0
        });

        // error!("Update File Metadata size {}, offset {}", file.get_attr()?.size(), offset);
        Ok(0)
    }
    
    /// 功能：
    /// - 同步缓存到文件。
    /// 
    /// 参数: 
    /// - all == true => msync，刷新所有 page，原因是 mmap 后的页面读写无法在内核中维护脏页；
    /// - all == false => fsync，只刷新 dirty_pages 队列中的 page。
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

    /// 在 PagePool 中清除关于这个文件的所有页面，并同步信息。
    pub fn clear(&self) -> LinuxResult<isize> {
        let page_keys: Vec<PageKey> = {
            let pages = self.pages.read();
            pages.values().copied().collect()
        };
        for key in page_keys {
            self.page_pool.drop_page(key);
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
        let mut path_cache = self.path_cache.lock();

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
        info!("page cache manager close fd {}", fd);

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

    // 辅助函数，根据 fd 查询对应的 page cache
    pub fn fd_cache(&self, fd: i32) -> Arc<PageCache> {
        let fd_path = self.fd_path.lock();
        let path = fd_path.get(&fd).unwrap();
        let path_cache = self.path_cache.lock();
        path_cache.get(path).unwrap().clone()
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

