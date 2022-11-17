.altmacro
.macro SAVE_SN n
    sd s\n, (\n+2)*8(a0)
.endm
.macro LOAD_SN n
    ld s\n, (\n+2)*8(a1)
.endm
    .section .text
    .global __switch
__switch:
    # __switch(
    #     current_task_ctx_ptr: *mut TaskContext,
    #     next_task_ctx_ptr: *const TaskContext
    # )
    sd ra, (a0)
    sd sp, 8(a0)
    .set n, 0
    .rept 12
        SAVE_SN %n
        .set n, n+1
    .endr
    ld ra, (a1)
    ld sp, 8(a1)
    .set n, 0
    .rept 12
        LOAD_SN %n
        .set n, n+1
    .endr
    ret

