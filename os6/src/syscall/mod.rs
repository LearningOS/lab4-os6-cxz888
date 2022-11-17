use crate::task::incr_syscall_times;

mod fs;
mod process;

pub const SYSCALL_OPEN: usize = 56;
pub const SYSCALL_CLOSE: usize = 57;
pub const SYSCALL_READ: usize = 63;
pub const SYSCALL_WRITE: usize = 64;
pub const SYSCALL_UNLINKAT: usize = 35;
pub const SYSCALL_LINKAT: usize = 37;
pub const SYSCALL_FSTAT: usize = 80;
pub const SYSCALL_EXIT: usize = 93;
// pub const SYSCALL_SLEEP: usize = 101;
pub const SYSCALL_YIELD: usize = 124;
pub const SYSCALL_GETTIMEOFDAY: usize = 169;
pub const SYSCALL_GETPID: usize = 172;
// pub const SYSCALL_GETTID: usize = 178;
pub const SYSCALL_FORK: usize = 220;
pub const SYSCALL_EXEC: usize = 221;
pub const SYSCALL_WAITPID: usize = 260;
pub const SYSCALL_SET_PRIORITY: usize = 140;
pub const SYSCALL_MUNMAP: usize = 215;
pub const SYSCALL_MMAP: usize = 222;
pub const SYSCALL_SPAWN: usize = 400;
// pub const SYSCALL_MAIL_READ: usize = 401;
// pub const SYSCALL_MAIL_WRITE: usize = 402;
// pub const SYSCALL_DUP: usize = 24;
// pub const SYSCALL_PIPE: usize = 59;
pub const SYSCALL_TASK_INFO: usize = 410;
// pub const SYSCALL_THREAD_CREATE: usize = 460;
// pub const SYSCALL_WAITTID: usize = 462;
// pub const SYSCALL_MUTEX_CREATE: usize = 463;
// pub const SYSCALL_MUTEX_LOCK: usize = 464;
// pub const SYSCALL_MUTEX_UNLOCK: usize = 466;
// pub const SYSCALL_SEMAPHORE_CREATE: usize = 467;
// pub const SYSCALL_SEMAPHORE_UP: usize = 468;
// pub const SYSCALL_ENABLE_DEADLOCK_DETECT: usize = 469;
// pub const SYSCALL_SEMAPHORE_DOWN: usize = 470;
// pub const SYSCALL_CONDVAR_CREATE: usize = 471;
// pub const SYSCALL_CONDVAR_SIGNAL: usize = 472;
// pub const SYSCALL_CONDVAR_WAIT: usize = 473;

pub fn syscall(syscall_id: usize, args: [usize; 4]) -> isize {
    incr_syscall_times(syscall_id);
    match syscall_id {
        SYSCALL_READ => fs::sys_read(args[0], args[1] as _, args[2]),
        SYSCALL_WRITE => fs::sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_OPEN => fs::sys_open(args[1] as _, args[2] as u32),
        SYSCALL_LINKAT => fs::sys_linkat(-100, args[1] as _, -100, args[3] as _, 0),
        SYSCALL_UNLINKAT => fs::sys_unlinkat(-100, args[1] as _, 0),
        SYSCALL_FSTAT => fs::sys_fstat(args[0], args[1] as _),
        SYSCALL_CLOSE => fs::sys_close(args[0]),
        SYSCALL_EXIT => process::sys_exit(args[0] as i32),
        SYSCALL_YIELD => process::sys_yield(),
        SYSCALL_GETPID => process::sys_getpid(),
        SYSCALL_SET_PRIORITY => process::sys_set_priority(args[0] as isize),
        SYSCALL_GETTIMEOFDAY => process::sys_get_time(args[0] as _, args[1]),
        SYSCALL_TASK_INFO => process::sys_task_info(args[0] as _),
        SYSCALL_MMAP => process::sys_mmap(args[0], args[1], args[2]),
        SYSCALL_MUNMAP => process::sys_munmap(args[0], args[1]),
        SYSCALL_FORK => process::sys_fork(),
        SYSCALL_EXEC => process::sys_exec(args[0] as _),
        SYSCALL_SPAWN => process::sys_spawn(args[0] as _),
        SYSCALL_WAITPID => process::sys_waitpid(args[0] as isize, args[1] as _),
        _ => {
            log::error!("Unsupported syscall_id: {}", syscall_id);
            process::sys_exit(-1);
        }
    }
}
