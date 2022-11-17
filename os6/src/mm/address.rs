use core::iter::Step;

use crate::config::{PAGE_SIZE, PAGE_SIZE_BITS, PTE_PER_PAGE};

use super::page_table::PageTableEntry;

/// 物理地址。在 Sv39 页表机制中，虚拟地址转化得到的物理地址总共为 56 位，其中页号 44 位，页内偏移 12 位。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct PhysAddr(pub(crate) usize);

impl PhysAddr {
    // pub const fn page_offset(&self) -> usize {
    //     self.0 & (PAGE_SIZE - 1)
    // }
    /// 向下取整页号
    pub fn floor(&self) -> PhysPageNum {
        PhysPageNum(self.0 / PAGE_SIZE)
    }
    /// 向上取整页号
    pub fn ceil(&self) -> PhysPageNum {
        PhysPageNum((self.0 + PAGE_SIZE - 1) / PAGE_SIZE)
    }
    pub fn ppn(&self) -> PhysPageNum {
        self.floor()
    }
    pub fn as_mut<T>(&self) -> &'static mut T {
        unsafe { (self.0 as *mut T).as_mut().unwrap() }
    }
}

/// 物理页号。Sv39 中合法的页号只考虑低 44 位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysPageNum(pub(crate) usize);

impl PhysPageNum {
    /// 清空该页帧
    pub fn clear(&self) {
        unsafe { core::ptr::write_bytes(self.page_start().0 as *mut u8, 0, PAGE_SIZE) }
    }
    pub fn page_start(&self) -> PhysAddr {
        PhysAddr(self.0 << PAGE_SIZE_BITS)
    }
    pub fn as_page_ptes_mut(&mut self) -> &'static mut [PageTableEntry; PTE_PER_PAGE] {
        self.as_mut()
    }
    pub fn as_page_bytes(&self) -> &'static [u8; PAGE_SIZE] {
        self.as_ref()
    }
    pub fn as_page_bytes_mut(&mut self) -> &'static mut [u8; PAGE_SIZE] {
        self.as_mut()
    }
    pub fn as_ref<T>(&self) -> &'static T {
        let pa = self.page_start();
        unsafe { (pa.0 as *mut T).as_ref().unwrap() }
    }
    pub fn as_mut<T>(&mut self) -> &'static mut T {
        let pa = self.page_start();
        unsafe { (pa.0 as *mut T).as_mut().unwrap() }
    }
    /// 将页中 offset 处作为一个结构体返回，请注意内存布局
    pub fn as_mut_at<T>(&mut self, offset: usize) -> &'static mut T {
        let pa = self.page_start();
        unsafe { ((pa.0 + offset) as *mut T).as_mut().unwrap() }
    }
    /// 将 `src` 中的数据复制到该页中。
    ///
    /// 需要保证 `src` 与该页不相交且长度不超过页长
    pub fn copy_from(&mut self, src: &[u8]) {
        let pa = self.page_start();
        unsafe {
            let dst = core::slice::from_raw_parts_mut(pa.0 as _, src.len());
            dst.copy_from_slice(src);
        };
    }
}

/// 虚拟地址。在 Sv39 页表机制中，虚拟地址 38~0 有效，39 及高位和 38 位一致。页号 27 位，业内偏移 12 位。
///
/// 由于 63~39 和 38 位保持一致，虚拟地址空间中只有 64 位的最低 256 GB 地址和最高 256 GB 地址有效。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct VirtAddr(pub(crate) usize);

impl VirtAddr {
    pub const fn page_offset(&self) -> usize {
        self.0 & (PAGE_SIZE - 1)
    }
    /// 向下取整页号
    pub const fn floor(&self) -> VirtPageNum {
        VirtPageNum(self.0 / PAGE_SIZE)
    }
    pub const fn vpn(&self) -> VirtPageNum {
        self.floor()
    }
    /// 向上取整页号
    pub const fn ceil(&self) -> VirtPageNum {
        VirtPageNum((self.0 - 1 + PAGE_SIZE) / PAGE_SIZE)
    }
}

/// 虚拟页号。应满足：仅低 27 位有效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtPageNum(pub(crate) usize);

impl VirtPageNum {
    pub fn indexes(&self) -> [usize; 3] {
        let mut vpn = self.0;
        let mut idx = [0; 3];
        for i in idx.iter_mut().rev() {
            const LOW_MASK: usize = PTE_PER_PAGE - 1;
            *i = vpn & LOW_MASK;
            vpn >>= 9;
        }
        idx
    }
    pub fn page_start(&self) -> VirtAddr {
        VirtAddr(self.0 << PAGE_SIZE_BITS)
    }
}

impl Step for VirtPageNum {
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start > end {
            None
        } else {
            Some(end.0 - start.0)
        }
    }
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        // 0x0~0x3ff_ffff、0xffff_fc00_0000~0xffff_ffff_ffff 都是合法的虚拟页号
        if start.0 + count < 1 << 26
            || (start.0 + count >= (1 << 52) - (1 << 26) && start.0 + count < (1 << 52))
        {
            Some(Self(start.0 + count))
        } else {
            None
        }
    }
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        if start.0 >= count {
            Some(Self(start.0 - count))
        } else {
            None
        }
    }
}
