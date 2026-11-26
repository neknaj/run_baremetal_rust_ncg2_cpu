# RV64 の符号なし128ビット乗算

`::rv64::arithmetic::mul-wide left right initialize capture` は、64ビットの2値を符号なしとして乗算し、`(low high done)` を出す。MULHU に渡す値は `high`、MUL に渡せる値は `low` である。結果は `done=1` のときに使う。[RISC-V の整数乗算](https://docs.riscv.org/reference/isa/v20240411/unpriv/m-st-ext.html)

## 入力と完了

1. `left`、`right`、`initialize=1` を設定し、`capture=0` で460 tick 進める。次の1 tick だけ `capture=1` にするとオペランドを取り込み、`done` を0にする。
2. `initialize=0` にして、`capture=0` の460 tick と `capture=1` の1 tick を64回繰り返す。捕捉まで入力を保持する。
3. 64回目の捕捉で `done` が立ち、同じフレームの `low` と `high` に積が出る。`capture=0` に戻すと両方の結果と完了状態を保持する。

更新回路の NOR は3,138個で、最長経路は460段である。初期化から完了まではこの手順で29,965 tick かかる。捕捉の間隔を長くしても、同じ入力と状態を保持していれば結果は変わらない。[NCG2 の状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html)

## 積の正しさ

初期化で、状態の上位語に0、下位語に乗数を置く。被乗数は別に保持する。下位語の最下位ビットが1なら上位語に被乗数を加える。加算器の桁上がりを含む129ビットの値を右に1ビットずらし、上位語と下位語に保持する。加算する2値はそれぞれ128ビット以内なので、和は129ビットに収まる。反復回数を記録するビット列は初期化時に1になり、各更新で左に1ビットずれる。更新前にビット63が立っている64回目で完了する。

初期入力を被乗数 `A`、乗数 `B` とする。反復 `i` 回の後、処理済みの乗数の下位ビットを整数 `C = B mod 2^i`、残りを `R = floor(B / 2^i)` とすると、連結した状態の値は `Z = R + A × C × 2^(64-i)` となる。

初期化直後は `C=0`、`R=B`、`Z=B` で成り立つ。64回目の前までは `2^(64-i)` が偶数なので、状態の最下位ビット `b` は `R` の最下位ビットに等しい。1回の更新で `Z` に `b × A × 2^64` を加えて右シフトする。すると `R` は `floor(R / 2)` に、`C` は `C + b × 2^i` になり、不変条件を保つ。64回後に `R=0`、`C=B` となり、`Z=A×B` を得る。

## 検証

Z3 は、実 NOR の全入力に対する初期化と1回の更新を129ビットの独立仕様と照合する。整数の仕様では右シフトで捨てるビットと不変条件の保存を確認する。VM 結合テストは64ビット最大値同士の積、64回目の完了、Core 起動 ELF で観測した `32 × 32` での再初期化と積も確認する。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/mul-wide-round.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_mul_wide > "$model"
test "$(z3 -smt2 "$model")" = unsat
test "$(z3 -smt2 verification/mul_wide_composition.smt2)" = "$(printf 'unsat\nunsat')"
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test multiply_wide
```

Z3 は合計3行の `unsat` を返す。CPU の命令選択と書き戻しは接続側で検証する。

## 符号付き上位語と MULW

`::rv64::arithmetic::mul-correct` は元のオペランド、符号なし積の下位語と上位語、命令の指定を受け取る。MULH は両方のオペランドを符号付きとして読み、MULHSU は左のオペランドだけを符号付きとして読む。MULW は積の下位32ビットをビット31から64ビットに符号拡張する。[RISC-V M 拡張の乗算仕様](https://docs.riscv.org/reference/isa/v20240411/unpriv/m-st-ext.html)

64ビットの語の値域の大きさを `B = 2^64` とする。符号付きで負のオペランドの値は同じビット列を符号なしとして読んだ値から `B` を引いて得られる。左を負として読む場合は符号なし積の上位語から右の値を引き、右を負として読む場合は左の値を引く。両方が負のときに生じる `B²` の項は、積を128ビットで表すと上位64ビットの値に影響しない。

`word-mode` が1なら MULW の値を返す。それ以外では `high-half` で下位語か補正後の上位語を選ぶ。`signed-left` と `signed-right` は上位語を補正するときに使う。入力が変わった後の最長経路は477段で、展開後の NOR は4,238個である。

Z3 は260ビットの全入力について、展開した NOR の出力を独立したビットベクトル仕様と照合する。Verus は2語が元の符号なし積を表す場合に、補正した上位語が指定された符号での積の上位64ビットになることと、MULW が下位32ビット同士の積と一致することを証明する。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/mul-correct.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification -- mul-correct > "$model"
test "$(z3 -smt2 "$model")" = unsat
verus verification/multiply_correction.rs --crate-type=lib --no-cheating
```
