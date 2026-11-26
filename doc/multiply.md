# RV64 の MUL 回路

`::rv64::arithmetic::mul-low` は `left` と `right` の積の下位64ビットを返す。64ビットの2の補数表現では、符号付きとして解釈しても下位64ビットは同じになる。[RISC-V の MUL 命令](https://docs.riscv.org/reference/isa/v20240411/unpriv/m-st-ext.html)

## 入力と完了

1. 最初に `left`、`right`、`initialize=1` を設定する。`capture=0` のまま457 tick 進める。次の1 tick だけ `capture=1` にするとオペランドを取り込み、`product` と `done` を0にする。
2. `initialize=0` にし、`capture=0` で457 tick、`capture=1` で1 tick を64回繰り返す。各更新まで他の入力を保持する。
3. 64回目の更新で `done` が1になり、同じ tick の `product` に積が出る。取り込みと64回の更新を合わせて29,770 tick かかる。`capture=0` に戻すと結果を保持する。次の計算は手順1から始める。

組み合わせ部分 `mul-round` の実 NOR は3,138個で、最長の依存経路は457段である。NOR は tick 開始時の信号を読み、ラッチも tick 開始時のデータを取り込む。このため、入力変更または前回の状態更新後に457 tick 待ってからラッチを更新する。[NCG2 の tick と状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html)

## 積の正しさ

更新直前の64ビット値を被乗数 `m`、乗数 `q`、積の途中値 `a` とする。初期化では `m=left`、`q=right`、`a=0` を設定する。各更新で、`q` の最下位ビットが1なら `a` に `m` を加える。その後 `m` を左、`q` を右に1ビットずつずらす。加算と左シフトで64ビットを超える桁は捨てる。

`a + m × q` を `2^64` で割った余りは各更新で保存される。64回の論理右シフトで `q=0` になるため、最終的な `a` は積の下位64ビットに等しい。反復回数は初期化時にビット0を立てた64ビットの値を左にずらして記録する。更新前のビット63が1になるのは64回目であり、このとき積と同時に `done` が立つ。

検証は、実 NOR の全入力に対する1回の更新と、数学上の不変条件の保存に分ける。NCG2 のラッチと上記の入力手順を結んだ動作は、VM 結合テストで起動イメージの乗算を使って確認する。[CPU の実行回路](firmware-execution.md)は初期化後に518 tick 間隔で更新し、`done` が立ってから乗算命令を完了させる。

リポジトリのルートからホスト向けに実行する。Z3 の2つの検査はいずれも `unsat` を返す。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
cargo +1.99.1 run --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification -- mul-round > target/verification/mul-round.smt2
z3 target/verification/mul-round.smt2
z3 verification/mul_composition.smt2
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test multiply
```
