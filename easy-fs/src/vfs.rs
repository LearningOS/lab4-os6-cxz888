use super::{
    block_cache, block_cache_sync_all, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};

/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }
    pub fn inode_id(&self) -> usize {
        let fs = self.fs.lock();
        fs.inode_id(self.block_id, self.block_offset) as usize
    }
    /// 返回 0 为 NULL，1 为 Dir，2 为 File
    pub fn inode_type(&self) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|inode| {
            if inode.is_dir() {
                return 1;
            }
            if inode.is_file() {
                return 2;
            }
            return 0;
        })
    }
    pub fn inode_link_num(&self) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|inode| inode.link_num as usize)
    }
    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(dirent.inode_number() as u32);
            }
        }
        None
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Self>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    Arc::clone(&self.fs),
                    self.block_device.clone(),
                ))
            })
        })
    }
    pub fn find_entry_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(i as u32);
            }
        }
        None
    }
    /// 应当获取锁后调用
    ///
    /// FIXME: 偷了懒，移除目录项可能导致数据块的回收、inode size 的变化等
    fn swap_remove(&self, entry_id: usize, disk_inode: &mut DiskInode) -> u32 {
        let offset = entry_id * DIRENT_SZ;
        let mut dir_entry = DirEntry::empty();
        let last_offset = disk_inode.size as usize - DIRENT_SZ;
        disk_inode.read_at(last_offset, dir_entry.as_bytes_mut(), &self.block_device);
        disk_inode.write_at(offset, dir_entry.as_bytes(), &self.block_device);
        disk_inode.size -= DIRENT_SZ as u32;
        dir_entry.inode_number()
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    pub fn link(&self, old: &str, new: &str) -> bool {
        let mut fs = self.fs.lock();
        if let Some(id) = self.read_disk_inode(|root_inode| {
            assert!(root_inode.is_dir());
            self.find_inode_id(old, root_inode)
        }) {
            let dirent = DirEntry::new(new, id);
            self.modify_disk_inode(|root_inode| {
                let file_count = (root_inode.size as usize) / DIRENT_SZ;
                let new_size = (file_count + 1) * DIRENT_SZ;
                self.increase_size(new_size as u32, root_inode, &mut fs);
                root_inode.write_at(
                    file_count * DIRENT_SZ,
                    dirent.as_bytes(),
                    &self.block_device,
                );
            });
            log::debug!("write link ok");
            let (inode_block_id, inode_block_offset) = fs.get_disk_inode_pos(id);
            block_cache(inode_block_id as usize, Arc::clone(&self.block_device))
                .lock()
                .modify(inode_block_offset, |inode: &mut DiskInode| {
                    inode.link_num += 1;
                });
            true
        } else {
            false
        }
    }
    pub fn unlink(&self, path: &str) -> bool {
        let mut fs = self.fs.lock();
        if let Some(id) = self.read_disk_inode(|root_inode| {
            assert!(root_inode.is_dir());
            self.find_entry_id(path, root_inode)
        }) {
            let inode_id =
                self.modify_disk_inode(|root_inode| self.swap_remove(id as usize, root_inode));
            let (inode_block_id, inode_block_offset) = fs.get_disk_inode_pos(inode_id);
            block_cache(inode_block_id as usize, Arc::clone(&self.block_device))
                .lock()
                .modify(inode_block_offset, |inode: &mut DiskInode| {
                    inode.link_num -= 1;
                    if inode.link_num == 0 {
                        let size = inode.size;
                        let data_blocks_dealloc = inode.clear_size(&self.block_device);
                        assert!(
                            data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize
                        );
                        for data_block in data_blocks_dealloc {
                            fs.dealloc_data(data_block);
                        }
                        fs.dealloc_inode(inode_id as usize);
                    }
                });
            block_cache_sync_all();
            true
        } else {
            false
        }
    }
    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        if self
            .read_disk_inode(|root_inode| {
                // assert it is a directory
                assert!(root_inode.is_dir());
                // has the file been created?
                self.find_inode_id(name, root_inode)
            })
            .is_some()
        {
            return None;
        }
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut ret = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                ret.push(String::from(dirent.name()));
            }
            ret
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
}
