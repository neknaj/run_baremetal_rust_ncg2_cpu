# 整数除算の演算回路

RISC-V の整数除算はゼロ方向に丸める。除数が0なら商は演算幅の全ビットが1、余りは被除数である。符号付き最小値を −1 で割ると、商はその最小値、余りは0になる。[RISC-V M 拡張の除算仕様](https://docs.riscv.org/reference/isa/v20260120/unpriv/m-st-ext.html#_division_operations)

現在の回路は64ビットの符号なし除算を実装する。命令デコーダ、符号付き入力、32ビット命令の切り出しと符号拡張はこれから接続する。

## 開始と完了

`::rv64::arithmetic::div-wide` は64ビットの被除数と除数、`initialize`、`capture` を受け取り、`(quotient remainder done)` を返す。オペランドと `initialize=1` を設定する。`capture=0` で480 tick 進めた後、1 tick の `capture=1` で入力を取り込む。

`initialize=0` にして、480 tick の待機と1 tick の捕捉を64回繰り返す。64回目で `done=1` となり、同じフレームの商と余りを使える。待機を含めて初期化の入力設定から完了まで31,265 tick かかる。開始前と完了後は `initialize=0` の捕捉で状態を保つ。次の入力は初期化の捕捉で取り込む。

被除数と商は同じ64ビットのレジスタを共用する。上位から未処理の被除数を消費し、空いた下位に商を追加する。64回目にはレジスタ全体が商になる。回数を示すビット列を初期化で1にし、各反復で1ビットずらす。更新前に最上位ビットが1なら、その更新で完了とする。

## 符号なし除算の1反復

`::rv64::arithmetic::div-round` は64ビットの余り `r`、64ビットの除数 `d`、被除数の次のビット `b` を受け取る。試行値は65ビットの `t = 2r + b` である。`t ≥ d` なら商のビットは1、次の余りは `t − d` の下位64ビットとなる。それ以外では商のビットは0、次の余りは `t` の下位64ビットとなる。

回路は試行値の下位64ビットから除数を引く。この減算で借りが出ない場合と、試行値の65ビット目が1の場合に減算結果を選ぶ。反復器が `r < d` を満たして呼び出すと、次の余りも `d` 未満に収まる。

## 検証

展開した NOR の両出力を、65ビットで比較と減算を行う参照仕様に照合する。この照合は除数0と `r ≥ d` の入力も含む。もう一つの検査では `r < d` なら次の余りが `d` 未満になることを確認する。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/div-round.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification -- div-round > "$model"
test "$(z3 -smt2 "$model")" = "$(printf 'unsat\nunsat')"
```

展開した回路は1,666個の NOR からなり、入出力間の最長経路は462段である。検証器はこの段数を上限とする。

初期化、結果保持、64回目の完了を含む `div-wide-next` は4,810個の NOR で、最長経路は470段。手順の待機480 tick はこの遅延を覆う。Z3 で次状態の全出力を独立した式に照合する。VM の結合試験では初期化から完了まで、ゼロ除算、連続する演算、完了後の捕捉を調べる。

```sh
model=target/verification/div-wide-next.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification -- div-wide-next > "$model"
test "$(z3 -smt2 "$model")" = unsat
cargo +1.99.1 test --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test divide
```

## 反復と符号補正の証明

被除数を上位ビットから読み、処理済み部分の値を `n` とする。除数 `d` が正のとき、商 `q` と余り `r` は `n = d × q + r` と `0 ≤ r < d` を保つ。各反復で新しいビットを `n` に取り込み、比較結果を商に追加する。除数0では比較結果が毎回1になり、余りには処理済みの被除数が残る。

`verification/division.rs` では、上の反復から32・64ビットの整数除算の商と余りが得られることを Verus で証明する。符号付き演算は絶対値を使った反復に符号補正を適用し、余りの大きさと符号、ゼロ除算と符号付きオーバーフローの結果を照合する。次のコマンドはリポジトリのルートで実行する。

```sh
verus verification/division.rs --crate-type=lib --no-cheating
```
