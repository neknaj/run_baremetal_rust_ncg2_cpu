# Rust 起動回路の構成と検証

ELF を用意して実行する手順は[自作のベアメタル Rust を動かす](run-your-rust.md)を参照する。

`::rv64::firmware-machine` は起動 ELF を格納した ROM をフェッチし、レジスタとデータメモリを更新する。リセット時に起動アドレス `0x10000` から実行を始める。起動プログラムの命令と、セルフホスト用 ELF に必要な追加の算術・メモリ命令を扱う。[起動用命令](boot-execution.md) · [分岐と PC 相対命令](control-execution.md) · [整数 ALU](integer-alu.md) · [部分書き込み](subword-memory.md)

## 命令の完了と書き込み

レジスタファイルは1ポートを2つのオペランドの読み出しと書き戻しに使う。命令を開始してから145 tick 以内に両オペランドを取り込む。同梱 ROM のフェッチの最大経路は21段、受理と書き込み先を決める組み合わせ回路の最大経路は505段である。メモリの読み出しと選択信号が確定する時間も取り、命令を開始してから1024 tick 後に実行結果を使う。MUL と MULHU は同じ乗算回路を使う。初期化後に518 tick 間隔で更新し、64回目の更新で得た積の下位または上位64ビットを書き戻す。[128ビット乗算の回路と証明](multiply-wide.md)

DIV・REM 系8命令は、両オペランドとモードを除算器に渡し、完了後に商または余りを書き戻す。通常の完了通知は乗算と除算では抑止する。3種類のメモリ配置で、命令の受理、結果の選択、PC とレジスタの更新要求を展開済み NOR と独立した式で照合した。[整数除算の演算回路](division.md)

`commit` が立つフレームでは命令の PC オフセットと、レジスタ・メモリへの書き込み要求が出る。PC は次の tick、書き込み先が `x0` 以外のレジスタは8 tick 後に更新される。ストア先の RAM は起動デモで16 tick 後、セルフホスト用の構成で22 tick 後に更新される。フェッチ先が無効、命令が未対応、またはアクセスがメモリの規則に合わない場合は `stopped` が立ち、命令の完了と書き込みは発生しない。[更新要求の時系列検証](retire-control.md)

## 結果領域の観測

ロードまたはストアを処理している間は、その命令がメモリのデータポートを使う。他の命令では観測用のアドレスを選ぶ。`memory-address` は現在選んでいるアドレスを示し、`probe-data` にはメモリの読み出し遅延がある。観測するときは選択後のアドレスを保持し、起動デモでは32 tick、セルフホスト用の構成では53 tick 待つ。[メモリの読み出し規則](firmware-memory.md#ram-の更新と読み出し) · [セルフホスト用のメモリと入力回路](selfhost.md)

`firmware.toml` の `::rv64::firmware-demo` は、プログラムが積と終了状態を RAM に書くのを待ち、読み出した2語を出力に保持する。同梱イメージでは、82,894 tick で `finished` が立つ。プログラム終了後の JAL 自己ループは引き続き動作する。

## 検証の範囲

結合テストは実際の ROM から実行し、BSS の初期化ループ、LD、MUL、Rust からの復帰と、結果領域の値を確認する。別の命令列で部分書き込み、LB・LBU の拡張、非整列 SH による停止も確認する。[バイト読み出しと部分書き込み](subword-memory.md)

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --release --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test firmware_machine
```

`firmware-dispatch` と `firmware-dispatch-core` の検証では、起動デモ用とセルフホスト用のメモリ配置をそれぞれ選ぶ。任意の命令語、PC、レジスタ値、フェッチの成否、メモリの読み出し値と乗算の結果について、受理条件、次の PC、レジスタと RAM への更新要求、書き込む値を参照仕様と照合する。無効な入力では PC オフセットを保持し、両方の更新要求を抑止することも検証する。実際に更新する tick は CPU の結合テストで確認する。[組み合わせ回路と乗算の検証手順](boot-components.md#起動回路の部品を検証する)

小型の Core ELF は CPU 上で実行済みである。回路ソースを NOR に展開して評価する Core ELF でも、起動前に UART で送った全語が RAM の内容と一致した。起動後はアプリの退役 PC 50,924件と終了結果が QEMU と一致した。[全量転送とアプリの照合結果](selfhost.md#全量転送とアプリの照合結果)

全量照合は記載した ELF と入力を対象とする。CPU 全体の形式証明と、残りの RV64IM 命令の実装は今後の課題である。

## 同梱の起動 ELF を再生成する

```sh
rustup toolchain install nightly --profile minimal --component rust-src
cargo +nightly build --release --locked -Z build-std=core
```

成果物は `target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot`。ELF のロード領域を配置したあと、`0x10000` から実行する。起動コードがスタックを用意して BSS を消去し、Rust の `rust_main` を呼ぶ。`boot_halt` 到達後に RAM の `0x2fff0` が成功値 `1`、`0x2fff8` が `0x0000_0005_0000_0003` なら検査成功。A・C・F・D を無効にした QEMU ユーザーモードで確認した。

## ELF を回路のメモリに載せる

`circuit/firmware-rom.ncg2` は、起動 ELF の命令と読み取り専用データを格納する。通常の実行は NCG2 のファイルを使う。Rust を変更したときに、生成済み ROM を更新する。

```sh
python3 tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot \
  circuit/firmware-rom.ncg2 --format ncg2-rom
```

フェッチと、ROM・BSS・スタックの読み書き回路は[起動 ELF のメモリ配置](firmware-memory.md)に記す。

## NCG2M のプログラムデータ

`programs/boot.ncg2m` は、RAM の初期内容を64ビット語の添字と値で表す。省略した語の値は0。Rust を変更したときは、開発時に ELF から更新する。

```sh
python3 tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot programs/boot.ncg2m --format ncg2m
ncg2 check programs/boot.ncg2m
```

実行用の NCG2 ROM を、この NCG2M データから展開する経路は作業中である。
