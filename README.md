# NCG2 で作る RV64IM CPU

Neknaj Circuit Game 2 で RV64IM の CPU を組み、ベアメタル Rust のプログラムを動かす。最終的には、その CPU 上でも NCG2 Core で回路を評価できるようにする。

## 命令デコーダを試す

`circuit/decode.ncg2` は、32ビット命令語から opcode、レジスタ番号、funct3、funct7 と、64ビットに符号拡張した I・S・B・U・J 形式の即値を取り出す。命令形式の選択、レジスタの更新、メモリアクセスは後続の回路で扱う。

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

## 起動 ELF を作る

```sh
rustup toolchain install nightly --profile minimal --component rust-src
cargo +nightly build --release --locked -Z build-std=core
```

成果物は `target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot`。ELF のロード領域を配置したあと、`0x10000` から実行する。起動コードがスタックを用意して BSS を消去し、Rust の `rust_main` を呼ぶ。`boot_halt` 到達後に RAM の `0x2fff0` が成功値 `1`、`0x2fff8` が `0x0000_0005_0000_0003` なら検査成功。A・C・F・D を無効にした QEMU ユーザーモードで確認した。

## NCG2M のプログラムデータ

`programs/boot.ncg2m` は、RAM の初期内容を64ビット語の添字と値で表す。省略した語の値は0。Rust を変更したときは、開発時に ELF から更新する。

```sh
python tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot programs/boot.ncg2m --format ncg2m
ncg2 check programs/boot.ncg2m
```

次は、命令デコーダをレジスタ・ALU・PC に接続する。プログラムはプロジェクトに置いた命令イメージから NCG2M で ROM を生成し、通常のシミュレーションで読み出せるようにする。
