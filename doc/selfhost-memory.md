# Core 起動用メモリ

Core の起動 ELF では、命令と読み取り専用データが同じ ROM に入る。セルフホスト用のプロジェクト設定では、`firmware-memory.ncg2` と小さい起動 ROM に代えて、`selfhost-memory.ncg2` と12ビット添字の ROM を選ぶ。通常の起動デモは従来のソースを使う。

| 開始アドレス（含む） | 終了アドレス（含まない） | 語数 | 用途 |
| --- | --- | ---: | --- |
| `0x10000` | `0x16000` | 3,072 | 読み取り専用 ROM |
| `0x16000` | `0x19000` | 1,536 | BSS |
| `0x2bf00` | `0x2c000` | 32 | 起動要求の領域 |
| `0x2c000` | `0x30000` | 2,048 | スタックと結果 |

12ビット添字の ROM は4,096語を格納できる。データポートが ROM を読み出せるのは表の区間である。RAM は3つの記憶部に分かれ、リセットで全語が0になる。起動時に CPU の起動手順で要求領域の初期値を設定する。実行中は他の RAM と同じデータポートで要求領域を読み書きできる。

## 接続と tick

`firmware-fetch` は起点 `0x10000` からのバイトオフセットを受け付ける。オフセットが `0x6000` 未満で4バイト境界にあれば `valid=1` になり、対応する32ビットをリトルエンディアンで返す。無効な位置では命令出力を使わない。

`firmware-data-access` と `firmware-data-memory` のアドレスは絶対位置で、8バイト境界にある語を指す。ROM では `readable=1`、`writable=0`。RAM では両方が1で、未配置・非整列では両方が0になる。`read-data` は `readable=1` のときに使う。

アドレスを固定してから読み出しに必要な時間は `max(30, D + 4)` tick である。ここで `D` は ROM の最長 NOR 経路の段数。今回の Core 起動 ELF から作った ROM では `D=45`、フェッチとデータ読み出しはいずれも49 tick 以内に安定する。書き込み後に同じ位置の新しい値を読むときは、書き込みの要求を入力した tick `t` から `t + 30` までアドレスを保持する。

有効な RAM アドレスと `write=1` のフレームを tick `t` に入力すると、`t + 17` に更新する。連続した tick に異なるバンクや同じ語を指定できる。ROM・未配置・非整列への要求で RAM は変わらない。リセットでは途中の要求も破棄する。

書き込みのバンク判定は、同じフレームの正転・反転ビットを14段で処理する。その結果と、14段のラッチ経路を通してから反転した許可が `t + 14` にそろう。1段の NOR で書き込み可否を決め、ラッチを通すと `t + 16` に確定する。アドレスと値も同じ tick に届くため、RAM は次の tick に更新する。BSS の添字の反転は、最終ラッチと同じ段に置く。[NCG2 の状態素子](https://neknaj.github.io/circuitgame2/docs/simulation.html)

## 検証

`selfhost-memory.toml` はテスト用の ROM でメモリ部品を選ぶ。次の手順は、実際に NOR まで展開して、区間と整列の全入力での一致、14段の判定経路、フェッチの有効条件と命令を確認する。VM 結合テストでは、連続して指定した書き込みの確定時刻、各バンクの境界、リセット時の動作を、記憶部の値から確認する。

```sh
set -e
ncg2 check --project selfhost-memory.toml
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
for part in banks access fetch; do
  model="target/verification/selfhost-$part.smt2"
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --target-dir target/verification --example verify_selfhost_memory \
    -- "$part" > "$model"
  test "$(z3 -smt2 "$model")" = unsat
done
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test selfhost_memory
```

実 ELF を変換した ROM の正しさは、`verify_rom` で全語をバイナリイメージと照合する。`verify_selfhost_memory` の `measure-rom` と `measure-fetch` に ROM のソースファイルを渡すと、同じ ROM を使った最長経路を表示する。
