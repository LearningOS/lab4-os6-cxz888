use alloc::{collections::VecDeque, sync::Arc};
use lazy_static::lazy_static;

pub use super::tcb::TaskStatus;
use super::{tcb::TaskControlBlock, INITPROC};
use crate::{config::BIG_STRIDE, sync::UPSafeCell};

lazy_static! {
    static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

pub struct TaskManager {
    pub ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    pub fn add_task(task: Arc<TaskControlBlock>) {
        TASK_MANAGER.exclusive_access().ready_queue.push_back(task)
    }
    pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
        let ready_queue = &mut TASK_MANAGER.exclusive_access().ready_queue;
        if let Some((index, _)) = ready_queue
            .iter()
            .enumerate()
            .min_by_key(|(_, task)| task.inner_exclusive_access().pass)
        {
            let ret = ready_queue.swap_remove_back(index).unwrap();
            {
                let mut inner = ret.inner_exclusive_access();
                inner.pass.0 += BIG_STRIDE / inner.priority;
            }
            Some(ret)
        } else {
            None
        }
    }
}

pub fn add_initproc() {
    TaskManager::add_task(INITPROC.clone());
}
