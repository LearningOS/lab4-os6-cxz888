mod context;

use crate::{
    config::{TRAMPOLINE, TRAP_CONTEXT},
    syscall::syscall,
    task::{self, Processor},
    timer,
};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, stval, stvec,
};

pub use context::TrapContext;

core::arch::global_asm!(include_str!("trap.S"));

pub fn init() {
    set_kernel_trap_entry();
}

#[no_mangle]
pub fn trap_handler() -> ! {
    set_kernel_trap_entry();
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            let mut ctx = Processor::current_trap_ctx();
            ctx.sepc += 4;
            let result = syscall(ctx.x[17], [ctx.x[10], ctx.x[11], ctx.x[12], ctx.x[13]]) as usize;
            ctx = Processor::current_trap_ctx();
            ctx.x[10] = result;
        }
        Trap::Exception(Exception::StoreFault)
        | Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::LoadPageFault) => {
            log::error!("[kernel] PageFault in application, core dumped.");
            task::exit_current_and_run_next(-2);
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            log::error!("[kernel] IllegalInstruction in application, core dumped.");
            task::exit_current_and_run_next(-3);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            timer::set_next_trigger();
            task::suspend_current_and_run_next();
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
    trap_return()
}

#[no_mangle]
pub fn trap_return() -> ! {
    log::trace!("trap return");
    set_user_trap_entry();
    let trap_ctx_ptr = TRAP_CONTEXT;
    let user_satp = Processor::current_user_satp();
    extern "C" {
        fn __alltraps();
        fn __restore();
    }
    let restore_va = __restore as usize - __alltraps as usize + TRAMPOLINE;

    unsafe {
        core::arch::asm!(
            "fence.i",
            "jr {restore_va}",
            restore_va = in(reg) restore_va,
            in("a0") trap_ctx_ptr,
            in("a1") user_satp,
            options(noreturn)
        );
    }
}

// 这里也许会导致嵌套中断？
pub fn enable_timer_interrupt() {
    unsafe { sie::set_stimer() }
}

#[no_mangle]
pub fn trap_from_kernel() -> ! {
    panic!("a trap from kernel!");
}

fn set_kernel_trap_entry() {
    unsafe { stvec::write(trap_from_kernel as usize, TrapMode::Direct) }
}

fn set_user_trap_entry() {
    unsafe { stvec::write(TRAMPOLINE, TrapMode::Direct) }
}
