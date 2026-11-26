# 0x103f00 から8バイトごとに格納する22語。値の後ろは命令または検査条件。
    .quad 0xfffffffffffffffd  # DIV -17, 5
    .quad 0x0000000000000007  # 直後の ADDI が DIV の結果を使用
    .quad 0x0000000000000023  # MUL 5, 7
    .quad 0xfffffffffffffffe  # REM -17, 5
    .quad 0x333333333333332f  # DIVU (2^64-17), 5
    .quad 0xffffffffffffffde  # MULHU (2^64-17), (2^64-17)
    .quad 0x0000000000000004  # REMU (2^64-17), 5
    .quad 0xfffffffffffffffd  # DIVW -17, 5
    .quad 0x000000003333332f  # DIVUW (2^32-17), 5
    .quad 0xfffffffffffffffe  # REMW -17, 5
    .quad 0x0000000000000004  # REMUW (2^32-17), 5
    .quad 0xffffffffffffffef  # DIVUW の結果ビット31を符号拡張
    .quad 0xffffffffffffffef  # REMUW の結果ビット31を符号拡張
    .quad 0xffffffffffffffff  # DIV -17, 0
    .quad 0xffffffffffffffef  # REM -17, 0
    .quad 0xffffffffffffffff  # DIVUW の除数の下位32ビットが0
    .quad 0xffffffffffffffef  # REMUW の除数の下位32ビットが0
    .quad 0x0000000000000009  # x0 宛ての DIV の直後に x0 を読み出す
    .quad 0x8000000000000000  # DIV MIN64, -1
    .quad 0x0000000000000000  # REM MIN64, -1
    .quad 0xffffffff80000000  # DIVW MIN32, -1
    .quad 0x0000000000000000  # REMW MIN32, -1
