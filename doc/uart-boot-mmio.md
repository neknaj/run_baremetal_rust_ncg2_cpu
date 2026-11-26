# UART ブート用 MMIO

`::rv64::uart-boot-mmio` は、[UART 受信器](uart-rx.md)と[受信語バッファ](uart-word-buffer.md)を CPU のデータバスにつなぐ。入力はシリアル線 `rx`、64ビットの `address`、`write-enable`。出力は64ビットの `read-data` と `ready`、`error`。CPU 側は確定した64ビットストアの tick にだけ `write-enable` を1にする。書き込むデータは参照しない。

| アドレス | 読み出し | 確定した64ビットストア |
| --- | --- | --- |
| `0x200000` | STATUS。ビット0は `ready`、ビット1は `error`、残りは0 | 受信状態を保持 |
| `0x200008` | DATA。最後に登録した64ビットの受信語 | 受信状態を保持 |
| `0x200010` | 0 | ACK を発行 |
| それ以外 | 0 | 受信状態を保持 |

読出しは受信状態を変えない。DATA の値は `ready` が1のときに受信語として扱う。CPU は STATUS のエラーを確認してから DATA を読み、その語に1回 ACK する。エラー発生時はブートローダーのエラー経路へ進む。

入力 tick `t` のアドレスに対応する `read-data` は `t + 19` に確定する。この時点で公開する `ready` と `error` は、STATUS が返す値と同じ受信状態を観測する。アドレスと書き込み許可は同じ入力 tick から取る。受信語を保持しているとき、ACK の確定効果はストアの入力 tick から35 tick 後に公開される。

送信側は reset 後に16 tick 以上アイドル High を維持する。8N1 の1ビットは16 tickとする。8バイトで1語を組み立てる。前の語に対する ACK が反映されるまでに次の語が完成しないよう、送信時に語間の待ち時間を設ける。

## 検証

`verify-uart-mmio` は、公開モジュールが使うバスポートを実際に NOR とラッチへ展開する。全64ビットのアドレス、書き込み許可、受信語と状態信号を tick ごとに任意とし、読み出し値と ACK をアドレスの完全一致で定めた参照仕様と比較する。reset から19 tick までに加え、任意の初期状態から20 tick 後の一致も検査する。この20 tick の窓を後続の任意の時刻に適用できる。実 VM の試験ではシリアル線から受信し、読み出しに副作用がないこと、別アドレスのストアで ACK しないこと、正しい ACK とフレーミングエラーを確認する。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 test --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart-mmio
cargo +1.99.1 run --locked --release --manifest-path verification/Cargo.toml \
  --target "$host" --bin verify-uart-mmio | z3 -in
```
