# UART 受信器

`::rv64::uart-rx` は 8N1 を受信する。入力 `rx` は1ビットで、アイドルと停止は High、開始は Low。データ8ビットは最下位ビットから来る。固定の1ビット時間は16 tick。中点を一度ずつサンプリングする。[UART のフレームと中点サンプリング](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ApplicationNotes/ApplicationNotes/doc0952.pdf)

reset 後、送信側は最初の開始まで RX を16 tick 以上 High に保つ。開始候補は、フレームを受信していないときに4 tick 以上連続した High の直後に来る Low である。この Low を最初に適用した tick を `s` とする。

| tick | 動作 |
| --- | --- |
| `s + 8` | 開始を採取する。High ならこの候補を破棄する |
| `s + 24 + 16i` | データビット `i` を採取する。 `i = 0..7` |
| `s + 152` | 停止を採取する |
| `s + 159` | 開始が Low かつ停止が High なら `byte` を更新し `valid` を1にする。開始と停止がともに Low なら `framing-error` を1にする |
| `s + 160` 以降 | 次の開始候補を受け付ける |

`valid` と `framing-error` は各1 tick。両方が同時に1になることはない。`byte` は reset で0になり、次の `valid` まで値を保つ。ロード側は `valid` と同じ公開フレームの `byte` を1バイトとして受け取る。回路の別のラッチに接続するとき、ラッチはその公開フレームを次の tick の更新で読む。

## 検証

`verify-uart` は Core が実際に展開した NOR とラッチから tick の遷移式を作る。reset から隣接する二つの開始候補を置き、開始候補の Low と直前の High 以外の RX を記号化する。すべての tick の `byte`、`valid`、`framing-error` を、中点のサンプルだけから決めた仕様と比較する。Z3 の `unsat` は、この二枠の全入力で差がないことを示す。任意長の入力列での帰納的な保証は、この検証の範囲に含まれない。実 VM では続けて届くバイト、開始候補の破棄、フレーミングエラーからの回復、停止の中点を過ぎてからのノイズを確認する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart | z3 -in
```
