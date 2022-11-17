use alloc::vec::Vec;

use crate::{config::MEMORY_END, mm::address::PhysAddr, sync::UPSafeCell};

use super::address::PhysPageNum;

/// 物理页帧管理器
pub trait FrameAllocator {
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}

/// 朴素的栈式管理
pub struct StackFrameAllocator {
    current: PhysPageNum,
    end: PhysPageNum,
    recycled: Vec<PhysPageNum>,
}

impl Default for StackFrameAllocator {
    fn default() -> Self {
        Self {
            current: PhysPageNum(0),
            end: PhysPageNum(0),
            recycled: Vec::new(),
        }
    }
}

impl StackFrameAllocator {
    pub const fn new() -> Self {
        Self {
            current: PhysPageNum(0),
            end: PhysPageNum(0),
            recycled: Vec::new(),
        }
    }
    pub fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        assert!(l < r, "PPN range invalid(l:{}, r:{})", l.0, r.0);
        self.current = l;
        self.end = r;
    }
}

impl FrameAllocator for StackFrameAllocator {
    /// 如果有回收的物理页，则出栈并返回。否则从区间左侧弹出。
    fn alloc(&mut self) -> Option<PhysPageNum> {
        self.recycled.pop().or_else(|| {
            if self.current == self.end {
                None
            } else {
                self.current.0 += 1;
                Some(PhysPageNum(self.current.0 - 1))
            }
        })
    }

    fn dealloc(&mut self, ppn: PhysPageNum) {
        if ppn >= self.current || self.recycled.iter().any(|&n| n == ppn) {
            panic!("Frame ppn={:#x} has not been allocated!", ppn.0);
        }
        self.recycled.push(ppn);
    }
}

static FRAME_ALLOCATOR: UPSafeCell<StackFrameAllocator> =
    unsafe { UPSafeCell::new(StackFrameAllocator::new()) };

/// initiate the frame allocator using `ekernel` and `MEMORY_END`
pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    // ekernel 之前都是系统使用的内存，之后的内存则可以分配给应用
    FRAME_ALLOCATOR.exclusive_access().init(
        PhysAddr(ekernel as usize).ceil(),
        PhysAddr(MEMORY_END).floor(),
    );
}

#[derive(Debug)]
pub struct FrameTracker {
    pub ppn: PhysPageNum,
}

impl FrameTracker {
    pub fn new(ppn: PhysPageNum) -> Self {
        log::trace!("clear frame: {:#x}", ppn.0);
        ppn.clear();
        Self { ppn }
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        FRAME_ALLOCATOR.exclusive_access().dealloc(self.ppn)
    }
}

pub fn frame_alloc() -> Option<FrameTracker> {
    log::trace!("allocate frame");
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
}

pub fn frame_dealloc(ppn: PhysPageNum) {
    log::trace!("deallocate frame");
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn)
}
