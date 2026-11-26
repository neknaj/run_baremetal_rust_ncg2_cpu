    .option norvc
    .option norelax
    .section .text.boot, "ax", @progbits
    .balign 4
    .globl _start
    .type _start, @function

_start:
    lui t0, 0x200
    lui t1, 0x10
    lui t2, 0x8
    addi t2, t2, 0x400

    # 先頭の1語は転送する語の数。エラーを ready より先に判定する。
header:
    ld t3, 0(t0)
    andi t6, t3, 2
    bne t6, zero, boot_error
    andi t3, t3, 1
    beq t3, zero, header
    ld t4, 8(t0)
    beq t4, zero, boot_error
    bltu t2, t4, boot_error
    sd zero, 16(t0)

    # ヘッダの語数検査が終わったので、t2 を正常な STATUS の値1に切り替える。
    addi t2, zero, 1

    # DATA は ACK を書くまで同じ語を返す。SRAM への書き込み要求を発行した後で、受信語を消費する。
payload:
    ld t3, 0(t0)
    beq t3, zero, payload
    bne t3, t2, boot_error
    ld t5, 8(t0)
    sd t5, 0(t1)
    sd zero, 16(t0)
    addi t1, t1, 8
    addi t4, t4, -1
    bne t4, zero, payload

    # 最後の転送語を書き込んだ後で、アプリの先頭へ移る。
    lui t1, 0x10
    jalr zero, 0(t1)

boot_error:
    # CPU の不正命令経路で停止し、以降の RAM 更新を抑止する。
    .4byte 0
    .size _start, . - _start
