use alloc::sync::Arc;

use crate::{sync::UPSafeCell, timer, trap::TrapContext};

use super::{
    context::TaskContext, manager::TaskManager, switch::__switch, tcb::TaskControlBlock, TaskStatus,
};

pub static PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };

/// 负责管理处理器
pub struct Processor {
    current: Option<Arc<TaskControlBlock>>,
    /// 每个 Processor 都有一个 idle 控制流，它尝试从 TaskManager 中选出一个任务来执行
    idle_task_ctx: TaskContext,
}

impl Processor {
    pub const fn new() -> Self {
        Self {
            current: None,
            idle_task_ctx: TaskContext::zero_init(),
        }
    }
    fn idle_task_ctx_ptr(&self) -> *const TaskContext {
        &self.idle_task_ctx as *const _
    }
    pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
        PROCESSOR.exclusive_access().current.take()
    }
    pub fn current_task() -> Option<Arc<TaskControlBlock>> {
        PROCESSOR.exclusive_access().current.clone()
    }
    pub fn current_user_satp() -> usize {
        Self::current_task()
            .unwrap()
            .inner_exclusive_access()
            .user_satp()
    }
    pub fn current_trap_ctx() -> &'static mut TrapContext {
        Self::current_task()
            .unwrap()
            .inner_exclusive_access()
            .trap_ctx()
    }

    /// 应用交出控制权，切入内核态后，将会调用 `schedule` 函数进入 idle 控制流进行任务调度
    pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
        let idle_task_cx_ptr = PROCESSOR.exclusive_access().idle_task_ctx_ptr();
        unsafe {
            __switch(switched_task_cx_ptr, idle_task_cx_ptr);
        }
    }
}

/// idle 控制流不断运行该函数，从 TaskManager 拉取任务
pub fn run_tasks() -> ! {
    loop {
        if let Some(task) = TaskManager::fetch_task() {
            let next_task_ctx_ptr = {
                let mut task_inner = task.inner_exclusive_access();
                task_inner.task_status = TaskStatus::Running;
                if task_inner.start_time == 0 {
                    task_inner.start_time = timer::get_time_ms();
                }
                &task_inner.task_ctx as *const TaskContext
            };
            let idle_task_ctx_ptr = {
                let mut processor = PROCESSOR.exclusive_access();
                processor.current = Some(task);
                &mut processor.idle_task_ctx as *mut _
            };

            unsafe {
                __switch(idle_task_ctx_ptr, next_task_ctx_ptr);
            }
        }
    }
}
