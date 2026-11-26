# NCG2 で作る RV64IM CPU

Neknaj Circuit Game 2 で RV64IM の CPU を組み、ベアメタル Rust のプログラムを動かす。現在は、この CPU 上で Core の厳格 S 式パーサーと1ゲートの NOR 評価器を実行できる。

[設計と検証のドキュメント](https://neknaj.github.io/run_baremetal_rust_ncg2_cpu/)では CPU の構成、UART ブート、Core の実行を説明する。ローカルで読むにはリポジトリ直下で `mdbook serve` を実行する。

自分で書いたコードを試す場合は、[自作のベアメタル Rust を動かす](doc/run-your-rust.md)の手順で ELF を作り、NCG2 の ROM に載せる。

## Rust の起動プログラムを実行する

`firmware.toml` は、リポジトリに同梱した Rust ELF のイメージを読む NCG2 回路を起動する。実行に Python や QEMU は不要で、`ncg2` の `run` で tick を進める。

```sh
ncg2 check --project firmware.toml
ncg2 run --project firmware.toml --ticks 90000 | tail -n 1
```

最終行のビット列は PC オフセット・終了状態・積を64ビットずつ、続いて `finished` と `stopped` を1ビットずつ並べる。このプログラムの結果は PC オフセット `0x64`、終了状態 `1`、積 `0x0000_0005_0000_0003`、`finished=1`、`stopped=0` である。各ポートは最下位ビットから表示する。[実行回路と検証手順](doc/firmware-execution.md)

## Core を組み込んだ ELF

`selfhost.toml` では、ベアメタル Rust から Core を呼び、RAM 上のソースを解析する。続いて固定の NOR 回路を評価し、ノード数と出力を書き戻す。[ビルドと検証手順](doc/selfhost.md)

`compile-circuit` を指定してビルドすると、受け取った回路ソースを Core で NOR に展開し、指定した入力で評価できる。この ELF と要求データを `uart-boot.toml` の回路へ転送し、RAM から起動したアプリの退役 PC 50,924件と終了結果が QEMU と一致した。[ビルド、UART 転送と照合の手順](doc/selfhost.md#ram-の回路ソースを展開する-elf)

## 起動回路の部品を検証する

[起動回路の部品を試す](doc/boot-components.md)に、デコーダ、レジスタ、Z3 による検証の手順をまとめる。
