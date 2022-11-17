use alloc::vec::Vec;

use crate::{
    config::{KERNEL_STACK_SIZE, PAGE_SIZE, TRAMPOLINE},
    mm::{
        address::VirtAddr,
        memory_set::{MapPermission, KERNEL_SPACE},
    },
    sync::UPSafeCell,
};

pub struct PidHandle(pub usize);

pub struct PidAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl PidAllocator {
    const fn new() -> Self {
        PidAllocator {
            current: 0,
            recycled: Vec::new(),
        }
    }
    pub fn alloc() -> PidHandle {
        let mut allocator = PID_ALLOCATOR.exclusive_access();
        if let Some(pid) = allocator.recycled.pop() {
            PidHandle(pid)
        } else {
            allocator.current += 1;
            PidHandle(allocator.current - 1)
        }
    }
    fn dealloc(&mut self, pid: usize) {
        assert!(pid < self.current);
        assert!(
            self.recycled.iter().find(|&&ppid| ppid == pid).is_none(),
            "pid {} has been deallocated!",
            pid
        );
        self.recycled.push(pid);
    }
}

impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

static PID_ALLOCATOR: UPSafeCell<PidAllocator> = unsafe { UPSafeCell::new(PidAllocator::new()) };

pub struct KernelStack {
    pid: usize,
}

/// 返回内核栈在内核地址空间中的 (bottom, top)。注意 bottom 为低地址。
pub const fn kernel_stack_position(app_id: usize) -> (usize, usize) {
    let top = TRAMPOLINE - app_id * (KERNEL_STACK_SIZE + PAGE_SIZE);
    let bottom = top - KERNEL_STACK_SIZE;
    (bottom, top)
}

impl KernelStack {
    pub fn new(pid_handle: &PidHandle) -> Self {
        let pid = pid_handle.0;
        let (kernel_stack_bottom, kernel_stack_top) = kernel_stack_position(pid);
        KERNEL_SPACE.exclusive_access().insert_framed_area(
            VirtAddr(kernel_stack_bottom),
            VirtAddr(kernel_stack_top),
            MapPermission::R | MapPermission::W,
        );
        KernelStack { pid: pid_handle.0 }
    }
    pub const fn top(&self) -> usize {
        let (_, kernel_stack_top) = kernel_stack_position(self.pid);
        kernel_stack_top
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        let (kernel_stack_bottom, _) = kernel_stack_position(self.pid);
        let kernel_stack_bottom_va = VirtAddr(kernel_stack_bottom);
        KERNEL_SPACE
            .exclusive_access()
            .remove_area_with_start_vpn(kernel_stack_bottom_va.vpn());
    }
}
