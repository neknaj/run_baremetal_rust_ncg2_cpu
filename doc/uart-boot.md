# UART ブートローダーの ROM

`programs/uart-boot.S` はリセット PC `0x0` で実行する。UART の受信レジスタから64ビットの語を順に読み、RAM の `0x10000` から書き込む。全語を書き込んだ後、物理アドレス `0x10000` へ `JALR` で移る。アセンブリは104バイトで、128バイトの ROM に収まる。分岐の戻り先をレジスタへ保存する処理やスタックを使わない。

## データの受け渡し

| アドレス | 方向 | 値 |
| --- | --- | --- |
| `0x200000` | 読み出し | STATUS。ビット0が受信語の有効性、ビット1が受信エラー |
| `0x200008` | 読み出し | DATA。リトルエンディアンで組み立てた64ビットの受信語 |
| `0x200010` | 書き込み | ACK。64ビットの書き込み1回で受信語を消費する。書き込む値は0 |

CPU は STATUS を読み、エラービットを優先する。エラーがなく有効性が0なら同じ読み出しを繰り返す。受信側は DATA の読み出しで語を消費せず、ACK までその語を保持する。消費前に次の完全な語が届く可能性がある場合は、受信側で流量を制御するか、保持できないことをエラーとして記録する。エラーを含む語を有効にしない。

STATUS は上位62ビットが0なので、ペイロードの待機ループは値全体を比較できる。`0` なら再読出し、`1` なら DATA へ進み、`2` と `3` では停止する。ヘッダの上限検査が済んだあと、`t2` に正常値 `1` を入れる。ペイロードの各反復では従来の2回の `ANDI` を省き、DATA の読出し、RAM への書き込み、ACK の順を保つ。[UART MMIO の STATUS](uart-boot-mmio.md)

最初の語を符号なしの転送語数とする。許容範囲は `1..=0x8400`。範囲外ならヘッダを ACK せず、不正命令を置いた位置へ分岐し、CPU の停止経路に入る。受理した場合はヘッダを ACK し、後続の語ごとに RAM への `SD`、受信語の ACK、アドレスの8バイト増加、残り語数の減算を行う。上限まで転送した場合に最後に更新するアドレスは `0x51ff8`。不足した語は受信側がエラーを報告するまで待つ。

最後の `SD` からアプリへの `JALR` までに、ACK、ポインタ更新、語数更新、分岐、入口アドレスの設定を行う。接続側では、`JALR` の退役までに最後の RAM 書き込みが確定することを確認する。現在の CPU では命令ごとの退役間隔がメモリの更新遅延より長い。[CPU の退役と書き込み](firmware-execution.md#命令の完了と書き込み)

`::rv64::uart-boot-fetch` の PC 入力は物理アドレス。 `0x0..0x7f` のうち4バイト境界に位置する命令を返す。ROM の未使用領域は0で埋める。アプリの PC と RAM に接続するモジュールと、UART のビット受信・語組み立てはこの ROM の外に置く。

## RAM のアドレス

`::rv64::uart-ram-map` は8バイト境界を受け付ける。範囲の終点は含まない。

| 範囲 | 容量 | 用途 |
| --- | --- | --- |
| `0x10000..0x50000` | 256 KiB | アプリの機械語とデータ |
| `0x50000..0x52000` | 8 KiB | アプリ領域の続き |
| `0xfff00..0x100000` | 256 B | アプリへの要求 |
| `0x100000..0x104000` | 16 KiB | スタックと結果 |

256 KiB のバンクでは物理アドレスの下位18ビットを8で割って語の添字を求める。同じバンク内の異なる整列アドレスには異なる添字を与える。残りの3バンクでは先頭語の添字が0になる。検証器の `uart-ram-map` を Z3 に渡し、任意の64ビットアドレスでの範囲判定、添字、256 KiB バンク内で添字が重複しないことを照合する。

`::rv64::uart-ram` は CPU のフェッチとデータアクセスを同じポートで受け付ける。書き込みを指定する tick `t` までに、アドレスを32 tick 以上保持する。`t` の書き込み許可、アドレス、データは同じ入力の組として扱い、該当する語を `t + 2` に更新する。未配置または非整列なら更新しない。書き込み時、内蔵 RAM の登録出力は更新前の値を受け取る。読み出しアドレスを固定すれば、RAM の値は47 tick 以内に `read-data` へ届く。

`verify-uart-ram` は、任意の内部状態と、安定させた任意のアドレスに対する書き込み要求から、RAM の入口に届く許可とアドレス・データが参照仕様と一致することを検証する。VM の試験では、バンク境界の語、添字が一周する位置、無効なアドレス、連続した書き込みの確定時刻を確認する。

回路の照合では、Core が展開した配線を SMT-LIB へ写す検証器と Z3 を信頼する。CPU が任意の命令列で32 tick のアドレス保持条件を満たすことは、RAM 単体の検証範囲に含まれない。

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

検証器は展開済み ROM の全16語を独立に組み立てたバイト列と比べる。Z3 で全 PC のフェッチ可否と命令値、ヘッダの拒否条件、各語の書き込み先とカウンタ更新を確認する。STATUS の上位62ビットが0であることは展開済み MMIO を用いた検証で確認し、`uart_boot_bounds.smt2` では4通りの STATUS に対する旧分岐と新分岐の結果を独立した判定仕様と比べる。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
model=target/verification/uart-boot-fetch.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_uart_boot \
  -- target/verification/uart-boot.bin > "$model"
test "$(z3 -smt2 "$model")" = unsat
test "$(z3 -smt2 verification/uart_boot_bounds.smt2)" = "$(printf 'unsat\nunsat\nunsat')"
```

## UART からアプリを起動する結合試験

`uart-boot.toml` の入口は `::rv64::uart-system`。入力は UART の `rx` と64ビットの観測アドレスである。CPU が命令フェッチやメモリアクセスをしていない間に、観測アドレスがデータポートへ届く。公開出力は物理 PC、バスアドレス、読み出し値、受信の有効性とエラー、命令の完了と停止、書き込み許可の順に並ぶ。

次の試験は語数と2語の機械語を UART の 8N1 波形で送る。ROM が受信語を消費し、RAM から起動したアプリが観測先へ42を書き込むことを確かめる。全書き込みで直前32 tick のアドレスの保持を検査する。ペイロードの各書き込みが対応する ACK より先に来ることと、アプリの命令語が RAM の指定位置にあることも照合する。標準 VM と疎実行器の公開出力は、リセットから結果を読み出すまで毎 tick 比較し、最後にプログラム用 RAM の内容を照合する。

小アプリの起動試験では、ヘッダと2語の機械語を同じ順で送信し、各語を ACK したことを確認してから次の語を送った。旧 ROM（`29d24e6`）より、最後の1語に要する ACK 間隔が11,396 tick から9,324 tick に減った。これは2命令分の2,072 tick に当たる。RAM 上で起動したアプリの出力を観測するまでの総計は46,763 tick から38,475 tick に減った。総計には、変更によって最初のペイロードをポーリングする時点が変わる効果も含む。

```sh
ncg2 check --project uart-boot.toml
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --release --locked --manifest-path verification/Cargo.toml \
  --target "$host" --test uart_system
```
