# 自作のベアメタル Rust を動かす

リポジトリにある起動コードとリンカスクリプトを使い、Rust の計算を NCG2 上の RV64IM CPU で実行する。最初は付属の小さな例で `42` が返るところまで確かめ、その後に計算を書き換える。

## Rust から ELF を作る

リポジトリのルートで実行する。あらかじめ Python 3.10 以降 と [NCG2 CLI](https://github.com/neknaj/circuitgame2-cli#インストールする)を用意する。

```sh
rustup toolchain install nightly --profile minimal --component rust-src
cargo +nightly build --release --locked -Z build-std=core --example firmware_42
```

`examples/firmware_42.rs` の `rust_main` が実行するプログラムである。起動コードがスタックを用意して BSS をゼロにし、`rust_main` を呼ぶ。この例は計算結果を `0x2fff8` に書いてから、非ゼロの終了状態を `0x2fff0` に書く。CPU の観測用回路はこの順で書かれた2語を読み取る。関数から戻ると起動コードの自己ループに入る。[例のソース](https://github.com/neknaj/run_baremetal_rust_ncg2_cpu/blob/main/examples/firmware_42.rs)

## ELF を読み込ませる

ELF を NCG2 の ROM に変換する。

```sh
mkdir -p target/custom
python3 tools/elf_image.py target/riscv64im-unknown-none-elf/release/examples/firmware_42 \
  target/custom/firmware-rom.ncg2 --format ncg2-rom
test -e firmware-local.toml || cp firmware.toml firmware-local.toml
```

`firmware-local.toml` の `sources` で、`"circuit/firmware-rom.ncg2"` を `"target/custom/firmware-rom.ncg2"` に置き換える。相対パスは設定ファイルのあるディレクトリから解決するので、設定ファイルはリポジトリのルートに置く。

```sh
ncg2 check --project firmware-local.toml
ncg2 run --project firmware-local.toml --ticks 90000 | tail -n 1
```

最後の行には tick 番号とビット列が並ぶ。ビット列は、PC オフセット、終了状態、計算結果を各64ビットで並べ、末尾に `finished` と `stopped` を各1ビットで付ける。各値は最下位ビットから出力する。この例の終了状態は `1`、計算結果は `42`、末尾は `10` になる。`finished=0` のままなら実行 tick を増やす。`stopped=1` なら、未対応の命令やメモリアクセスなどで CPU が停止している。

## 自分のプログラムに置き換える

最初は `examples/firmware_42.rs` の `rust_main` にある計算だけを書き換え、ビルドと ROM の生成をやり直す。観測用回路をそのまま使う場合は、結果を書いてから非ゼロの終了状態を書き込む。

別の Cargo プロジェクトから ELF を作る場合も、`#![no_std]`、`#![no_main]`、パニックハンドラと `rust_main(_: u64)` の C ABI を用意し、release のパニック方式を `abort` にする。必要に応じてこのリポジトリの `src/start.S`、`memory.x`、`build.rs` を利用し、`--target riscv64im-unknown-none-elf` を指定する。ROM 変換時に自分の ELF のパスを渡す。[Rust の RV64 ベアメタルターゲット](https://doc.rust-lang.org/nightly/rustc/platform-support/riscv64-unknown-none-elf.html)

この構成の ROM には命令と読み取り専用データをあわせて256バイトまで置ける。書き込み領域の初期値はゼロで、CPU が実装している命令だけを実行できる。大きなプログラムは UART から RAM へ載せる構成があり、ロード先と終了結果のアドレスが異なる。[起動 ELF のメモリ配置](firmware-memory.md) · [UART ブートローダー](uart-boot.md)
