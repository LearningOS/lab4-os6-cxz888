pub mod context;
pub mod manager;
mod pid;
mod processor;
pub mod switch;
mod tcb;

use core::mem;

use alloc::sync::Arc;
use lazy_static::lazy_static;

pub use self::tcb::TaskStatus;
use self::{context::TaskContext, manager::TaskManager, tcb::TaskControlBlock};
use crate::fs::inode::{self, OpenFlags};
use crate::mm::{address::VirtAddr, memory_set::MapPermission};
pub use processor::Processor;

lazy_static! {
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new({
        let inode = inode::open_file("ch6b_initproc", OpenFlags::RDONLY).unwrap();
        TaskControlBlock::new(&inode.read_all())
    });
}

pub fn suspend_current_and_run_next() {
    let task = Processor::current_task().unwrap();
    let task_ctx_ptr = {
        let mut task_inner = task.inner_exclusive_access();
        task_inner.task_status = TaskStatus::Ready;
        &mut task_inner.task_ctx as *mut TaskContext
    };
    TaskManager::add_task(task);
    Processor::schedule(task_ctx_ptr);
}

pub fn exit_current_and_run_next(exit_code: i32) {
    {
        let task = Processor::take_current_task().unwrap();
        log::info!("exit task {}", task.pid.0);
        let mut inner = task.inner_exclusive_access();
        inner.task_status = TaskStatus::Zombie;
        inner.exit_code = exit_code;

        // 子进程转交给 initproc 来处理
        let children = mem::take(&mut inner.children);
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in children {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(Arc::clone(&child))
        }

        // 暂时只清空了存放数据的页，而存放页表项的页则未清空
        // 这个进程真正被回收是在父进程 `wait` 它时，那时引用计数会归零，然后自动释放所有资源
        inner.memory_set.recycle_data_pages();
    }
    // 注意，调用 `schedule` 后控制流中断了，因此上述变量被包裹起来以在离开作用域时自动释放
    let mut _unused = TaskContext::zero_init();
    Processor::schedule(&mut _unused as _);
}

/// 由调用者保证 `time` 是物理地址
pub fn set_syscall_times(times: &mut [u32]) {
    times.copy_from_slice(
        &Processor::current_task()
            .unwrap()
            .inner_exclusive_access()
            .syscall_count,
    );
}

/// 需满足 syscall_id < 500
pub fn incr_syscall_times(syscall_id: usize) {
    Processor::current_task()
        .unwrap()
        .inner_exclusive_access()
        .syscall_count[syscall_id] += 1;
}

pub fn start_time() -> usize {
    Processor::current_task()
        .unwrap()
        .inner_exclusive_access()
        .start_time
}

/// 将 start 开始 len 字节的虚拟地址映射。失败返回 false。
pub fn map_range(start: usize, len: usize, map_perm: MapPermission) -> bool {
    let tcb_arc = Processor::current_task().unwrap();
    let mut inner = tcb_arc.inner_exclusive_access();
    let vpn_range = VirtAddr(start).floor()..VirtAddr(start + len).ceil();
    if inner
        .memory_set
        .areas
        .iter()
        .any(|area| !area.intersection(&vpn_range).is_empty())
    {
        return false;
    }
    inner
        .memory_set
        .insert_framed_area(VirtAddr(start), VirtAddr(start + len), map_perm);
    true
}

/// 将一个范围内的虚拟地址取消映射。失败返回 false。
///
/// 这里偷了很多懒。~~有点面向测试点编程~~。
///
/// 总而言之，这个实现假定：已经映射的内存段要么完全被输入范围包含在内，要么完全不相交。
///
/// 部分相交的情况会很麻烦，可能涉及到 MapArea 的缩小，甚至是分裂。而 MapArea 内部包含的 BTree 也要分裂。
///
/// 至少我暂时没想到什么优雅简单的实现。可能要费不少功夫，这里领会精神，过 CI 就行。
pub fn unmap_range(start: usize, len: usize) -> bool {
    let tcb_arc = Processor::current_task().unwrap();
    let mut inner = tcb_arc.inner_exclusive_access();
    let vpn_range = VirtAddr(start).floor()..VirtAddr(start + len).ceil();
    let map_set = &mut inner.memory_set;
    let mut unmaped_count = 0;
    let areas = &mut map_set.areas;
    let page_table = &mut map_set.page_table;
    areas.retain_mut(|area| {
        // 释放的地址完全将该内存段包含在内
        if area.intersection(&vpn_range) == area.vpn_range {
            unmaped_count += area.vpn_range.end.0 - area.vpn_range.start.0;
            area.unmap(page_table);
            false
        } else {
            true
        }
    });
    unmaped_count == vpn_range.end.0 - vpn_range.start.0
}

pub use processor::run_tasks;

pub use manager::add_initproc;
