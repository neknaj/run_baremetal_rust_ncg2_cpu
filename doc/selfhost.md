# Core を組み込んだ RV64IM ELF

`selfhost/` には固定版の NCG2 Core を呼ぶベアメタル Rust プログラムを置く。RAM にある1件の要求から UTF-8 のソースを取り出し、厳格 S 式パーサーで解析する。次に Core で2入力の NOR を評価し、解析結果のノード数と評価結果を RAM に書く。評価に使う1ゲートのネットリストはこのプログラムが用意する。

## ELF のビルド

リポジトリのルートで実行する。vstd の準備には Python 3.12 以降と Git が必要である。

```sh
cd selfhost
python prepare_vstd.py target/vendor/vstd
RUSTC_BOOTSTRAP=1 cargo build --release --locked -Z build-std=core,alloc
```

ターゲットは `riscv64im-unknown-none-elf`、Rust は1.99.1。Core の依存先は Cargo.toml とロックファイルで固定する。vstd の原本とパッチはそれぞれ SHA-256 で検査する。パッチはアトミック機能を必要とするモジュールを、ターゲットに対応する機能がある場合だけ組み込む。プログラムは単一ハートで動き、8 KiB のヒープを使う。ビルドで設定する `RUSTC_BOOTSTRAP` はこのターゲット向けに標準ライブラリの `core` と `alloc` をビルドするために使う。

成果物は `selfhost/target/riscv64im-unknown-none-elf/release/ncg2-rv64im-core-selfhost`。リポジトリのルートに戻り、ELF を NCG2 の ROM に変換する。

```sh
cd ..
python tools/elf_image.py selfhost/target/riscv64im-unknown-none-elf/release/ncg2-rv64im-core-selfhost \
  selfhost/target/core-firmware.ncg2 --format ncg2-rom --rom-layout core
ncg2 check --project selfhost.toml
```

`selfhost.toml` はこの ROM と、セルフホスト用の RAM 配線を選ぶ。[ELF のメモリ配置](firmware-memory.md#セルフホスト用の物理アドレス) · [セルフホスト用のメモリ回路](selfhost-memory.md)

## NCG2 での実行

`selfhost.toml` を使うと、起動時に回路が入力要求を RAM に書く。ELF から ROM を生成した後は `ncg2` だけで実行できる。

```sh
ncg2 run --project selfhost.toml --ticks 16000000 | tail -n 1
```

出力は PC オフセット、終了状態、解析と実行の結果を64ビットずつ並べ、その後ろに `finished` と `stopped` を1ビットずつ並べる。各ポートの最下位ビットが先頭にある。QEMU で同じ入力を与えたときの終了状態は `1`、結果は `0x07e00113`。PC は終了時に `0x10068` の自己ループへ入る。

入力の18語は tick 693 に RAM へ揃い、tick 723 に CPU からの書き込みを受け付ける。CPU が最初の命令を完了するのは tick 1037 である。[起動時の入力回路](selfhost-preload.md)

## RAM 上の入出力

入力と出力のアドレスは固定し、64ビット語はリトルエンディアンで並べる。

| 物理アドレス | 内容 |
| --- | --- |
| `0x2bf00` | ソースのバイト数。上限128 |
| `0x2bf08` | NOR の入力2ビット。下位ビットから順に使う |
| `0x2bf10` | ソースの UTF-8 バイト列 |
| `0x2fff0` | 終了状態。成功は1 |
| `0x2fff8` | 成功時の情報。ビット0～7はノード数、ビット8は NOR の値、ビット16以上はヒープの予約済みバイト数 |

このリポジトリからビルドした ELF を QEMU で実行した。58バイトの入力を解析して19ノードを得た。2入力がともに0の NOR は1を返し、ヒープの予約済みサイズは2,016バイトだった。起動から終了まで14,233命令を実行した。同じ ELF の全4,096語を NCG2 ROM の展開結果と照合した。

## QEMU の PC 遷移との照合

QEMU の `-one-insn-per-tb -d exec,nochain` で、起動から `boot_halt` に到達するまでを記録する。QEMU の入力領域には NCG2 の入力回路と同じ要求を設定する。次の検証コマンドは Core の標準 VM を使い、PC が変わるたびに参照と照合する。同じ PC で連続した命令完了は1回の遷移としても現れない。終了を検知したら RAM から取得した終了状態と結果を上記の期待値と照合する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_selfhost_run \
  -- selfhost.toml target/verification/qemu-trace.log 16000000
```
