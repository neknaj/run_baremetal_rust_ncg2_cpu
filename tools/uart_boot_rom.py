"""GNU binutils で作ったブートローダーの .text から NCG2 ROM を生成する。"""

import argparse
from pathlib import Path

from elf_image import RAM_SIZE, ncg2_rom_source, write_output


BOOT_BYTES = 128


def main() -> None:
    """4バイト境界の命令を128バイトの読み出し専用回路に収める。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("text", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    if args.text.resolve() == args.output.resolve():
        parser.error("入力と出力には別のファイルを指定する。")
    data = args.text.read_bytes()
    if not data or len(data) > BOOT_BYTES or len(data) % 4:
        parser.error("ブートコードは4バイトから128バイトまでの4の倍数とする。")
    rom = ncg2_rom_source(data.ljust(RAM_SIZE, b"\0"), address_bits=4)
    rom = rom.decode("utf-8").replace("firmware-rom", "uart-boot-rom")
    rom = rom.replace(
        "起動 ELF の初期化済みデータを、下位バイトから並べた64ビット語で読む。",
        "UART ブートローダーの命令を、下位バイトから並べた64ビット語で読む。",
    ).replace(
        "tools/elf_image.py --format ncg2-rom で更新する。",
        "tools/uart_boot_rom.py で programs/uart-boot.S の組立結果から更新する。",
    )
    write_output(args.output, rom.encode("utf-8"))


if __name__ == "__main__":
    main()
