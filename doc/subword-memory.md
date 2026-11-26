# RV64 のバイト読み出しと部分書き込み

アドレスの下位3ビットを `byte-offset` とし、そのアドレスを含む64ビットの語を `word` に渡す。オフセット0は語の下位8ビットを指す。メモリのアドレス判定には、下位3ビットを0にした語のアドレスを使う。[RISC-V のロードとストア](https://docs.riscv.org/reference/isa/v20240411/unpriv/rv32.html)

## バイト読み出し

`::rv64::memory::read-byte word byte-offset unsigned` は、指定したバイトを64ビットへ拡張した `value` を返す。LB では `unsigned=0` として符号拡張し、LBU では `unsigned=1` としてゼロ拡張する。8つのオフセットをすべて受け付ける。

## 部分書き込み

`::rv64::memory::merge-store word value byte-offset funct3` は、命令の幅と自然整列を確認して `(valid updated-word)` を返す。対象の語はメモリから読み出し、その値を `word` に入れる。

| 命令 | `funct3` | 有効なオフセット | 書き込む `value` の下位ビット数 |
| --- | --- | --- | ---: |
| SB | `000` | 0～7 | 8 |
| SH | `001` | 0、2、4、6 | 16 |
| SW | `010` | 0、4 | 32 |

指定した位置のバイトだけを書き換え、語の残りのビットは保持する。表にない `funct3` や整列違反では `valid=0` とし、`word` をそのまま返す。接続側は書き込み可能なアドレスであることと `valid=1` を確認して、読み出した語と同じアドレスへ `updated-word` を書き戻す。SD は既存の8バイト書き込み経路を使う。

## 検証

展開した `read-byte` は394 NOR、最長経路は14段である。`merge-store` は2,149 NOR、最長経路は28段である。組み合わせ部分の出力を捕捉するまでに、接続側では入力が変わってからそれぞれ14 tick、28 tick を確保する。メモリ読み出し自体の遅延も加える。[NCG2 の tick と状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html)

実 NOR の全入力を符号拡張とビットマスクの数式に照合する。アドレスの所属、メモリポートの選択、書き込みの確定は、これらの部品を呼び出す CPU の接続で扱う。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for circuit in read-byte merge-store; do
  model="target/verification/$circuit.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification --example verify_subword \
    -- "$circuit" > "$model"
  result=$(z3 -smt2 "$model")
  printf '%s\n' "$result"
  test "$result" = unsat
done
```
