# NCG2 で作る RV64IM CPU

Neknaj Circuit Game 2 で RV64IM の CPU を組み、ベアメタル Rust のプログラムを動かす。最終的には、その CPU 上でも NCG2 Core で回路を評価できるようにする。

## Rust の起動プログラムを実行する

`firmware.toml` は、リポジトリに同梱した Rust ELF のイメージを読む NCG2 回路を起動する。実行に Python や QEMU は不要で、`ncg2` の `run` で tick を進める。

```sh
ncg2 check --project firmware.toml
ncg2 run --project firmware.toml --ticks 90000 | tail -n 1
```

最終行のビット列は PC オフセット・終了状態・積を64ビットずつ、続いて `finished` と `stopped` を1ビットずつ並べる。このプログラムの結果は PC オフセット `0x64`、終了状態 `1`、積 `0x0000_0005_0000_0003`、`finished=1`、`stopped=0` である。各ポートは最下位ビットから表示する。[実行回路と検証手順](doc/firmware-execution.md)

## 起動 ROM を実行する

`circuitgame2.toml` のエントリは起動 ROM、レジスタ、実行回路、RAM をつないだ `::rv64::boot-machine` である。リセット後に tick を進めると、ROM の6命令が順に実行される。

```sh
ncg2 check --project circuitgame2.toml
ncg2 run --project circuitgame2.toml --input 11111111111
```

この `run` は1 tick 進める。入力の11ビットは RAM の観測用アドレスで、すべて1にすると末尾の語を指定する。全6命令を実行する結合テストでは、物理アドレス `0x2fff8` に `0x0000000100000003` が書かれ、最後の JAL が同じ PC に戻ることを確認する。実行時の信号と検査手順は[起動回路](doc/boot-execution.md)を参照。

この構成は固定6命令の回路を試すためのものである。

## 命令デコーダを試す

`circuit/decode.ncg2` は、32ビット命令語から opcode、レジスタ番号、funct3、funct7 と、64ビットに符号拡張した I・S・B・U・J 形式の即値を取り出す。命令形式の選択、レジスタへの書き戻し、メモリアクセスは制御回路で扱う。

```sh
ncg2 check circuit/decode.ncg2
ncg2 eval --func ::rv64::decode --input 00000000000000000000000000000001 circuit/decode.ncg2
```

ビット列は各ポートの最下位ビットから並べる。この例では命令語のビット31だけを立て、即値の上位が1になることを確認できる。エンコードは [RISC-V の仕様](https://docs.riscv.org/reference/isa/v20260120/unpriv/rv32.html#_base_instruction_formats) に従う。

## PC の保持を試す

`circuit/program-counter.ncg2` は、起動アドレス `0x10000` からの64ビットのオフセット `pc-offset` を保持する。リセット後はオフセットが0になる。フェッチ先は `0x10000 + pc-offset` を64ビットに収めた値である。

```sh
ncg2 check circuit/decode.ncg2 circuit/program-counter.ncg2
ncg2 run --entry ::rv64::program-counter --input 10000000000000000000000000000000000000000000000000000000000000001 --input 00000000000000000000000000000000000000000000000000000000000000000 circuit/decode.ncg2 circuit/program-counter.ncg2
```

先頭行はリセット直後の出力で、後続の行は各 tick の確定後の出力。最初の入力は `next-offset = 1` と `write-enable = 1` を指定する。次の tick では許可ビットが0なのでオフセット1を保持する。出力を最下位ビットから読むと、オフセットは順に `0, 1, 1` となる。ラッチの更新と観測は [NCG2 の tick と状態素子の仕様](https://neknaj.github.io/circuitgame2/docs/simulation.html#状態素子) に従う。

## レジスタファイルを試す

`circuit/register-file.ncg2` の `::rv64::register-file` は5ビットの `index`、64ビットの `write-data`、1ビットの `write-enable` を受け付ける。入力した tick を `t` とすると、`t + 7` の出力 `read-data` はその入力が指定したレジスタの更新前の値である。書き込みを許可した場合は同じ tick に新しい値も確定する。レジスタ `x0` への書き込みは無効であり、読み出しは常に0。tick 0 から7の出力は0であり、最初の要求の結果は tick 8 に現れる。

アドレスと書き込み入力は各 tick に1回ずつ与えられる。有効な書き込みの直後に同じレジスタを読むと、直前の書き込みが反映される。例えば tick 1 に `x1` へ42を書き、tick 2 に `x1` を読むと、tick 8 の出力は0、tick 9 の出力は42になる。後段の状態素子が tick 9 の読み出し結果を使うのは tick 10 からである。

```sh
ncg2 check circuit/decode.ncg2 circuit/register-file.ncg2
ncg2 run --entry ::rv64::register-file circuit/decode.ncg2 circuit/register-file.ncg2
```

`run` に `--input` を追加すると、1入力につき1 tick 進む。各ポートのビットは最下位から並べる。更新と観測は [NCG2 の tick と状態素子の仕様](https://neknaj.github.io/circuitgame2/docs/simulation.html#状態素子) に従う。

## 起動 ELF を作る

```sh
rustup toolchain install nightly --profile minimal --component rust-src
cargo +nightly build --release --locked -Z build-std=core
```

成果物は `target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot`。ELF のロード領域を配置したあと、`0x10000` から実行する。起動コードがスタックを用意して BSS を消去し、Rust の `rust_main` を呼ぶ。`boot_halt` 到達後に RAM の `0x2fff0` が成功値 `1`、`0x2fff8` が `0x0000_0005_0000_0003` なら検査成功。A・C・F・D を無効にした QEMU ユーザーモードで確認した。

## ELF を回路のメモリに載せる

`circuit/firmware-rom.ncg2` は、起動 ELF の命令と読み取り専用データを格納する。通常の実行は NCG2 のファイルを使う。Rust を変更したときに、生成済み ROM を更新する。

```sh
python tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot \
  circuit/firmware-rom.ncg2 --format ncg2-rom
```

フェッチと、ROM・BSS・スタックの読み書き回路は[起動 ELF のメモリ配置](doc/firmware-memory.md)に記す。

## NCG2M のプログラムデータ

`programs/boot.ncg2m` は、RAM の初期内容を64ビット語の添字と値で表す。省略した語の値は0。Rust を変更したときは、開発時に ELF から更新する。

```sh
python tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot programs/boot.ncg2m --format ncg2m
ncg2 check programs/boot.ncg2m
```

実行用の NCG2 ROM を、この NCG2M データから展開する経路は作業中である。

## Core を組み込んだ ELF

固定版の Core をベアメタル Rust から呼び、RAM に置いた入力を解析・評価する。[ビルドと RAM の入出力](doc/selfhost.md)

## 起動回路の部品を検証する

検証ツールは Core が展開した NOR ネットリストを読み、参照仕様と出力が異なる入力が存在するかを Z3 に問い合わせる。対象は64ビットの算術・論理・比較、RAM のアドレス変換、[起動用の5種類の命令](doc/boot-execution.md)、[PC 相対の命令と条件分岐](doc/control-execution.md)である。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for circuit in add shift-left xor compare-unsigned less-than-unsigned \
  boot-ram-address execute-boot execute-control execute-load execute-logic \
  firmware-dispatch mul-round; do
  model="target/verification/$circuit.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification -- "$circuit" > "$model"
  result=$(z3 -smt2 "$model")
  printf '%s\n' "$result"
  test "$result" = unsat
done
```

各結果が `unsat` なら、その回路の出力は仕様の対象となる全入力で参照仕様と一致する。
