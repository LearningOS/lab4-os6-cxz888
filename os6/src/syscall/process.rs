use alloc::sync::Arc;

use crate::{
    config::{MAX_SYSCALL_NUM, PAGE_SIZE},
    fs::inode::{self, OpenFlags},
    mm::{address::VirtAddr, memory_set::MapPermission, page_table::PageTable},
    task::{self, manager::TaskManager, Processor, TaskStatus},
    timer::{self, MICRO_PER_SEC},
};

pub fn sys_exit(exit_code: i32) -> ! {
    log::info!("[kernel] Application exited with code {}", exit_code);
    task::exit_current_and_run_next(exit_code);
    unreachable!();
}

/// APP 将 CPU 控制权交给 OS，由 OS 决定下一步。
///
/// 总是返回 0.
///
/// syscall ID: 124
pub fn sys_yield() -> isize {
    task::suspend_current_and_run_next();
    0
}

#[repr(C)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// `_tz` 在我们的实现中忽略
///
/// syscall ID: 169
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    let ts_mut = PageTable::translated_mut(Processor::current_user_satp(), ts);
    let us = timer::get_time_us();
    ts_mut.sec = us / MICRO_PER_SEC;
    ts_mut.usec = us % MICRO_PER_SEC;
    0
}

pub struct TaskInfo {
    status: TaskStatus,
    syscall_times: [u32; MAX_SYSCALL_NUM],
    time: usize,
}

/// 查询任务信息。syscall_id = 410
///
/// 成功返回 0，错误返回 -1
pub fn sys_task_info(ti: *mut TaskInfo) -> isize {
    let page_table = PageTable::from_satp(Processor::current_user_satp());
    let ti_va = VirtAddr(ti as usize);
    let ti_mut = page_table
        .translate(ti_va.floor())
        .unwrap()
        .ppn()
        .as_mut_at::<TaskInfo>(ti_va.page_offset());
    ti_mut.status = TaskStatus::Running;
    task::set_syscall_times(&mut ti_mut.syscall_times);
    let start_time = task::start_time();
    let now = timer::get_time_ms();
    ti_mut.time = now - start_time;
    0
}

/// 本实验仅用于申请内存。syscall id = 222。成功返回 0，错误返回 -1。
///
/// `start` 要求按页对齐。port 低三位分别表示以下属性，其它位无效且必须为 0
///
/// - `port[2]`: read.
/// - `port[1]`: write.
/// - `port[0]`: exec.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    if len == 0 {
        return 0;
    }
    if start % PAGE_SIZE != 0 || port & !0x7 != 0 || port & 0x7 == 0 {
        return -1;
    }
    let map_perm = MapPermission::from_bits_truncate((port as u8) << 1) | MapPermission::U;
    if task::map_range(start, len, map_perm) {
        0
    } else {
        -1
    }
}

/// 取消映射。syscall id = 215。成功返回 0，错误返回 -1。
///
/// `start` 要求按页对齐。
///
/// FIXME: 注意，这里的实现是钻空子的。具体请看 `task::unmap_range` 的注释
pub fn sys_munmap(start: usize, len: usize) -> isize {
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    if task::unmap_range(start, len) {
        0
    } else {
        -1
    }
}

/// 功能：由当前进程 fork 出一个子进程。
/// 返回值：对于子进程返回 0，对于当前进程则返回子进程的 PID。
/// syscall ID：220
pub fn sys_fork() -> isize {
    let current_task = Processor::current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    let trap_ctx = new_task.inner_exclusive_access().trap_ctx();
    // 父进程调用了 sys_fork() 创建子进程，接收 sys_fork() 的返回值
    // 而子进程被创建之后，下次被调度时才会正式开始执行，修改其 `trap_ctx` 中保存的寄存器值即可模拟返回值
    trap_ctx.x[10] = 0;
    TaskManager::add_task(new_task);
    new_pid as isize
}

/// 功能：将当前进程的地址空间清空并加载一个特定的可执行文件，返回用户态后开始它的执行。
///
/// 参数：字符串 path 给出了要加载的可执行文件的名字；
///
/// 返回值：如果出错的话（如找不到名字相符的可执行文件）则返回 -1，否则不应该返回。
///
/// 注意：path 必须以 "\0" 结尾，否则内核将无法确定其长度
///
/// syscall ID：221
pub fn sys_exec(path: *const u8) -> isize {
    let user_satp = Processor::current_user_satp();
    let path = PageTable::translated_str(user_satp, path);
    if let Some(app_inode) = inode::open_file(&path, OpenFlags::RDONLY) {
        let task = Processor::current_task().unwrap();
        task.exec(&app_inode.read_all());
        0
    } else {
        -1
    }
}

/// 功能：新建子进程，使其执行目标程序。
///
/// 参数：字符串 path 给出了要加载的可执行文件的名字，必须以 "\0" 结尾
///
/// 返回值：成功返回子进程 id，否则返回 -1。
///
/// syscall ID：400
pub fn sys_spawn(path: *const u8) -> isize {
    let user_satp = Processor::current_user_satp();
    let path = PageTable::translated_str(user_satp, path);
    if let Some(app_inode) = inode::open_file(&path, OpenFlags::RDONLY) {
        let task = Processor::current_task().unwrap();
        task.spawn(&app_inode.read_all()) as isize
    } else {
        -1
    }
}

/// 功能：当前进程等待一个子进程变为僵尸进程，回收其全部资源并收集其返回值。
/// 参数：pid 表示要等待的子进程的进程 ID，如果为 -1 的话表示等待任意一个子进程；
/// exit_code 表示保存子进程返回值的地址，如果这个地址为 0 的话表示不必保存。
/// 返回值：如果要等待的子进程不存在则返回 -1；否则如果要等待的子进程均未结束则返回 -2；
/// 否则返回结束的子进程的进程 ID。
/// syscall id = 260
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    let task = Processor::current_task().unwrap();

    let mut inner = task.inner_exclusive_access();

    // 不存在这样的子进程
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.pid())
    {
        log::debug!("not such child: {}", pid);
        return -1;
    }

    if let Some((idx, _)) = inner.children.iter().enumerate().find(|(_, p)| {
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.pid())
    }) {
        let child = inner.children.swap_remove(idx);
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.pid();
        let exit_code = child.inner_exclusive_access().exit_code;
        *(PageTable::translated_mut(inner.user_satp(), exit_code_ptr)) = exit_code;
        found_pid as isize
    } else {
        -2
    }
}

pub fn sys_getpid() -> isize {
    Processor::current_task().unwrap().pid.0 as isize
}

// syscall ID：140
// 设置当前进程优先级为 prio
// 参数：prio 进程优先级，要求 prio >= 2
// 返回值：如果输入合法则返回 prio，否则返回 -1
pub fn sys_set_priority(priority: isize) -> isize {
    if priority <= 1 {
        return -1;
    }
    Processor::current_task()
        .unwrap()
        .inner_exclusive_access()
        .priority = priority as usize;
    priority
}
