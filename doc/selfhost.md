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

## RAM 上の入出力

入力と出力のアドレスは固定し、64ビット語はリトルエンディアンで並べる。入力は起動前に用意する。

| 物理アドレス | 内容 |
| --- | --- |
| `0x2bf00` | ソースのバイト数。上限128 |
| `0x2bf08` | NOR の入力2ビット。下位ビットから順に使う |
| `0x2bf10` | ソースの UTF-8 バイト列 |
| `0x2fff0` | 終了状態。成功は1 |
| `0x2fff8` | 成功時の情報。ビット0～7はノード数、ビット8は NOR の値、ビット16以上はヒープの予約済みバイト数 |

このリポジトリからビルドした ELF を QEMU で実行した。58バイトの入力を解析して19ノードを得た。2入力がともに0の NOR は1を返し、ヒープの予約済みサイズは2,016バイトだった。起動から終了まで14,233命令を実行した。同じ ELF の全4,096語を NCG2 ROM の展開結果と照合した。入力領域の初期設定と、NCG2 上での完走の照合を進めている。
