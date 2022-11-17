use alloc::{string::String, vec, vec::Vec};
use bitflags::bitflags;
use riscv::register::satp;

use super::{
    address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum},
    frame_allocator::{frame_alloc, FrameTracker},
};
use crate::config::PAGE_SIZE;

bitflags! {
    pub struct PTEFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

#[derive(Clone)]
pub struct PageTableEntry {
    pub bits: usize,
}

impl PageTableEntry {
    pub const fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
        PageTableEntry {
            bits: ppn.0 << 10 | flags.bits as usize,
        }
    }
    pub const fn empty() -> Self {
        PageTableEntry { bits: 0 }
    }
    pub fn ppn(&self) -> PhysPageNum {
        const LOW_44_MASK: usize = (1 << 44) - 1;
        PhysPageNum((self.bits >> 10) & LOW_44_MASK)
    }
    pub fn flags(&self) -> PTEFlags {
        PTEFlags::from_bits_truncate(self.bits as u8)
    }
    pub fn is_valid(&self) -> bool {
        self.flags() & PTEFlags::V != PTEFlags::empty()
    }
    pub fn readable(&self) -> bool {
        self.flags() & PTEFlags::R != PTEFlags::empty()
    }
    pub fn writable(&self) -> bool {
        self.flags() & PTEFlags::W != PTEFlags::empty()
    }
    pub fn executable(&self) -> bool {
        self.flags() & PTEFlags::X != PTEFlags::empty()
    }
}

/// 注意 `PageTable` 所拥有的的物理页仅用于存放页表节点数据。
#[derive(Debug)]
pub struct PageTable {
    root_ppn: PhysPageNum,
    frames: Vec<FrameTracker>,
}

impl PageTable {
    pub fn new() -> Self {
        log::trace!("new PageTable");
        let frame = frame_alloc().unwrap();
        PageTable {
            root_ppn: frame.ppn,
            frames: vec![frame],
        }
    }
    /// 创造一个专门用于手动查询的页表。
    ///
    /// 在内核看来，ekernel 之后的所有地址都是 Identical 映射的
    pub fn from_satp(satp: usize) -> Self {
        const LOW_44_MASK: usize = (1 << 44) - 1;
        // RV64 中 `satp` 低 44 位是根页表的 PPN
        // frames 为空，意味着它只是临时使用，而不管理 frame 的资源
        Self {
            root_ppn: PhysPageNum(satp & LOW_44_MASK),
            frames: Vec::new(),
        }
    }
    pub fn satp(&self) -> usize {
        (satp::Mode::Sv39 as usize) << 60 | self.root_ppn.0
    }

    /// 将 vpn 映射到 ppn，且其标志位设为 flags | V
    pub fn map(&mut self, vpn: VirtPageNum, ppn: PhysPageNum, flags: PTEFlags) {
        log::trace!("map vpn: {:#x} to ppn: {:#x}", vpn.0, ppn.0);
        let pte = self.find_pte_create(vpn);
        // 这个 pte 之前不能被映射过。
        assert!(!pte.is_valid(), "vpn {} is mapped before mapping", vpn.0);
        *pte = PageTableEntry::new(ppn, flags | PTEFlags::V)
    }
    /// 解除 vpn 的映射
    pub fn unmap(&mut self, vpn: VirtPageNum) {
        let pte = self.find_pte_create(vpn);
        // 这个 pte 之前必须被映射过。
        assert!(pte.is_valid(), "vpn {} is invalid before unmapping", vpn.0);
        *pte = PageTableEntry::empty();
    }
    /// 尝试寻找 vpn 对应的 pte。如果遇到未分配的页帧就会返回 None。
    pub fn find_pte(&self, vpn: VirtPageNum) -> Option<&PageTableEntry> {
        let idx = vpn.indexes();
        let mut ppn = self.root_ppn;
        for i in 0..idx.len() {
            let pte = &mut ppn.as_page_ptes_mut()[idx[i]];
            // 找到叶 PTE 后，不着急设置为有效，交给调用者处理
            if i == idx.len() - 1 {
                return Some(pte);
            }
            if !pte.is_valid() {
                return None;
            }
            ppn = pte.ppn();
        }
        unreachable!()
    }
    /// 尝试寻找 vpn 对应的 pte。如果查询过程中遇到了未分配的页帧就会自动创建。
    ///
    /// # Panics
    ///
    /// 物理内存不足时会 panic
    fn find_pte_create(&mut self, vpn: VirtPageNum) -> &'static mut PageTableEntry {
        let idx = vpn.indexes();
        let mut ppn = self.root_ppn;
        for i in 0..idx.len() {
            let pte = &mut ppn.as_page_ptes_mut()[idx[i]];
            // 找到叶 PTE 后，不着急设置为有效，交给调用者处理
            if i == idx.len() - 1 {
                return pte;
            }
            if !pte.is_valid() {
                let frame = frame_alloc().expect("Physical Memory should be enough");
                *pte = PageTableEntry::new(frame.ppn, PTEFlags::V);
                self.frames.push(frame);
            }
            ppn = pte.ppn();
        }
        unreachable!()
    }
    /// 采用 `find_pte` 的实现，查页表失败就会返回 None
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.find_pte(vpn).map(Clone::clone)
    }
    pub fn translate_va_to_pa(&mut self, va: VirtAddr) -> PhysAddr {
        PhysAddr(self.find_pte(va.vpn()).unwrap().ppn().page_start().0 + va.page_offset())
    }
    pub fn translate_va_as<T>(&mut self, va: VirtAddr) -> &'static mut T {
        self.find_pte(va.vpn())
            .unwrap()
            .ppn()
            .as_mut_at(va.page_offset())
    }
    pub fn translated_mut<T>(satp: usize, ptr: *mut T) -> &'static mut T {
        let mut page_table = PageTable::from_satp(satp);
        let va = VirtAddr(ptr as usize);
        page_table.translate_va_as(va)
    }
    pub fn translated_str(satp: usize, ptr: *const u8) -> String {
        let mut page_table = PageTable::from_satp(satp);
        let mut bytes = Vec::new();
        let mut va = ptr as usize;
        // 内核不知道用户地址空间中字符串的长度，而且字符串可能跨页，所以逐字节查页表，直到为 `\0`
        loop {
            let byte: u8 = *(page_table.translate_va_as(VirtAddr(va)));
            if byte == 0 {
                break;
            } else {
                bytes.push(byte);
                va += 1;
            }
        }
        String::from_utf8(bytes).unwrap()
    }
}

pub fn translated_byte_buffer(satp: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let page_table = PageTable::from_satp(satp);
    let mut start = ptr as usize;
    let end = start + len;
    let mut v = Vec::with_capacity(len / PAGE_SIZE + 2);
    while start < end {
        let start_va = VirtAddr(start);
        let mut vpn = start_va.floor();
        let mut ppn = page_table.translate(vpn).unwrap().ppn();
        vpn.0 += 1;
        let mut end_va = vpn.page_start();
        end_va = end_va.min(VirtAddr(end));
        if end_va.page_offset() == 0 {
            v.push(&mut ppn.as_page_bytes_mut()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.as_page_bytes_mut()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.0;
    }
    v
}

pub struct UserBuffer {
    pub buffers: Vec<&'static mut [u8]>,
}

impl UserBuffer {
    pub fn new(buffers: Vec<&'static mut [u8]>) -> Self {
        Self { buffers }
    }
    pub fn len(&self) -> usize {
        self.buffers.iter().fold(0, |tot, buf| tot + buf.len())
    }
}

impl IntoIterator for UserBuffer {
    type IntoIter = IntoIter;
    type Item = *mut u8;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            buffers: self.buffers,
            index: 0,
            offset: 0,
        }
    }
}

pub struct IntoIter {
    buffers: Vec<&'static mut [u8]>,
    index: usize,
    offset: usize,
}

impl Iterator for IntoIter {
    type Item = *mut u8;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.buffers.len() {
            return None;
        }
        let buffer = &mut self.buffers[self.index];
        let len = buffer.len();
        let ret = &mut buffer[self.offset];
        self.offset += 1;
        if self.offset >= len {
            self.offset = 0;
            self.index += 1;
        }
        Some(ret as *mut u8)
    }
}
