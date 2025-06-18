use super::Kstat;
use crate::file::{File, FileLike};
use alloc::{
    collections::{btree_map::BTreeMap, btree_set::BTreeSet, linked_list::LinkedList},
    string::String,
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};
use axalloc::GlobalPage;
use axconfig::plat::PHYS_VIRT_OFFSET;
use axerrno::{LinuxError, LinuxResult};
use axfs::fops::OpenOptions;
use axhal::{arch::flush_tlb, paging::MappingFlags};
use axio::SeekFrom;
use axprocess::Pid;
use axsync::Mutex;
use axtask::{TaskExtRef, current};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{isize, mem::ManuallyDrop, ops::DerefMut, sync::atomic::AtomicIsize};
use lazy_static::lazy_static;
use memory_addr::{PAGE_SIZE_4K, PhysAddr, VirtAddr};
use spin::{RwLockWriteGuard, rwlock::RwLock};
use starry_core::task::{ProcessData, get_process};

// GlobalAllocator 的性能并不高（似乎是 O(n)?），尽可能频繁避免申请释放 GlobalPage
lazy_static! {
    static ref GLOBAL_PAGE_POOL: Arc<Mutex<Vec<GlobalPage>>> = Arc::new(Mutex::new(Vec::new()));
}

/// 文件页，在创建的时候自动加载文件内容。
/// 并发不安全，需要上层加读写锁。
///
/// 脏页管理机制：
/// - 由 sys_write 等产生的脏页：特点是会调用 Page::write 操作，直接修改 Page 的 dirty 成员。
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
        let mut inner_page = {
            let mut global_page_pool = GLOBAL_PAGE_POOL.lock();
            global_page_pool
                .pop()
                .unwrap_or_else(|| GlobalPage::alloc().expect("GlobalPage alloc failed"))
        };
        debug!(
            "New file page: paddr {:#x}",
            inner_page.start_paddr(axhal::mem::virt_to_phys)
        );
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

    /// 初始化 file 和 page_id 字段，加载文件内容，清空脏页标记。
    ///
    /// 注意：
    /// - 只有 Page::load 涉及 axfs 层的文件 read 操作。
    fn load(&mut self, file: Weak<Mutex<axfs::fops::File>>, page_id: usize) {
        debug!("ready load page {}", self.page_id);
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
        debug!(
            "Load page {} done, length = {}, {:?}..",
            self.page_id,
            bytes_read,
            &buf[0..5]
        );
        self.set_clean();
    }

    /// 将 Page 的内容刷新回文件，清空脏页标记，返回本页写回文件的大小。
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

        debug!(
            "Flush page {} done, length = {}, {:?}..",
            self.page_id,
            bytes_write,
            &buf[0..5]
        );
        self.set_clean();
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
    /// 参数 direct: 是否直接同步到文件，主要用于 PageCache::fill_gap
    fn write(&mut self, offset: usize, buf: &[u8], direct: bool) -> LinuxResult<isize> {
        debug!(
            "write page {}, len {}, direct {}",
            self.page_id,
            buf.len(),
            direct
        );
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

        let virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        for (pid, vaddr) in virt_pages {
            match get_process(pid) {
                Ok(process) => {
                    let process_data = process.data::<ProcessData>().unwrap();
                    let mmap_manager = process_data.vma_mnager();

                    // 可能进程 munmap 取消映射，但是 Page 中没有实时更新
                    if mmap_manager.query(vaddr).is_some() {
                        let mut aspace = process_data.aspace.lock();
                        if aspace.check_page_dirty(vaddr) {
                            debug!("check dirty {}, {:#x}", self.page_id, vaddr);
                            return true;
                        }
                    }
                }
                _ => continue,
            }
        }
        return false;
    }

    /// 遍历所有映射到这个物理页的页表，清除本页脏位
    fn set_clean(&mut self) {
        self.dirty = false;

        let virt_pages: Vec<_> = self.virt_pages.iter().copied().collect();
        for (pid, vaddr) in virt_pages {
            match get_process(pid) {
                Ok(process) => {
                    let process_data = process.data::<ProcessData>().unwrap();
                    let mmap_manager = process_data.vma_mnager();

                    // 可能进程 munmap 取消映射，但是 Page 中没有实时更新
                    if mmap_manager.query(vaddr).is_some() {
                        debug!("page {} set clean vadr = {:#x}", self.page_id, vaddr);
                        let mut aspace = process_data.aspace.lock();
                        aspace.set_page_dirty(vaddr, false);
                    } else {
                        self.virt_pages.remove(&(pid, vaddr));
                    }
                }
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

        debug!(
            "Try force map page {}, {:#x} => {:#x}, {}",
            self.page_id,
            aligned_virt_page,
            paddr,
            current().id_name()
        );

        // 这里先 flush，再 set_clean 的目的是：单纯修改页表也会导致页面被标记为脏页。
        match self.flush() {
            Ok(_) => (),
            _ => return false,
        }
        if aspace.force_map_page(aligned_virt_page, paddr, access_flags) {
            self.virt_pages.insert((pid, aligned_virt_page));
            drop(aspace);
            self.set_clean();
            return true;
        }
        return false;
    }

    /// 功能：
    /// - 取消当前进程的一个虚拟页到此 Page 的映射，修改页表
    ///
    /// 注意：
    /// - 这里修改了页表，在 aspace 视角，这个页面始终是一个未分配的 unpopulate 页面
    /// - Drop Page 修改也表是在 drop 里面进行的
    fn unmap_virt_page(&mut self, aligned_virt_page: VirtAddr) {
        let curr = current();
        let pid = curr.task_ext().thread.process().pid();
        assert!(aligned_virt_page.as_usize() % PAGE_SIZE_4K == 0);

        // 理论上不应该出现这种情况
        if !self.virt_pages.remove(&(pid, aligned_virt_page)) {
            return;
        }

        debug!(
            "unmap virt page {} ({}, {:#x})",
            self.page_id, pid, aligned_virt_page
        );
        
        let mut aspace = curr.task_ext().process_data().aspace.lock();
        aspace.force_unmap_page(aligned_virt_page);
        flush_tlb(Some(aligned_virt_page));
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
                    let mmap_manager = process_data.vma_mnager();

                    if let Some(_) = mmap_manager.query(vaddr) {
                        let mut aspace = process_data.aspace.lock();
                        debug!("force unmap {:#x}", vaddr);
                        aspace.force_unmap_page(vaddr);
                    }
                }
                _ => continue,
            }
        }

        let mut global_page_pool = GLOBAL_PAGE_POOL.lock();
        let inner = unsafe { ManuallyDrop::take(&mut self.inner) };
        global_page_pool.push(inner);
    }
}

type PageKey = isize;
type PagePtr = Arc<RwLock<Page>>;

/// 并发安全的文件页池
struct PagePool {
    // 唯一的数据结构存储 Page，只要在 pages 中删掉 PageKey，就会 Drop 该 Page
    pages: RwLock<BTreeMap<PageKey, PagePtr>>,
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

    /// 调整 PagePool 的大小。
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

    /// 根据 page_key 判断该页是否仍在 PagePool 中
    fn check_page_exist(&self, page_key: &PageKey) -> bool {
        let pages = self.pages.read();
        pages.contains_key(page_key)
    }

    /// 自动挑选一个 Page，Drop 它
    fn auto_drop_page(&self, pages: &mut RwLockWriteGuard<BTreeMap<PageKey, Arc<RwLock<Page>>>>) {
        // 选择需要被丢弃的页面
        let key = {
            let mut clean_list = self.clean_list.lock();
            let mut dirty_list = self.dirty_list.lock();

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
    static ref PAGE_POOL: Arc<PagePool> = Arc::new(PagePool::new(DEFAULT_PAGE_POOL_SIZE));
}

/// 用于维护一个文件的页缓存，接管文件的基本操作
pub struct PageCache {
    // 对接的文件名
    path: String,
    // axfs 层的 File 指针
    file: Arc<Mutex<axfs::fops::File>>,
    // 文件大小
    size: AtomicUsize,
    // 已占用的最大 page id
    max_page_id: AtomicIsize,
    // fill gap 预留的 最大 page od
    reserved_page_id: AtomicIsize,
    // 使用的 PagePool
    page_pool: Arc<PagePool>,
    // 维护页面 -> PageKey 的缓存
    pages: RwLock<BTreeMap<usize, PageKey>>,
    // 这个 dirty_pages 只管理 write 产生的脏页，无法维护 mmap 后通过地址映射修改的脏页
    dirty_pages: RwLock<BTreeSet<usize>>,
}

impl PageCache {
    pub fn new(path: String) -> Self {
        warn!("Create Pagecache {}", path);
        let opts = OpenOptions::new().set_read(true).set_write(true);
        let file = axfs::fops::File::open(path.as_str(), &opts).unwrap();
        let file = Arc::new(Mutex::new(file));
        let size = {
            let file = file.lock();
            file.get_attr().unwrap().size()
        };

        let max_page_id = {
            if size == 0 {
                -1 as isize
            } else {
                (size as isize - 1) / PAGE_SIZE_4K as isize
            }
        };

        Self {
            path: path,
            file: file.clone(),
            size: AtomicUsize::new(size as usize),
            max_page_id: AtomicIsize::new(max_page_id),
            reserved_page_id: AtomicIsize::new(max_page_id),
            page_pool: PAGE_POOL.clone(),
            pages: RwLock::new(BTreeMap::new()),
            dirty_pages: RwLock::new(BTreeSet::new()),
        }
    }

    /// 将当前进程的 aligned_vaddr 映射到文件的 page_id 页面
    ///
    /// 注意：直接修改了页表！
    pub fn map_virt_page(
        &self,
        page_id: usize,
        aligned_vaddr: VirtAddr,
        access_flags: MappingFlags,
    ) -> bool {
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

    /// 并发安全的读取给定位置，返回读取到的长度
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let file_len = self.size.load(Ordering::SeqCst);
        let read_len = buf.len().min(file_len - offset);

        if offset >= file_len || read_len == 0 {
            return 0;
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
            self.read_slice_from_page(page_id, page_offset_start, slice)
                .expect("PageCache read failed");
            ret += cur_len;
        }
        ret
    }

    /// 并发安全地写入给定位置，更新文件大小，返回写入长度
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
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
            let len = self
                .write_slice_into_page(page_id, page_offset_start, slice, false)
                .expect("PageCache writea_at failed");
            ret += len as usize;
            self.size.fetch_max(offset + ret, Ordering::SeqCst);
        }
        ret
    }

    pub fn truncate(&self, offset: usize) -> usize {
        if self.size.fetch_min(offset, Ordering::SeqCst) == offset {
            return offset;
        }
        self.fill_gap(offset);
        return offset;
    }

    /// 获取文件属性。只有 size 是从 page cache 里面维护，其他都是从 axfs 层读取地。
    pub fn stat(&self) -> LinuxResult<Kstat> {
        let file = self.file.lock();
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

    /// 获取文件大小
    pub fn get_file_size(&self) -> usize {
        self.size.load(Ordering::SeqCst)
    }

    /// 同步内容修改到文件。
    ///
    /// 注意：mmap 后通过内存读写修改文件不经过 page cache，所以不会在 dirty_pages 中有记录。
    /// 但是 munmap 时会遍历所有页面并flush，保证了数据落到文件。
    pub fn sync(&self) -> LinuxResult<usize> {
        let page_ids: Vec<usize> = {
            let dirty_pages_lock = self.dirty_pages.read();
            dirty_pages_lock.iter().copied().collect()
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

    /// 根据 page_id，获得 Page 的 Arc 指针。
    /// 如果 PagePool 此时不存在，则新建一个 Page 并初始化
    fn acquire_page(&self, page_id: usize) -> PagePtr {
        let mut pages = self.pages.write();
        let orig_key = pages.get(&page_id).cloned().unwrap_or(-1);

        let (page, new_key) = self.page_pool.acquire_page(orig_key);
        if orig_key != new_key {
            let mut page = page.write();
            page.load(Arc::downgrade(&self.file), page_id);
            pages.remove(&page_id);
            pages.insert(page_id, new_key);
        }
        page
    }

    /// 检查页号为 page_id 的页是否存在于 PagePool，若存在则执行操作 f
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

    /// 读取页面 page_id，从 page_start 起的内容到 buf
    fn read_slice_from_page(
        &self,
        page_id: usize,
        page_start: usize,
        buf: &mut [u8],
    ) -> LinuxResult<isize> {
        let rwlock = self.acquire_page(page_id);
        let page = rwlock.read();
        page.read(page_start, buf)
    }

    /// 将 buf 的内容写到页面 page_id，从 page_start 起的位置
    ///
    /// fill_gap 参数含义：表明这次写入是为了扩展文件大小，写入内容全为 0。
    /// 要求立即刷回文件(direct)，并且如果文件已经是脏页（被其他进程写入了）则取消写入。
    fn write_slice_into_page(
        &self,
        page_id: usize,
        page_start: usize,
        buf: &[u8],
        fill_gap: bool,
    ) -> LinuxResult<isize> {
        {
            let mut dirty_pages = self.dirty_pages.write();
            dirty_pages.insert(page_id);
        }

        let rwlock = self.acquire_page(page_id);
        let mut page = rwlock.write();

        // 避免一种情况：文件初始为空，进程 A 需要在第8页写入，进程 B 需要在第 4页写入，
        // 而第 4 页再进程 B 写入之后又被进程 A 清空
        if fill_gap && self.size.load(Ordering::SeqCst) > page_id * PAGE_SIZE_4K {
            return Ok(0);
        }

        page.write(page_start, buf, fill_gap)
    }

    /// 处理新建的页，全部初始化为 0
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
            self.write_slice_into_page(page_id, 0, &buf, true)
                .expect("Failed to write gap");
            self.size
                .fetch_max(offset.min((page_id + 1) * PAGE_SIZE_4K), Ordering::SeqCst);
        }
    }

    /// 这里主要更新文件大小，以后可以扩展同步更多信息，比如修改
    fn update_metadata(&self) -> LinuxResult<isize> {
        let file = self.file.lock();

        // 更新文件大小
        let size = self.size.load(Ordering::SeqCst);
        if size != file.get_attr()?.size() as usize {
            assert!(size <= file.get_attr()?.size() as usize);
            file.truncate(size as u64).unwrap_or_else(|e| {
                error!("Failed to truncate file to size {}: {}", size, e);
            });
        }
        info!("Update File Metadata size {}", file.get_attr()?.size());
        Ok(0)
    }

    /// 在 PagePool 中清除关于这个文件的所有页面，并同步信息。
    fn clear(&self) -> LinuxResult<isize> {
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
        warn!("Drop page cache {}", self.path);
        self.clear().expect("PageCache::drop failed to clear");
    }
}

/// 仅有 PageCacheManager 长久持有 Arc<PageCache>，其他地方只允许临时持有 Arc<PageCache>
/// 当 PageCache 中删掉 Arc<PageCache>，即会 RAII 析构 PageCache
pub struct PageCacheManager {
    path_cache: Mutex<BTreeMap<String, Arc<PageCache>>>,
    path_cnt: Mutex<BTreeMap<String, i32>>,
}

impl PageCacheManager {
    const fn new() -> Self {
        Self {
            path_cache: Mutex::new(BTreeMap::new()),
            path_cnt: Mutex::new(BTreeMap::new()),
        }
    }

    //  由 File 的构造函数调用，获得 page cache 的指针并增加 path 的打开计数
    pub fn open_page_cache(&self, path: &String) -> Weak<PageCache> {
        let mut path_cnt = self.path_cnt.lock();
        let mut path_cache = self.path_cache.lock();

        if let Some(cache) = path_cache.get(path) {
            let cnt = path_cnt.get_mut(path).unwrap();
            *cnt += 1;
            return Arc::downgrade(cache);
        }

        info!("Create a new page cache for {}", path);
        assert!(!path_cnt.contains_key(path));
        path_cnt.insert(path.clone(), 1);
        let cache = Arc::new(PageCache::new(path.clone()));
        path_cache.insert(path.clone(), cache.clone());
        assert!(path_cache.contains_key(path));
        return Arc::downgrade(&cache);
    }

    /// 由 File 的析构函数调用，减少 path 的引用计数
    pub fn close_page_cache(&self, path: &String) {
        let mut path_cnt = self.path_cnt.lock();
        let mut path_cache = self.path_cache.lock();
        let cnt = path_cnt.get_mut(path).unwrap();
        *cnt -= 1;

        if *cnt == 0 as i32 {
            info!("Close page cache for {}", path);
            path_cache.remove(path).unwrap();
            path_cnt.remove(path).unwrap();
        }
    }

    pub fn populate(&self, fd: i32, offset: usize, length: usize) {
        let cache = self.fd_cache(fd);
        let start_id = offset / PAGE_SIZE_4K;
        let end_id = (offset + length) / PAGE_SIZE_4K;
        for id in start_id..end_id {
            let _ = cache.acquire_page(id);
        }
    }

    pub fn munmap(&self, fd: i32, offset: usize, length: usize, vaddr: usize) {
        let cache = self.fd_cache(fd);
        let start_id = offset / PAGE_SIZE_4K;
        let end_id = (offset + length) / PAGE_SIZE_4K;
        for id in start_id..end_id {
            let vaddr = VirtAddr::from(vaddr + (id - start_id) * PAGE_SIZE_4K);
            cache.unmap_virt_page(id, vaddr);
        }
    }

    pub fn msync(&self, fd: i32, offset: usize, length: usize) {
        let cache = self.fd_cache(fd);
        let start_id = offset / PAGE_SIZE_4K;
        let end_id = (offset + length) / PAGE_SIZE_4K;
        for id in start_id..end_id {
            cache.flush_page(id).expect("msync flush page failed");
        }
    }

    // 主要用于 mmap 相关函数，需要根据 fd 找到对应的 page cache
    pub fn fd_cache(&self, fd: i32) -> Arc<PageCache> {
        let file = File::from_fd(fd).unwrap();
        file.get_cache()
    }
}

lazy_static! {
    static ref PAGE_CACHE_MANAGER: Arc<PageCacheManager> = Arc::new(PageCacheManager::new());
}

pub fn page_cache_manager() -> Arc<PageCacheManager> {
    PAGE_CACHE_MANAGER.clone()
}
