# 起動用の命令実行回路

`::rv64::execute-boot` は、命令語、起動アドレス `0x10000` からの PC のバイトオフセット、読出し済みの `rs1`・`rs2` の値を受け取る。32 ビット命令をフェッチできたときに使う。演算結果とアドレスの計算は 64 ビットで桁あふれを捨てる。

| 命令 | 選択条件 | 有効時の処理 |
| --- | --- | --- |
| ADDI | opcode が `0x13`、funct3 が `0` | 符号拡張した I 即値と `rs1` の和を `rd` に書き込む |
| SLLI | opcode が `0x13`、funct3 が `1`、命令ビット26～31がすべて0 | `rs1` を命令ビット20～25が表す量だけ左にシフトし、`rd` に書き込む |
| LUI | opcode が `0x37` | 符号拡張した U 即値を `rd` に書き込む |
| SD | opcode が `0x23`、funct3 が `3`、`rs1 + imm-s` が8バイト境界 | 実アドレス `rs1 + imm-s` に `rs2` を書き込む |
| JAL | opcode が `0x6f`、`pc-offset + imm-j` が4バイト境界 | `pc-offset + imm-j` に進み、実アドレス `0x10000 + pc-offset + 4` を `rd` に書き込む |

有効な JAL 以外の命令では PC の相対位置を4バイト進める。未対応の符号化やこの表の整列条件を満たさない命令では `valid` を0にし、PC を保持して両方の書き込みを抑止する。

`register-index` と `register-data` は `register-write` が1のときに使う。レジスタファイルが `x0` への書き込みを抑止する。RAM への書き込みは `store-write` が1で、実アドレスが接続先の RAM の範囲内にある場合に確定する。ジャンプ先の命令が実際に読めることはフェッチ回路で検査する。

[RISC-V の基本命令形式](https://docs.riscv.org/reference/isa/v20260120/unpriv/rv32.html#_base_instruction_formats) · [RV64I の整数命令](https://docs.riscv.org/reference/isa/unpriv/rv64.html)

## 起動回路の tick

`::rv64::boot-machine` は reset から起動を始める。PC は起動アドレス `0x10000` からのオフセットを保持する。1命令の間に2つのオペランドを単一ポートのレジスタファイルから順に取り込み、実行結果が確定するまで待つ。

有効な命令では `commit` が1 tick だけ立つ。そのフレームの `pc-offset` は実行した命令を指し、次の tick に PC とストア先の RAM が更新される。レジスタへの書き戻しはレジスタファイルの入力パイプラインを通る。未対応の命令や範囲外のストアでは `stopped` が立ち、PC と記憶を更新せず、次の命令を開始しない。reset 直後の出力はすべて0になる。

RAM は `0x2c000` から `0x2ffff` までを8バイトの語で保持する。1ポートをストア先と観測先で共有する。ある tick に読み出した添字が `probe-response` に、同じ添字の更新前の値が `probe-data` に現れる。RAM とアドレスを保持するラッチが同じ配線を読むため、両出力は同じ語を指す。指定した語を観測するときは `probe-response` が入力した `probe-address` と一致したフレームを使う。実行設定にある公開 RAM `boot-ram` からは最後に確定した tick の記憶内容を直接読める。[NCG2 の状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html#状態素子)

ROM の6命令の `commit` は tick 1031、2061、3091、4121、5151、6181 に現れる。ストアは tick 5152 に `0x2fff8` を `0x0000000100000003` に更新する。最後の JAL は PC オフセット20に戻る。

## 検査

リポジトリのルートで、ホスト向けに検査を実行する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test boot_machine
```

結合テストは Core が組み立てた回路を tick ごとに進め、命令完了、物理 RAM の更新、読み出し応答を照合する。

`execute-boot` は展開済み NOR 回路を Z3 で検証する。任意の命令語、PC オフセット、2つのオペランドに対して、有効ビット、次の PC オフセット、書き込みの許可が表の仕様と一致する。書き込みを許可した場合は対象と値も一致する。整列違反や未対応の命令で PC と書き込みが抑止されることもこの比較に含む。命令フェッチと順序回路の tick を含めた回路全体の形式証明は今後の作業である。
