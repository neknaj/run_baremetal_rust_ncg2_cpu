# UART ブート用 CPU の命令フェッチ

`::rv64::uart-firmware-machine` は物理 PC を `0x0` に初期化し、ブート ROM と書き込み可能な RAM から命令を取得する。アプリは `[0x10000, 0x52000)` の4バイト境界で実行できる。命令を取得できない PC では停止信号 `stopped` が立つ。ROM の未使用語も不正命令として停止する。[ブート ROM の配置と受信プロトコル](uart-boot.md)

## PC とメモリポート

既存の実行部には物理 PC から `0x10000` を引いた相対 PC を渡し、更新先に `0x10000` を足す。いずれも64ビットの演算である。したがって、`AUIPC` が加算に使う値、`JAL` と `JALR` のリンク値、`JALR` の飛び先は物理アドレスになる。`commit` と同じ tick の `pc` は実行済み命令のアドレスで、次の tick に更新先が現れる。

`bus-address` と `read-data` は1つのポートを共有する。`issue` は初期化時と命令を完了したあとに立つ CPU 内部信号である。

| 時点 | ポートの動作 |
| --- | --- |
| `issue + 7` | フェッチのアドレスが確定する。PC の下位3ビットを0にし、命令のある64ビット語を読む。 |
| `issue + 64` | RAM の語を `read-data` に確定させる期限。この値を命令の取り込みまで安定させる。 |
| `issue + 72` | 選択済みの32ビット命令と有効ビットをラッチに取り込む。次のフェッチまで保持する。 |
| `issue + 79` | ポートは実行部のデータアドレスを選ぶ。このアドレスはオペランドが確定してから決まる。 |

RAM の語に対する応答時間はバスアドレスの確定から57 tick 以内とする。PC のビット2で語の下位32ビットか上位32ビットを選ぶ。ROM や範囲外をフェッチする期間にも PC に対応するアドレスを外部に送る。外部回路は読み出し自体に副作用を持たせず、`bus-write` に合わせて同じフレームの `bus-address` と `bus-write-data` を取り込む。RAM は同じ語の書き込みと読み出しが重なれば更新前の値を返す。[NCG2 の状態素子と read-first](https://neknaj.github.io/circuitgame2/docs/simulation.html)

## MMIO のアクセス幅

UART の語レジスタは表に示す命令と物理アドレスの組み合わせでアクセスできる。

| レジスタ | アドレス | 命令 |
| --- | --- | --- |
| STATUS | `0x200000` | `LD` |
| DATA | `0x200008` | `LD` |
| ACK | `0x200010` | `SD` |

語内の一部にアクセスするロードとストアは RAM に限る。MMIO のアドレスに1〜7を加えた位置や、表にない命令でアクセスした場合は、命令を無効とし、レジスタとメモリへの書き込みを抑止する。[UART の語レジスタ](uart-boot-mmio.md)

## 検証

SMT 検証器はPC の変換と RAM フェッチの実 NOR を展開する。64ビットの加減算、フェッチ可能な範囲と整列、語内の選択を独立したビットベクトル式と照合する。既存の命令実行部と組み合わせ、`AUIPC`・`JAL`・`JALR` の物理アドレスでの結果と、飛び先が4バイト境界を外れた場合の挙動も照合する。

命令全体の検証器は旧2プロファイルと UART プロファイルをそれぞれ展開する。有効判定、次の PC、レジスタやメモリへの書き込み要求を、プロファイルごとの独立したアドレス仕様と照合する。UART では LD と SD の許可と、部分アクセスや別アドレスから ACK を消費できないことも照合する。

VM のテストはブート ROM に UART の受信語を供給し、書き込んだ実 RAM からアプリを実行する。同じ語を更新する tick では更新前の値を確認し、アプリに移ったあとは連続した PC とレジスタ書き込み値を照合する。テスト用の UART は語インターフェースで代用し、RAM は先頭の4語を扱う。

```sh
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --example verify_uart_pc | z3 -in
for profile in firmware-dispatch firmware-dispatch-core firmware-dispatch-uart; do
  cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
    --target "$host" --bin ncg2-rv64-arithmetic-proof -- "$profile" | z3 -in
done
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --test uart_entry
```

## RAM 書き込み前のアドレス保持

`verify_uart_store` は `uart-boot.toml` の入口を階層展開する。バスアドレス、書き込み許可、RAM に入る書き込みデータを根とし、ラッチの出力と CPU に戻るデータ応答まで、実 NOR の配線をたどる。公開 PC の配線を特定した上で、残りのラッチを命令33ビット、オペランド64ビット、フェッチ選択1ビットとして区別する。この検査は、各ラッチが値を保持する期間を前提とする。

| 観測先 | 入力ごとの最長 NOR 経路 |
| --- | --- |
| バスアドレス | 命令479、`rs1` 465、PC 4、フェッチ選択4、観測用アドレス16 |
| dispatch の書き込み許可 | 命令502、`rs1` 490、`rs2` 70、PC 34、フェッチ有効4 |
| 書き込みデータ | データ応答から8 |

データ応答からアドレスと書き込み許可への組み合わせ経路はない。データ応答は部分書き込みの旧語と、次に取り込む命令の一部になる。検証器は `bus-write` のラッチ、4 tick の初期化遅延、退役パルスと dispatch の許可の論理積も実配線で確認する。

ラッチ群の値を固定し、フェッチ選択を解除した状態では、dispatch が書き込みを許した命令は自然整列した SD・SB・SH・SW のいずれかになる。各ストアのバスアドレスは任意の tick ごとの観測用アドレスに対して、`rs1 + 符号拡張した S 即値` を8バイト境界へ切り下げた値と一致する。検証器は最長16段の観測用アドレスの経路を tick ごとに展開し、非選択側の値を任意にしたまま照合する。固定した入力の SD が許可される照合結果は `sat`、違反を探す照合結果は `unsat` となる。

```sh
set -e
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/uart-store.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --example verify_uart_store > "$model"
test "$(z3 -smt2 "$model")" = "$(printf 'sat\nunsat')"
```

命令枠の開始を tick `i` とする。命令、`rs1`、`rs2` はそれぞれ `i+73`、`i+97`、`i+145` までに保持され、フェッチ選択は `i+75` までに解除される。PC は命令枠の間保持される。これらを前提にすると、ストアのアドレスは `i+562` から、書き込み許可は `i+587` から確定する。通常ストアの書き込みが `i+1033` に出るときは、その tick を含む直前32 tick の全てでアドレスが一致する。

この検査の対象は保持された値から得られるアドレスと書き込み許可である。リセット後と連続する命令枠で上の保持条件が成立すること、乗算を実行していない命令枠に `mul-finished` が早く届かないことは、CPU 全履歴の証明に残る。実行中の書き込み時の保持は結合試験でも観測する。[RAM の要求と結合試験](uart-boot.md)
