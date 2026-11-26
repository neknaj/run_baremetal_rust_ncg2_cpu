# 整数除算の演算回路

RISC-V の整数除算はゼロ方向に丸める。除数が0なら商は演算幅の全ビットが1、余りは被除数である。符号付き最小値を −1 で割ると、商はその最小値、余りは0になる。[RISC-V M 拡張の除算仕様](https://docs.riscv.org/reference/isa/v20260120/unpriv/m-st-ext.html#_division_operations)

現在の回路は符号なし除算の1反復を実装する。反復の開始と終了、命令デコーダ、32ビット命令の切り出し・符号拡張はこれから接続する。

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
