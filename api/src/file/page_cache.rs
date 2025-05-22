use alloc::{collections::{btree_map::BTreeMap, btree_set::BTreeSet}, sync::{Arc, Weak}, vec::Vec, vec};
use axio::SeekFrom;
use axsync::Mutex;
use spin::{RwLock, Lazy};
use axalloc::GlobalPage;
use axerrno::LinuxResult;
use memory_addr::{VirtAddr, PAGE_SIZE_4K};
use super::Kstat;
use hashbrown::hash_set;
use axerrno::ax_err_type;

struct FilePage {
    inner: GlobalPage,
    init: bool,
    dirty: bool,
}

impl FilePage {
    pub fn new() -> Self {
        FilePage { 
            inner: {
                let result = GlobalPage::alloc();
                let page = match result {
                    Ok(page) => page,
                    Err(_) => panic!("GlobalPage alloc failed"),
                };
                page
            },
            init: false, 
            dirty: false 
        }
    }
}

// 只有这里会长久存有 FilePage 的 Arc 指针
// 其他地方，要么是 Weak，要么只是临时使用 Arc
struct KernalPagePool {
    page_pool: Vec<Option<Arc<RwLock<FilePage>>>>,
    free_pages: Vec<usize>,
    alloced_paes: BTreeMap<VirtAddr, usize>,
    max_size: usize,
}

impl KernalPagePool {
    pub fn new(max_size: usize) -> Self {
        let mut ret = KernalPagePool { 
            page_pool: vec![None; max_size], 
            free_pages: Vec::new(),
            alloced_paes: BTreeMap::new(),
            max_size, 
        };

        for i in 0..max_size {
            ret.free_pages.push(i);
        }
        return ret;
    }

    pub fn alloc_page(&mut self) -> Arc<RwLock<FilePage>> {
        if self.free_pages.is_empty() {
            panic!("Page Cache Free Page empty!");
        }
        let id= self.free_pages.pop().unwrap();
        assert!(self.page_pool[id].is_none());
        let page = Arc::new(RwLock::new(FilePage::new()));
        self.page_pool[id] = Some(page.clone());
        let addr = page.read().inner.start_vaddr();
        // warn!("Kernel Page Pool alloc page: {:#x}", addr);
        self.alloced_paes.insert(addr, id);
        page
    }

    pub fn dealloc_page(&mut self, start_addr: VirtAddr) {
        let id = *self.alloced_paes.get(&start_addr).unwrap();
        self.page_pool[id] = None;
        self.free_pages.push(id);
        self.alloced_paes.remove(&start_addr);
    }
}

const MAX_KERNEL_PAGE_POOL_SIZE: usize = 40;
static KERNEL_PAGE_POOL: Lazy<Mutex<KernalPagePool>> = Lazy::new(|| {Mutex::new(
    KernalPagePool::new(MAX_KERNEL_PAGE_POOL_SIZE)
)});

fn kernel_page_pool() -> &'static Mutex<KernalPagePool> {
    &KERNEL_PAGE_POOL
}

pub struct FilePageCache {
    pub file: Arc<Mutex<axfs::fops::File>>,
    offset: usize,
    stat: Kstat,
    pages: BTreeMap<usize, Weak<RwLock<FilePage>>>,
    dirty_pages_set: BTreeSet<usize>,
}

impl FilePageCache {
    pub fn new(file: Arc<Mutex<axfs::fops::File>>) -> Self {
        let res = FilePageCache { 
            file: file.clone(),
            offset: {
                let file = file.clone();
                file.lock().get_offset() as usize
            },
            // stat 的初始化照抄 api/src/file/fs.rs
            stat: {
                let metadata = file.clone().lock().get_attr().unwrap();
                let ty = metadata.file_type() as u8;
                let perm = metadata.perm().bits() as u32;
                Kstat {
                    mode: ((ty as u32) << 12) | perm,
                    size: metadata.size(),
                    blocks: metadata.blocks(),
                    blksize: 512,
                    ..Default::default()
                }
            },
            // file_len: file.lock().get_attr().unwrap().size() as usize,
            pages: BTreeMap::new(),
            dirty_pages_set: BTreeSet::new(),
        };
        return res;
    }

    fn get_page(&mut self, page_id: usize) -> Arc<RwLock<FilePage>> {
        if let Some(page) = self.pages.get(&page_id) {
            if let Some(page) = page.upgrade() {
                return page;
            }
        }
        
        let mut pool = kernel_page_pool().lock();
        let page = pool.alloc_page();
        // warn!("PageCache alloc pageid = {}, addrr = {}", page_id, page);
        self.pages.insert(page_id, Arc::downgrade(&page));
        page
    }
    
    fn read_slice_from_page(&mut self, page_id: usize, page_start: usize, page_end: usize, buf: &mut [u8]) {
        let init = {
            let page = self.get_page(page_id);
            let page = page.read();
            page.init
        };

        // read 需要加载，而 write 则不需要
        if !init {
            let page = self.get_page(page_id);
            let mut page = page.write();
            let _ = self.file.lock().read_at((page_id * PAGE_SIZE_4K) as u64, page.inner.as_slice_mut());
            page.init = true;
        }        

        let page = self.get_page(page_id);
        let page = page.read();
        let slice = page.inner.as_slice();
        buf.copy_from_slice(&slice[page_start..page_end])
    }

    fn write_slice_into_page(&mut self, page_id: usize, page_start: usize, page_end: usize, buf: &[u8]) {
        let page = self.get_page(page_id);
        let mut page = page.write();

        // 在这里设置脏页标记
        page.dirty = true;
        page.init = true;
        self.dirty_pages_set.insert(page_id);

        let slice = page.inner.as_slice_mut();
        slice[page_start..page_end].copy_from_slice(&buf);
    }
    
    pub fn read_at(&mut self, offset: usize, buf: &mut [u8]) -> LinuxResult<usize> {
        let mut ret = 0 as usize;
        let file_len = self.stat.size as usize;
        let read_len = buf.len().min(file_len - offset);

        if read_len == 0 {
            return Ok(0);
        }

        while ret < read_len {
            let current_pos = offset + ret;
            let page_id = current_pos / PAGE_SIZE_4K;
            let page_offset_start = current_pos % PAGE_SIZE_4K;
            // 计算当前页剩余空间
            let bytes_left_in_page = PAGE_SIZE_4K - page_offset_start;
            // 计算本次可读取的最大长度（不超过页尾和总读取长度）
            let cur_len = bytes_left_in_page.min(read_len - ret);

            // 从 pagecache 读取数据到 buf
            let slice = &mut buf[ret..ret + cur_len];
            self.read_slice_from_page(page_id, page_offset_start, page_offset_start + cur_len, slice);
            ret += cur_len;
        }

        Ok(ret)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> LinuxResult<usize> {
        let result = self.read_at(self.offset, buf);
        self.offset += result.unwrap();
        result
    }

    pub fn write_at(&mut self, offset: usize, buf: &[u8]) -> LinuxResult<usize> {
        // warn!("page cache write: offset {}, len{}", offset, buf.len());
        
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
        if offset + ret > self.stat.size as usize {
            self.stat.size = (offset + ret) as u64;
        }

        Ok(ret)
    }

    pub fn write(&mut self, buf: &[u8]) -> LinuxResult<usize> {
        let result = self.write_at(self.offset, buf);
        self.offset += result.unwrap();
        result
    }

    pub fn stat(&self) -> LinuxResult<Kstat> {
        Ok(self.stat)
    }

    pub fn seek(&mut self, pos: SeekFrom) -> LinuxResult<isize> {
        let size = self.stat.size;
        let new_offset = match pos {
            SeekFrom::Start(pos) => Some(pos),
            SeekFrom::Current(off) => (self.offset as u64).checked_add_signed(off),
            SeekFrom::End(off) => size.checked_add_signed(off),
        }
        .ok_or_else(|| ax_err_type!(InvalidInput))?;
        self.offset = new_offset as usize;
        Ok(new_offset as isize)
    }

    pub fn flush(&mut self) -> LinuxResult<isize> {
        // error!("start flush");

        // 将数据写入文件系统
        if let Some(last_page_id) = self.dirty_pages_set.last() {
            for page_id in &self.dirty_pages_set {
                let page = self.pages.get(page_id).unwrap();
    
                // 如果这里页面不存在，说明由 KernelPagePool 执行了页面置换，此时一定已经写回文件
                if let Some(page) = page.upgrade() {
                    // 如果这里没有脏页标记，说明被 KernelPagePool 异步刷盘了
                    let mut page = page.write();
                    assert!(self.dirty_pages_set.contains(&page_id));
    
                    if page.dirty {
                        // 当前页面要读取的长度
                        let length = if page_id == last_page_id {
                            let length = self.stat.size as usize % PAGE_SIZE_4K;
                            if length == 0 {PAGE_SIZE_4K} else {length}
                        } else {
                            PAGE_SIZE_4K
                        };
                        
                        let mut file = self.file.lock();
                        
                        // 修改文件指针，只有利用 write 才能改变文件长度
                        let cur_pos = file.seek(SeekFrom::Start((page_id * PAGE_SIZE_4K) as u64))?;
                        // error!("before write {}, offset {}, length {}", page_id, cur_pos, length);

                        let mut cur = 0 as usize;
                        loop {
                            let slice = &page.inner.as_slice_mut()[cur..length];
                            let write_length = file.write(slice)?;
                            cur += write_length;
                            if cur == length {
                                break;
                            }
                        } 
                        
                        // error!("after write {}, slice length {}, write length {}", page_id, slice.len(), length);
                        // error!("file size: {}", file.get_attr().unwrap().size());
                    }
                    page.dirty = false;
                }
            }
        }
                // 更新文件大小
        let _ = {
            let file = self.file.lock();
            // error!("check");
            file.truncate(self.stat.size)
        };

        // 更新文件指针
        let _ = {
            let mut file = self.file.lock();
            let pos = SeekFrom::Start(self.offset as u64);
            file.seek(pos)
        };

        
        self.dirty_pages_set.clear();

        // TODO: 不知道还有没有其他信息需要更新

        // error!("end flush");

        Ok(0)
    }

    pub fn clear(&mut self) -> LinuxResult<isize> {
        self.flush();
        // for (page_id, page) in &self.pages {
        //     if let Some(page) = page.upgrade() {
        //         let page = page.read();
        //         let mut pool = kernel_page_pool().lock();
        //         pool.dealloc_page(page.inner.start_vaddr());
        //     }
        // }
        Ok(0)
    }
}