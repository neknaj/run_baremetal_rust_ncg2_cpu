# UART ブートローダーの ROM

`programs/uart-boot.S` はリセット PC `0x0` で実行する。UART の受信レジスタから64ビットの語を順に読み、RAM の `0x10000` から書き込む。全語を書き込んだ後、物理アドレス `0x10000` へ `JALR` で移る。アセンブリは108バイトで、128バイトの ROM に収まる。分岐の戻り先をレジスタへ保存する処理やスタックを使わない。

## データの受け渡し

| アドレス | 方向 | 値 |
| --- | --- | --- |
| `0x200000` | 読み出し | STATUS。ビット0が受信語の有効性、ビット1が受信エラー |
| `0x200008` | 読み出し | DATA。リトルエンディアンで組み立てた64ビットの受信語 |
| `0x200010` | 書き込み | ACK。64ビットの書き込み1回で受信語を消費する。書き込む値は0 |

CPU は STATUS を読み、エラービットを優先する。エラーがなく有効性が0なら同じ読み出しを繰り返す。受信側は DATA の読み出しで語を消費せず、ACK までその語を保持する。消費前に次の完全な語が届く可能性がある場合は、受信側で流量を制御するか、保持できないことをエラーとして記録する。エラーを含む語を有効にしない。

最初の語を符号なしの転送語数とする。許容範囲は `1..=0x8400`。範囲外ならヘッダを ACK せず、不正命令を置いた位置へ分岐し、CPU の停止経路に入る。受理した場合はヘッダを ACK し、後続の語ごとに RAM への `SD`、受信語の ACK、アドレスの8バイト増加、残り語数の減算を行う。上限まで転送した場合に最後に更新するアドレスは `0x51ff8`。不足した語は受信側がエラーを報告するまで待つ。

最後の `SD` からアプリへの `JALR` までに、ACK、ポインタ更新、語数更新、分岐、入口アドレスの設定を行う。接続側では、`JALR` の退役までに最後の RAM 書き込みが確定することを確認する。現在の CPU では命令ごとの退役間隔がメモリの更新遅延より長い。[CPU の退役と書き込み](firmware-execution.md#命令の完了と書き込み)

`::rv64::uart-boot-fetch` の PC 入力は物理アドレス。 `0x0..0x7f` のうち4バイト境界に位置する命令を返す。ROM の未使用領域は0で埋める。アプリの PC と RAM に接続するモジュールと、UART のビット受信・語組み立てはこの ROM の外に置く。

## ROM の更新と検証

通常の実行では `circuit/uart-boot-rom.ncg2` を使う。アセンブリを変更したときは GNU binutils で組み立て直し、命令バイトを ROM に変換する。

```sh
set -e
mkdir -p target/verification
riscv64-linux-gnu-as -march=rv64i -mabi=lp64 -mno-relax \
  programs/uart-boot.S -o target/verification/uart-boot.o
riscv64-linux-gnu-ld --no-relax -Ttext=0 -e _start \
  target/verification/uart-boot.o -o target/verification/uart-boot.elf
riscv64-linux-gnu-objcopy -O binary --only-section=.text \
  target/verification/uart-boot.elf target/verification/uart-boot.bin
python tools/uart_boot_rom.py target/verification/uart-boot.bin circuit/uart-boot-rom.ncg2
```

検証器は展開済み ROM の全16語を独立に組み立てたバイト列と比べる。Z3 で全 PC のフェッチ可否と命令値、ヘッダの拒否条件、各語の書き込み先とカウンタ更新を確認する。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
model=target/verification/uart-boot-fetch.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_uart_boot \
  -- target/verification/uart-boot.bin > "$model"
test "$(z3 -smt2 "$model")" = unsat
test "$(z3 -smt2 verification/uart_boot_bounds.smt2)" = "$(printf 'unsat\nunsat')"
```
