# 起動用の命令実行回路

`::rv64::execute-boot` は、命令語、起動アドレス `0x10000` からの PC のバイトオフセット、読出し済みの `rs1`・`rs2` の値を受け取る。32 ビット命令をフェッチできたときに使う。演算結果とアドレスの計算は 64 ビットで桁あふれを捨てる。

| 命令 | 選択条件 | 有効時の処理 |
| --- | --- | --- |
| ADDI | opcode が `0x13`、funct3 が `0` | 符号拡張した I 即値と `rs1` の和を `rd` に書き込む |
| SLLI | opcode が `0x13`、funct3 が `1`、命令ビット26～31がすべて0 | `rs1` を命令ビット20～25が表す量だけ左にシフトし、`rd` に書き込む |
| LUI | opcode が `0x37` | 符号拡張した U 即値を `rd` に書き込む |
| SD | opcode が `0x23`、funct3 が `3`、`rs1 + imm-s` が8バイト境界 | 実アドレス `rs1 + imm-s` に `rs2` を書き込む |
| JAL | opcode が `0x6f`、`pc-offset + imm-j` が4バイト境界 | `pc-offset + imm-j` に進み、実アドレス `0x10000 + pc-offset + 4` を `rd` に書き込む |

有効な JAL 以外の命令では、PC の相対位置を4バイト進める。未対応の符号化やこの表の整列条件を満たさない命令では `valid` を0にし、PC を保持して両方の書き込みを抑止する。

`register-index` と `register-data` は `register-write` が1のときに使う。レジスタファイルが `x0` への書き込みを抑止する。RAM への書き込みは `store-write` が1で、実アドレスが接続先の RAM の範囲内にある場合に確定する。ジャンプ先の命令が実際に読めることはフェッチ回路で検査する。

[RISC-V の基本命令形式](https://docs.riscv.org/reference/isa/v20260120/unpriv/rv32.html#_base_instruction_formats) · [RV64I の整数命令](https://docs.riscv.org/reference/isa/unpriv/rv64.html)
