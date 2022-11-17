use crate::trap::trap_return;

#[repr(C)]
#[derive(Clone)]
pub struct TaskContext {
    ra: usize,
    sp: usize,
    // callee-saved 寄存器
    s: [usize; 12],
}

impl core::fmt::Debug for TaskContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ra: {:#x}, sp: {:#x}", self.ra, self.sp)
    }
}

impl TaskContext {
    pub const fn zero_init() -> Self {
        Self {
            ra: 0,
            sp: 0,
            s: [0; 12],
        }
    }
    pub fn goto_trap_return(kernel_stack_ptr: usize) -> Self {
        Self {
            ra: trap_return as usize,
            sp: kernel_stack_ptr,
            s: [0; 12],
        }
    }
}
