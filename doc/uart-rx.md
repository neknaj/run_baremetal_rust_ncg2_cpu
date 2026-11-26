# UART 受信器

`::rv64::uart-rx` は 8N1 を受信する。入力 `rx` は1ビットで、アイドルと停止は High、開始は Low。データ8ビットは最下位ビットから来る。1ビットの長さは `::rv64::uart-rx` が16 tick、連続転送用の `::rv64::uart-rx-8` が8 tick。各ビットの中点を一度ずつ採取する。[UART のフレームと中点サンプリング](https://ww1.microchip.com/downloads/aemDocuments/documents/OTH/ApplicationNotes/ApplicationNotes/doc0952.pdf)

reset 後、送信側は最初の開始まで RX を16 tick 以上 High に保つ。開始候補はフレームを受信していないときに4 tick 以上連続した High の直後に来る Low である。この Low を最初に適用した tick を `s` とする。

| 動作 | 16 tick/bit | 8 tick/bit |
| --- | --- | --- |
| 開始を採取する。High なら候補を破棄する | `s + 8` | `s + 4` |
| データビット `i` を採取する（`i = 0..7`） | `s + 24 + 16i` | `s + 12 + 8i` |
| 停止を採取する | `s + 152` | `s + 76` |
| 受信結果を公開する | `s + 159` | `s + 87` |
| 次の開始候補を受け付ける | `s + 160` 以降 | `s + 80` 以降 |

開始が Low で停止が High なら、受信結果の公開時に `byte` を更新して `valid` を1にする。開始と停止がともに Low なら `framing-error` を1にする。8 tick/bit では通知の配線遅延があるため、前の結果の公開より先に、直後のフレームの開始を受け付けられる。

`valid` と `framing-error` は各1 tick。両方が同時に1になることはない。`byte` は reset で0になり、次の `valid` まで値を保つ。ロード側は `valid` と同じ公開フレームの `byte` を1バイトとして受け取る。回路の別のラッチに接続するとき、ラッチはその公開フレームを次の tick の更新で読む。

## 検証

`verify-uart` は Core が実際に展開した NOR とラッチから tick の遷移式を作る。reset から二つの開始候補を置き、開始候補の Low と直前の High 以外の RX を記号化する。全 tick の `byte`、`valid`、`framing-error` を中点採取から決まる仕様と比較する。

16 tick/bit と8 tick/bit のそれぞれで連続する二つの候補を検査する。8 tick/bit では候補間をさらに4 tick 空けて、停止が Low の入力とその後の受信も検査する。Z3 の `unsat` はこの範囲の入力で差がないことを示す。実 VM の結合試験では受信器から語バッファに渡った64ビット語と、ローカル ACK の後に `ready` が下がることを両周期で確認する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart | z3 -in
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart -- --8-tick | z3 -in
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart -- --8-tick-recovery | z3 -in
```
