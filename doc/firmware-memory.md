# 起動 ELF のメモリ配置

起動 ELF の命令とデータは、同じ物理アドレスで指定する。回路は次の範囲を扱う。区間の終点は含まない。

| 開始 | 終点 | 内容 |
| --- | --- | --- |
| `0x10000` | `0x10100` | ROM。起動コード、Rust の命令、読み取り専用データを格納する |
| `0x11000` | `0x12000` | RAM。BSS を置く |
| `0x2c000` | `0x30000` | RAM。スタックと結果領域を置く |

`firmware-fetch` は起動アドレス `0x10000` からのバイトオフセットで命令を読む。ROM 内の4バイト境界を有効とし、その位置の4バイトをリトルエンディアンで返す。無効な位置の命令出力は使わない。`firmware-data-map` は絶対アドレスで、8バイト境界にある語のバンクとバンク内添字を求める。

`ncg2-rom` の生成では ELF のロード領域全体を検査する。読み取り専用領域は ROM に収める。書き込み可能でファイルによる初期化が不要な領域は、いずれか一方の RAM に収める。リンカは BSS とスタックを別々の `PT_LOAD` に置く。リセット時の RAM は全語を0にする。

## RAM の更新と読み出し

`firmware-data-memory` は各 tick のアドレス、データ、許可を1件の書き込み要求として受け取る。tick `t` の要求が書き込み可能な語を指すとき、その語を `t + 15` に更新する。ROM、未配置、非整列への書き込み要求で RAM は変わらない。連続した tick に異なるバンクや添字を指定できる。

アドレスの正極性と反転ビットを同じフレームから取り込み、12段の NOR 木でバンク判定をそろえる。判定と同じフレームの許可が到着してから、該当する RAM に書き込む。読み出し側はアドレスを32 tick 保持した後に `readable` と `read-data` を使う。tick `t` に同じ語への書き込みを要求し、そのアドレスを保持する場合、新しい値は `t + 24` までに読み出し出力に届く。[NCG2 の tick と状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html#状態素子)

## 検証

リポジトリのルートで起動 ELF をビルドしてから実行する。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
image=target/verification/firmware.bin
python tools/elf_image.py target/riscv64im-unknown-none-elf/release/ncg2-rv64-boot "$image"
for circuit in firmware-rom firmware-fetch firmware-data-map; do
  model="target/verification/$circuit.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification -- "$circuit" "$image" > "$model"
  result=$(z3 -smt2 "$model")
  printf '%s\n' "$result"
  test "$result" = unsat
done
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test firmware_memory
```

Z3 は ROM の全語、フェッチの有効条件と命令、メモリ区間と添字を展開済み NOR 回路と照合する。区間の検証では、バンク判定の全入力経路が12段であることも実ネットリストから確認する。結合テストは VM で連続する書き込みの確定 tick と RAM の内容を検査する。CPU からの命令実行を含む順序回路全体の形式証明は今後の作業である。
