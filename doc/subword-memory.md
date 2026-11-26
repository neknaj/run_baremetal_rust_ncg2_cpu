# RV64 の部分読み出しと部分書き込み

アドレスの下位3ビットを `byte-offset` とし、そのアドレスを含む64ビットの語を `word` に渡す。オフセット0は語の下位8ビットを指す。メモリのアドレス判定には、下位3ビットを0にした語のアドレスを使う。[RISC-V のロードとストア](https://docs.riscv.org/reference/isa/v20240411/unpriv/rv32.html)

## バイト読み出し

`::rv64::memory::read-byte word byte-offset unsigned` は、指定したバイトを64ビットへ拡張した `value` を返す。LB では `unsigned=0` として符号拡張し、LBU では `unsigned=1` としてゼロ拡張する。8つのオフセットをすべて受け付ける。

## ハーフワード読み出し

`::rv64::memory::read-halfword word byte-offset unsigned` は、オフセットが0、2、4、6なら `aligned=1` とし、その位置から16ビットを取り出す。LH は `unsigned=0` で符号拡張し、LHU は `unsigned=1` でゼロ拡張する。奇数オフセットでは `aligned=0` となり、読み出し結果は使わない。

`execute-subword` は LB・LBU と、I 即値の加算器とメモリマップを共有する。LH と LHU では自然整列したアドレスが属する64ビット語を読む。整列違反や読み出し不可の場合、CPU は PC を保持し、書き戻しを抑止する。[RV64I のロードとストア](https://docs.riscv.org/reference/isa/v20240411/unpriv/rv64.html)

## 部分書き込み

`::rv64::memory::merge-store word value byte-offset funct3` は、命令の幅と自然整列を確認して `(valid updated-word)` を返す。対象の語はメモリから読み出し、その値を `word` に入れる。

| 命令 | `funct3` | 有効なオフセット | 書き込む `value` の下位ビット数 |
| --- | --- | --- | ---: |
| SB | `000` | 0～7 | 8 |
| SH | `001` | 0、2、4、6 | 16 |
| SW | `010` | 0、4 | 32 |

指定した位置のバイトだけを書き換え、語の残りのビットは保持する。表にない `funct3` や整列違反では `valid=0` とし、`word` をそのまま返す。接続側は書き込み可能なアドレスであることと `valid=1` を確認して、読み出した語と同じアドレスへ `updated-word` を書き戻す。SD は既存の8バイト書き込み経路を使う。

## 検証

展開後の NOR は、`read-byte` が394個、`read-halfword` が339個、`merge-store` が2,149個である。それぞれの最長経路は14段、10段、28段。接続側は、入力を変更してから出力を捕捉するまで、その段数に相当する tick を確保する。メモリ読み出し自体の遅延も加える。[NCG2 の tick と状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html)

実 NOR の全入力を符号拡張とビットマスクの数式に照合する。アドレスの所属、メモリポートの選択、書き込みの確定は、これらの部品を呼び出す CPU の接続で扱う。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for circuit in read-byte read-halfword merge-store; do
  model="target/verification/$circuit.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification --example verify_subword \
    -- "$circuit" > "$model"
  result=$(z3 -smt2 "$model")
  printf '%s\n' "$result"
  test "$result" = unsat
done
```
