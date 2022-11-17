use core::cell::RefMut;

use alloc::{
    sync::{Arc, Weak},
    vec,
    vec::Vec,
};

use crate::{
    config::{BIG_STRIDE, MAX_SYSCALL_NUM, TRAP_CONTEXT},
    fs::{
        stdio::{Stdin, Stdout},
        File,
    },
    mm::{
        address::{PhysPageNum, VirtAddr},
        memory_set::{MemorySet, KERNEL_SPACE},
    },
    sync::UPSafeCell,
    trap::{self, TrapContext},
};

use super::{
    context::TaskContext,
    manager::TaskManager,
    pid::{KernelStack, PidAllocator, PidHandle},
};

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Zombie
pub enum TaskStatus {
    UnInit,
    Ready,
    Running,
    Zombie,
}

pub struct TaskControlBlock {
    pub pid: PidHandle,
    pub kernel_stack: KernelStack,
    inner: UPSafeCell<TaskControlBlockInner>,
}

impl TaskControlBlock {
    pub fn new(elf_data: &[u8]) -> Self {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        // `from_elf` 中已经将为 TRAP_CONTEXT 分配好了地址，所以这里可以直接 `unwrap()`
        let trap_ctx_ppn = memory_set
            .translate(VirtAddr(TRAP_CONTEXT).vpn())
            .unwrap()
            .ppn();
        let pid = PidAllocator::alloc();
        let kernel_stack = KernelStack::new(&pid);
        let kernel_stack_top = kernel_stack.top();
        let tcb = Self {
            pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    task_ctx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    trap_ctx_ppn,
                    base_size: user_sp,
                    parent: None,
                    children: Vec::new(),
                    syscall_count: [0; MAX_SYSCALL_NUM],
                    start_time: 0,
                    exit_code: 0,
                    priority: 16,
                    pass: Pass(0),
                    fd_table: vec![
                        Some(Arc::new(Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                })
            },
        };
        let trap_ctx = tcb.inner_exclusive_access().trap_ctx();
        *trap_ctx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().satp(),
            kernel_stack_top,
            trap::trap_handler as usize,
        );
        tcb
    }
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        let mut parent_inner = self.inner_exclusive_access();
        let memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
        let trap_ctx_ppn = memory_set
            .translate(VirtAddr(TRAP_CONTEXT).vpn())
            .unwrap()
            .ppn();
        let pid = PidAllocator::alloc();
        let kernel_stack = KernelStack::new(&pid);
        let kernel_stack_top = kernel_stack.top();
        let tcb = Arc::new(Self {
            pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    task_ctx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    trap_ctx_ppn,
                    base_size: parent_inner.base_size,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    syscall_count: [0; 500],
                    start_time: 0,
                    exit_code: 0,
                    priority: 16,
                    pass: Pass(0),
                    fd_table: vec![
                        Some(Arc::new(Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                })
            },
        });
        parent_inner.children.push(Arc::clone(&tcb));
        let trap_ctx = tcb.inner_exclusive_access().trap_ctx();
        trap_ctx.kernel_sp = kernel_stack_top;
        tcb
    }
    pub fn exec(&self, elf_data: &[u8]) {
        let (memory_set, user_sp, entry) = MemorySet::from_elf(elf_data);
        let trap_ctx_ppn = memory_set
            .translate(VirtAddr(TRAP_CONTEXT).vpn())
            .unwrap()
            .ppn();
        let mut inner = self.inner_exclusive_access();
        inner.memory_set = memory_set;
        inner.trap_ctx_ppn = trap_ctx_ppn;
        let trap_ctx = inner.trap_ctx();
        *trap_ctx = TrapContext::app_init_context(
            entry,
            user_sp,
            KERNEL_SPACE.exclusive_access().satp(),
            self.kernel_stack.top(),
            trap::trap_handler as usize,
        );
    }
    pub fn spawn(self: &Arc<Self>, elf_data: &[u8]) -> usize {
        // 1. 创建子进程对应的 tcb
        let (memory_set, user_sp, entry) = MemorySet::from_elf(elf_data);
        let trap_ctx_ppn = memory_set
            .translate(VirtAddr(TRAP_CONTEXT).vpn())
            .unwrap()
            .ppn();
        let pid = PidAllocator::alloc();
        let kernel_stack = KernelStack::new(&pid);
        let kernel_stack_top = kernel_stack.top();
        let tcb = Arc::new(TaskControlBlock {
            pid,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    task_ctx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    trap_ctx_ppn,
                    base_size: user_sp,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    syscall_count: [0; MAX_SYSCALL_NUM],
                    start_time: 0,
                    exit_code: 0,
                    priority: 16,
                    pass: Pass(0),
                    fd_table: vec![
                        Some(Arc::new(Stdin)),
                        Some(Arc::new(Stdout)),
                        Some(Arc::new(Stdout)),
                    ],
                })
            },
        });
        // 2. 加入当前进程的子进程队列
        self.inner_exclusive_access()
            .children
            .push(Arc::clone(&tcb));
        // 3. 准备子进程的 trap_ctx
        let trap_ctx = tcb.inner_exclusive_access().trap_ctx();
        *trap_ctx = TrapContext::app_init_context(
            entry,
            user_sp,
            KERNEL_SPACE.exclusive_access().satp(),
            kernel_stack_top,
            trap::trap_handler as usize,
        );
        let pid = tcb.pid();
        // 4. 子进程等待调度
        TaskManager::add_task(tcb);
        pid
    }
    pub fn inner_exclusive_access(&self) -> RefMut<TaskControlBlockInner> {
        self.inner.exclusive_access()
    }
    pub fn pid(&self) -> usize {
        self.pid.0
    }
}

pub struct TaskControlBlockInner {
    pub task_ctx: TaskContext,
    pub task_status: TaskStatus,
    pub memory_set: MemorySet,
    /// Trap Context 所在的物理页号
    pub trap_ctx_ppn: PhysPageNum,
    /// 统计应用数据的大小，包括用户栈
    pub base_size: usize,
    pub parent: Option<Weak<TaskControlBlock>>,
    pub children: Vec<Arc<TaskControlBlock>>,
    pub syscall_count: [u32; MAX_SYSCALL_NUM],
    pub start_time: usize,
    pub exit_code: i32,
    pub priority: usize,
    pub pass: Pass,
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Pass(pub usize);

impl Ord for Pass {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        Self::partial_cmp(&self, other).unwrap()
    }
}

impl PartialOrd for Pass {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        use core::cmp::Ordering;
        if self.0 <= other.0 {
            if other.0 - self.0 > BIG_STRIDE / 2 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Less)
            }
        } else {
            if self.0 - other.0 > BIG_STRIDE / 2 {
                Some(Ordering::Less)
            } else {
                Some(Ordering::Greater)
            }
        }
    }
}

impl TaskControlBlockInner {
    pub fn trap_ctx(&mut self) -> &'static mut TrapContext {
        self.trap_ctx_ppn.as_mut()
    }
    pub fn user_satp(&self) -> usize {
        self.memory_set.satp()
    }
    pub fn is_zombie(&self) -> bool {
        self.task_status == TaskStatus::Zombie
    }
    pub fn alloc_fd(&mut self) -> usize {
        for (fd, file) in self.fd_table.iter().enumerate() {
            if file.is_none() {
                return fd;
            }
        }
        self.fd_table.push(None);
        self.fd_table.len() - 1
    }
}
