# RV64 の整数 ALU

`::rv64::arithmetic::execute-alu instruction first-value second-value` は、命令語と読み出したレジスタ値から `(valid result)` を返す。表の符号化を受理し、結果を64ビットで返す。未対応の符号化では `valid=0`、`result=0` となる。

| 命令 | opcode | funct3 | funct7 または即値の条件 | 結果 |
| --- | --- | --- | --- | --- |
| ADD | `0x33` | 0 | funct7 が `0x00` | `rs1 + rs2` の下位64ビット |
| ADDIW | `0x1b` | 0 | 12ビットの即値 | `rs1` と符号拡張した即値の和を下位32ビットに切り詰め、結果を64ビットへ符号拡張する |
| AND | `0x33` | 7 | funct7 が `0x00` | `rs1` と `rs2` のビットごとの AND |
| ANDI | `0x13` | 7 | 12ビットの即値 | `rs1` と符号拡張した即値のビットごとの AND |
| OR | `0x33` | 6 | funct7 が `0x00` | `rs1` と `rs2` のビットごとの OR |
| ORI | `0x13` | 6 | 12ビットの即値 | `rs1` と符号拡張した即値のビットごとの OR |
| SLTIU | `0x13` | 3 | 12ビットの即値 | `rs1` が符号拡張した即値より小さければ1。拡張後の両値を符号なしで比較する |
| SRAI | `0x13` | 5 | ビット26～31が `010000` | 元の符号ビットを埋める右シフト |
| SRL | `0x33` | 5 | funct7 が `0x00` | `rs2` の下位6ビット分、0を埋める右シフト |
| SRLI | `0x13` | 5 | ビット26～31が `000000` | 0を埋める右シフト |
| SUB | `0x33` | 0 | funct7 が `0x20` | `rs1 − rs2` の下位64ビット |
| XORI | `0x13` | 4 | 12ビットの即値 | `rs1` と符号拡張した即値のビットごとの XOR |

SRLI と SRAI は命令ビット20～25の6ビットをシフト量として使う。ビット25が1でも、上位6ビットが表に合うなら受理する。[RV64I の整数命令](https://docs.riscv.org/reference/isa/v20240411/unpriv/rv64.html) · [RV32I と共通の整数命令](https://docs.riscv.org/reference/isa/v20240411/unpriv/rv32.html)

ADD・ADDIW・SUB は既存の加算器を共有する。SUB では第2オペランドを反転し、最下位桁への桁上がりを1にする。ADDIW では加算結果のビット31を上位32ビットへ複製する。論理右シフトと算術右シフトは同じ段を使い、空いた上位ビットに設定する値を切り替える。

## 検証

展開後の NOR は8,948個で、最長経路は478段である。接続側は入力が変化した後、結果を取り込むまでに478 tick を確保する。アドレス計算、PC の更新、レジスタへの書き込みは接続側で行う。[NCG2 の tick](https://neknaj.github.io/circuitgame2/docs/simulation.html)

Z3 で、全命令語について受理条件が一致すること、受理した12命令について全オペランドの結果が一致すること、拒否時の結果が0であることを調べる。出力は14行すべてが `unsat` になる。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/integer-alu.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_alu > "$model"
result=$(z3 -smt2 "$model")
printf '%s\n' "$result"
test "$(printf '%s\n' "$result" | grep -cx unsat)" -eq 14
test "$(printf '%s\n' "$result" | wc -l)" -eq 14
```
