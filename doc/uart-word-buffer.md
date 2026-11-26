# UART の受信語バッファ

`::rv64::uart-word-buffer` は、[UART 受信器](uart-rx.md)の `byte`、`valid`、`framing-error` と CPU 側の `ack` を入力に取る。公開出力は64ビットの `word`、1ビットの `ready`、1ビットの `error`。reset 後は全部0で、受信途中のバイトも破棄する。

`valid` が1の tick ごとに1バイトを採取する。最初の `valid` は reset から16 tick 以降とし、以後の `valid` も16 tick 以上空ける。採取順に8バイトずつ1語にし、最初のバイトを最下位に置く。`framing-error` はエラーを記録し、バイトの採取数は `valid` だけで進む。

入力 tick `t` の効果は `t + 12` の公開フレームに現れる。ACK は1 tick のパルスで、入力 tick の開始時点で公開 `ready` が1なら受け付ける。CPU は `ready` を確認して `word` を読み、1回 ACK する。次の ACK はその効果が公開された後に、次の語を読み出してから送る。

| 入力の組み合わせ | 更新後の状態 |
| --- | --- |
| ACK だけ | 未消費の語を消費して `ready` を0にする。`word` の値を保持する |
| 8バイト目だけ、未消費の語なし | 新語を登録し、`ready` を1にする |
| 8バイト目だけ、未消費の語あり | 旧語と `ready` を保持し、`error` を1にする。完成した新語は破棄する |
| 受け付けた ACK と8バイト目 | 旧語を消費し、新語を登録する。`ready` を1に保つ |
| `framing-error` が1 | `error` を1にする |

`error` に1が入ると reset まで保持する。ACK は `error` を消去しない。公開 `ready` が0の期間に ACK を送っても、組み立て途中の語やまだ公開されていない語を消費しない。

CPU 側は STATUS のビット0を `ready`、ビット1を `error` として読み、DATA を `LD` して ACK へ `SD` する。受信バッファが保持できる完成語は1個。16 tick/bit で間隔を空けず送信すると、1280 tick ごとに1語が完成する。CPU が前の語を消費する前に次の語を完成させないよう、送信側は語の間に待ち時間を入れる。

## 検証

`verify-uart-word` は実際に展開した NOR とラッチを使い、24バイトを16 tick 間隔で受け取る期間を検査する。バイト値、各 tick の ACK とフレーミングエラーを任意とし、公開出力の全ビットを独立した参照仕様と全 tick で比較する。任意長の通信列に対する帰納的な保証は検証範囲に含まれない。実 VM の試験では、公開前の ACK、完成と同時の ACK、未消費語の保護、エラーの保持を確認する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart-word
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart-word | z3 -in
```
