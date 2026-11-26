# 整数除算の演算回路

RISC-V の整数除算はゼロ方向に丸める。除数が0なら商は演算幅の全ビットが1、余りは被除数である。符号付き最小値を −1 で割ると、商はその最小値、余りは0になる。[RISC-V M 拡張の除算仕様](https://docs.riscv.org/reference/isa/v20260120/unpriv/m-st-ext.html#_division_operations)

符号なし除算の1反復と、符号付き命令・32ビット命令の入出力補正を実装した。反復の開始と終了、命令デコーダへの接続は後続の工程で行う。

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

## 反復と符号補正の証明

被除数を上位ビットから読み、処理済み部分の値を `n` とする。除数 `d` が正のとき、商 `q` と余り `r` は `n = d × q + r` と `0 ≤ r < d` を保つ。各反復で新しいビットを `n` に取り込み、比較結果を商に追加する。除数0では比較結果が毎回1になり、余りには処理済みの被除数が残る。

`verification/division.rs` では、上の反復から32・64ビットの整数除算の商と余りが得られることを Verus で証明する。符号付き演算は絶対値を使った反復に符号補正を適用し、余りの大きさと符号、ゼロ除算と符号付きオーバーフローの結果を照合する。次のコマンドはリポジトリのルートで実行する。

```sh
verus verification/division.rs --crate-type=lib --no-cheating
```

## 符号と演算幅の補正

`::rv64::arithmetic::divide-prepare` は被除数、除数、符号付き指定、W 指定を受け取り、符号なし除算器に渡す64ビットの大きさを2つ返す。W 指定では元の入力の上位32ビットを捨て、符号付きなら下位32ビットを符号拡張し、符号なしならゼロ拡張する。符号付きの負の値は2の補数で絶対値にする。

`::rv64::arithmetic::divide-result` は同じ入力と、符号なし除算器から得た商と余りを受け取る。符号付きの場合、両オペランドの符号が異なれば商の符号を反転し、被除数が負なら余りの符号を反転する。演算幅で解釈した除数が0の場合と、符号付き最小値を −1 で割る場合は ISA で決まる値を選ぶ。W の出力は符号付き、符号なしともにビット31から64ビットへ符号拡張する。

Z3 は入力整形の全130ビット、出力補正の全258ビットについて、展開した NOR の出力と独立したビットベクトル仕様との差を探す。出力補正では商と余りの大きさも任意の入力として扱う。最長経路は入力整形が457段、出力補正が469段である。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for part in divide-prepare divide-result; do
  model=target/verification/$part.smt2
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification -- "$part" > "$model"
  test "$(z3 -smt2 "$model")" = unsat
done
```
