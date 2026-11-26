# はじめに

このプロジェクトは Neknaj Circuit Game 2（NCG2）で RV64IM CPU を組み、ベアメタル Rust のプログラムを動かす。起動 ROM、命令の実行回路、RAM、UART を NCG2 で定義する。

- CPU を動かすには [README の実行手順](https://github.com/neknaj/run_baremetal_rust_ncg2_cpu#rust-の起動プログラムを実行する)を参照する。
- UART でプログラムを転送する仕組みは [UART ブートローダーの ROM](uart-boot.md)から読める。
- NCG2 Core を CPU 上で動かす方法と検証範囲は [Core を組み込んだ RV64IM ELF](selfhost.md)を参照する。

回路記述とシミュレーションの規則は [NCG2 の設計と仕様](https://neknaj.github.io/circuitgame2/docs/)で確認できる。
