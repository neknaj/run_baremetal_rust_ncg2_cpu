# 目次

[はじめに](introduction.md)

# Rust の実行と UART ブート

- [自作のベアメタル Rust を動かす](run-your-rust.md)
- [Rust 起動回路の構成と検証](firmware-execution.md)
  - [起動 ELF のメモリ配置](firmware-memory.md)
- [UART ブートローダーの ROM](uart-boot.md)
  - [UART 受信器](uart-rx.md)
  - [UART の受信語バッファ](uart-word-buffer.md)
  - [UART ブート用 MMIO](uart-boot-mmio.md)
  - [UART ブート用 CPU の命令フェッチ](uart-cpu.md)

# CPU の構成

- [起動回路の部品を試す](boot-components.md)
- [起動用の命令実行回路](boot-execution.md)
  - [PC 相対の命令と条件分岐](control-execution.md)
  - [命令完了から更新要求まで](retire-control.md)
- [整数 ALU](integer-alu.md)
  - [MUL 回路](multiply.md)
  - [符号なし128ビット乗算](multiply-wide.md)
  - [整数除算の演算回路](division.md)
- [部分読み出しと部分書き込み](subword-memory.md)

# NCG2 Core のセルフホスト

- [Core を組み込んだ RV64IM ELF](selfhost.md)
  - [Core 起動用メモリ](selfhost-memory.md)
  - [Core の起動要求を RAM に設定する](selfhost-preload.md)
- [変化した配線から順序回路を実行する](event-clocked.md)
