# Core の起動要求を RAM に設定する

`selfhost-preloaded-memory` は、起動時に `selfhost-preload` の要求をメモリのデータポートへ渡す。入力には、QEMU の検証で使った58バイトのソースと、値が両方0の NOR 入力を指定する。長さと入力ビットの2語、UTF-8 のソースと末尾のゼロを含む16語を、`0x2bf00` から連続する18語に書く。[RAM 上の入出力](selfhost.md#ram-上の入出力)

## フレームの切り替え

リセットで添字と `ready` は0に戻る。リセット後の tick 42 から37 tick ごとに18件の書き込みを指定する。最後のフレームは tick 671 で、選択回路の4段を通り、次の tick にメモリへ到達する。同じフレームの書き込みは RAM に17 tick 後に反映されるため、18語目の確定は tick 693 になる。[メモリの書き込み時刻](selfhost-memory.md#接続と-tick)

| tick | 状態 |
| ---: | --- |
| 42 | 添字0の書き込みを指定する |
| 671 | 添字17の書き込みを指定する |
| 693 | 最後の語が RAM に確定する |
| 715 | アドレスとデータを CPU 側に切り替える |
| 723 | `ready` が立ち、次の入力フレームから CPU の書き込みを受け付ける |

最初のフレームは32段の遅延から開始パルスを作って指定する。以降は書き込みパルスを32段のラッチ、2段の OR、3段の許可回路へ戻す。添字が更新されてから20段以内に要求の値が安定するため、次のパルスは前の語を参照しない。添字18に達したら新しい要求を止め、メモリへの書き込みが確定する時間を取ってから切り替える。

選択経路と書き込み許可経路には、各フレームの信号からメモリの入口まで同じ4段がある。CPU の書き込みは入力から21 tick 後に確定する。読み出しは元のメモリの待機時間に4 tick 加え、同じ Core ELF の ROM では53 tick 待つ。切り替え完了までは CPU ポートの書き込みを抑止し、その間に返る読み出し値は CPU 入力に対する結果として使わない。命令フェッチは ROM の別経路を使う。

## 検証

`selfhost-preload.toml` は検証用の小さな ROM で切り替えを試す設定である。組み合わせ回路は添字の全32通りを要求のバイト列と照合する。Z3 は回路を展開した NOR と切り替え仕様の一致を調べる。停止判定の全入力経路が7段、アドレス・値・許可の全経路が4段であることも検証器が確認する。

VM 結合テストでは全書き込みのフレームと終了時刻、RAM に入った18語、未使用の14語を確認する。初期化を妨げる CPU 書き込みの抑止、切り替え後の通常書き込み、リセット後の再初期化も調べる。

```sh
set -e
ncg2 check --project selfhost-preload.toml
host=$(rustc +1.99.1 -vV | sed -n 's/^host: //p')
mkdir -p target/verification
model=target/verification/selfhost-preload.smt2
cargo +1.99.1 run --quiet --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --example verify_selfhost_preload > "$model"
test "$(z3 -smt2 "$model")" = unsat
cargo +1.99.1 test --locked --manifest-path verification/Cargo.toml \
  --target "$host" --target-dir target/verification --test selfhost_preload
```
