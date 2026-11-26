.section .text.start, "ax"
.globl _start
.type _start, @function
_start:
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    la sp, __stack_top
    la t0, __bss_start
    la t1, __bss_end
1:
    bgeu t0, t1, 2f
    sd zero, 0(t0)
    addi t0, t0, 8
    j 1b
2:
    addi sp, sp, -16
    li t0, 0x7a650c691bd3482f
    sd t0, 8(sp)
    ld a0, 8(sp)
    call rust_main
    addi sp, sp, 16

.globl boot_halt
.type boot_halt, @function
boot_halt:
    j boot_halt
