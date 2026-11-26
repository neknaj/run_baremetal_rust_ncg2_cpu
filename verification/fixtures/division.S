# 裸 RV64IM の除算命令を実行し、結果と次命令が読んだ値を RAM に記録する。

    .section .rodata, "a", @progbits
    .balign 8
expected_results:
    .include "verification/fixtures/division-results.inc"
expected_results_end:
    .equ result_count, 22
    .if (expected_results_end - expected_results) != 8 * result_count
    .error "expected exactly 22 division results"
    .endif

    .option norvc
    .option norelax
    .section .text.start, "ax", @progbits
    .global _start
_start:
    jal zero, run_division

    # 既存の UART 検証器と同じ停止位置に置く。
    .org 0x68
    .global boot_halt
boot_halt:
    jal zero, boot_halt

run_division:
    lui t0, 0x104
    addi t0, t0, -256
    addi t1, zero, -17
    addi t2, zero, 5

    div t3, t1, t2
    addi t4, t3, 10
    sd t3, 0(t0)
    sd t4, 8(t0)
    mul t3, t2, t4
    sd t3, 16(t0)
    rem t3, t1, t2
    sd t3, 24(t0)
    divu t3, t1, t2
    sd t3, 32(t0)
    mulhu t3, t1, t1
    sd t3, 40(t0)
    remu t3, t1, t2
    sd t3, 48(t0)

    # W の被除数と除数に0でない上位32ビットを与える。
    lui t1, 0x20
    slli t1, t1, 16
    addi t1, t1, -17
    addi t2, zero, 1
    slli t2, t2, 32
    addi t2, t2, 5
    divw t3, t1, t2
    sd t3, 56(t0)
    divuw t3, t1, t2
    sd t3, 64(t0)
    remw t3, t1, t2
    sd t3, 72(t0)
    remuw t3, t1, t2
    sd t3, 80(t0)

    # 符号なし W でも結果のビット31を上位32ビットに複製する。
    addi t2, zero, 1
    divuw t3, t1, t2
    sd t3, 88(t0)
    addi t2, zero, -1
    remuw t3, t1, t2
    sd t3, 96(t0)

    addi t1, zero, -17
    div t3, t1, zero
    sd t3, 104(t0)
    rem t3, t1, zero
    sd t3, 112(t0)
    addi t2, zero, 1
    slli t2, t2, 32
    divuw t3, t1, t2
    sd t3, 120(t0)
    remuw t3, t1, t2
    sd t3, 128(t0)

    addi t2, zero, 5
    div zero, t1, t2
    addi t3, zero, 9
    sd t3, 136(t0)

    # 64ビットと32ビットそれぞれの符号付き最小値を -1 で割る。
    addi t1, zero, 1
    slli t1, t1, 63
    addi t2, zero, -1
    div t3, t1, t2
    sd t3, 144(t0)
    rem t3, t1, t2
    sd t3, 152(t0)
    lui t1, 0x80000
    divw t3, t1, t2
    sd t3, 160(t0)
    remw t3, t1, t2
    sd t3, 168(t0)

    # RAM に保存した各語を期待値と照合する。失敗時は状態4と相違した語の添字を返す。
    lui t2, %hi(expected_results)
    addi t2, t2, %lo(expected_results)
    addi t1, t0, 0
    addi t3, zero, result_count
check_next:
    ld t4, 0(t1)
    ld t5, 0(t2)
    bne t4, t5, bad_result
    addi t1, t1, 8
    addi t2, t2, 8
    addi t3, t3, -1
    bne t3, zero, check_next
    addi t3, zero, result_count
    addi t4, zero, 1
    jal zero, publish
bad_result:
    addi t4, zero, result_count
    sub t3, t4, t3
    addi t4, zero, 4
publish:
    sd t3, 248(t0)
    sd t4, 240(t0)
    jal zero, boot_halt

    # セルフホスト用リンカースクリプトが予約する領域。入力要求は使わない。
    .section .request, "aw", @nobits
    .zero 144
    .section .result, "aw", @nobits
    .balign 8
    .zero 16
