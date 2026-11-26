# 符号なし除算の1反復

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
