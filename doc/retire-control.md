# 命令完了から更新要求まで

`firmware-machine` は、通常命令の完了通知 `simple` と、乗算器・除算器から受けた完了通知 `long-finished` を `firmware-retire` に渡す。この回路は PC 更新指示 `commit`、レジスタ書き込み要求 `register-enable`、メモリ書き込み要求 `memory-write` を返す。乗算器または除算器からの通知は、論理和を通る2 tick 後に `long-finished` に届く。

## ポートの契約

reset 直後を tick 0、最初に外部入力を適用する tick を1とする。tick `t` の入力はその tick に適用するフレーム、出力は同じ tick の確定直後の値を指す。

- `accept-pulse` の出力は tick 0～4 で0になる。tick `t >= 3` の `request` と `condition` が両方1のときだけ、tick `t + 2` に1を返す。
- `firmware-retire` は tick 0～5 で更新要求を出さない。tick `t >= 1` のどちらかの完了通知が1なら、tick `t + 3` の許可を適用し、tick `t + 5` に要求を出す。両通知が同じ tick に来た場合は1件として扱う。
- `commit` には `valid` が1、各書き込み要求には同じ tick の `valid` と対応する書き込み許可がともに1であることを要する。更新要求側の入力条件として、dispatch は `valid` が0なら両方の書き込み許可を0にする。
- 完了通知が0なら対応する要求は全て0になる。条件を満たす1 tick の完了通知は、固定の遅延を経て各要求に1回だけ現れる。

乗算・除算の完了通知を発行する条件、アドレスやデータの確定、更新要求を受け取った状態素子の動作は、それぞれの回路の契約で扱う。

## 展開済み配線の検証

検証器は CPU 本体と同じ NCG2 の定義を Core で階層展開する。VM が使う NOR、ラッチ、公開出力の配線から、参照仕様に従って各 tick の式を作る。各 NOR とラッチは tick 開始時の状態を読み、ラッチは許可が0なら値を保持する。公開出力は確定した状態とその tick の外部入力から求める。検証器はラッチの幅と配置、ポート幅を検査し、RAM があればモデルの生成を拒否する。[tick と状態素子の参照仕様](https://neknaj.github.io/circuitgame2/docs/simulation.html)

reset からの tick 0～5 の出力と、任意の状態から6 tick 進めた最後の出力を、上記のポート契約と比較する。後者ではすべてのゲート値とラッチ値を開始時に自由に選べるため、6 tick 以降の任意の実行を直前の6 tick に絞って検査できる。SMT-LIB への変換と Z3 を信頼し、Z3 が `unsat` を返すことを確認する。検証範囲はこの2回路のポートを通る要求である。

リポジトリのルートで実行する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for circuit in accept-pulse firmware-retire; do
  model="target/verification/$circuit.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification --bin verify-retire -- "$circuit" > "$model"
  z3 -smt2 "$model"
done
```

`sat` が出た場合は、生成した SMT-LIB の末尾に `(get-model)` を加えて再実行する。reset からの反例に含まれる `i_<tick>_<port>` をポート順のビット列にし、同じ VM に渡す。出力は tick とポート順の値である。任意の開始状態からの反例では、reset からその状態に到達するかを別に調べる。

```sh
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --bin verify-retire -- \
  firmware-retire --replay 10100 00000 00000 00110 00000 00000
```
