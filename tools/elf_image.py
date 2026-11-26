"""固定アドレス向け RV64IM ELF を、回路の開発時メモリイメージへ変換する。"""

import argparse
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
import struct
import sys
import tempfile
from typing import Iterator


RAM_BASE = 0x10000
RAM_END = 0x30000
RAM_SIZE = RAM_END - RAM_BASE
WORD_SIZE = 8
WORD_BITS = WORD_SIZE * 8
WORD_COUNT = RAM_SIZE // WORD_SIZE
ADDRESS_BITS = (WORD_COUNT - 1).bit_length()
ENTRY = RAM_BASE
ROM_ADDRESS_BITS = 5
ROM_WORDS = 1 << ROM_ADDRESS_BITS
ROM_END = RAM_BASE + ROM_WORDS * WORD_SIZE
WRITABLE_BANKS = ((0x11000, 0x12000), (0x2C000, RAM_END))

ELF_HEADER = struct.Struct("<16sHHIQQQIHHHHHH")
PROGRAM_HEADER = struct.Struct("<IIQQQQQQ")
ET_EXEC = 2
EM_RISCV = 243
PT_LOAD = 1
PT_DYNAMIC = 2
PT_INTERP = 3
PF_X = 1
PF_W = 2


class ErrorKind(Enum):
    HEADER = "elf-header"
    TARGET = "elf-target"
    SEGMENT = "load-segment"
    ENTRY = "entry-point"


class ElfError(ValueError):
    """不正な ELF の種別と、該当するプログラムヘッダー番号を持つ。"""

    def __init__(self, kind: ErrorKind, message: str, header_index: int | None = None):
        self.kind = kind
        self.header_index = header_index
        super().__init__(message)


@dataclass(frozen=True)
class RomLayout:
    """ROM の添字幅、読み出し区間の上端、ゼロ初期化 RAM の区間を持つ。"""

    address_bits: int
    rom_end: int
    writable_banks: tuple[tuple[int, int], ...]


ROM_LAYOUTS = {
    "boot": RomLayout(ROM_ADDRESS_BITS, ROM_END, WRITABLE_BANKS),
    # 32 KiB分の ROM を構成するが、0x16000 からは RAM を参照する。
    "core": RomLayout(12, 0x16000, ((0x16000, 0x19000), (0x2BF00, RAM_END))),
}


@dataclass(frozen=True)
class Segment:
    """PT_LOAD が RAM に占める範囲と、初期値を供給するファイル上の位置。"""

    address: int
    size: int
    offset: int
    file_size: int
    executable: bool
    writable: bool
    header_index: int


def load_segments(elf: bytes) -> tuple[Segment, ...]:
    """固定メモリ配置と整合する PT_LOAD を、開始アドレス順に返す。

    ファイルの抜け、ソースや RAM の範囲外、重複、先頭命令を置けない配置は ElfError を返す。
    """
    if len(elf) < ELF_HEADER.size:
        raise ElfError(ErrorKind.HEADER, "ELF ヘッダーが途中で終わっている。")
    (ident, kind, machine, version, entry, phoff, _, flags, ehsize, phentsize, phnum, *_) = ELF_HEADER.unpack_from(elf)
    if ident[:4] != b"\x7fELF" or ident[6] != 1:
        raise ElfError(ErrorKind.HEADER, "ELF の識別子または識別部の版が不正。")
    if ident[4:6] != b"\x02\x01" or kind != ET_EXEC or machine != EM_RISCV:
        raise ElfError(ErrorKind.TARGET, "64ビット・リトルエンディアンの RISC-V ET_EXEC が必要。")
    if version != 1 or ehsize != ELF_HEADER.size:
        raise ElfError(ErrorKind.HEADER, "ELF ヘッダーの版またはサイズが不正。")
    # この起動形式は圧縮命令や浮動小数点 ABI を指定する ELF フラグを受け付けない。
    if flags != 0:
        raise ElfError(ErrorKind.TARGET, f"未対応の RISC-V ELF フラグ {flags:#x}。")
    if entry != ENTRY:
        raise ElfError(ErrorKind.ENTRY, f"入口 {entry:#x} がリセット時の PC {ENTRY:#x} と異なる。")
    if (phnum in (0, 0xFFFF) or phentsize != PROGRAM_HEADER.size
            or phoff < ELF_HEADER.size or phoff + phnum * phentsize > len(elf)):
        raise ElfError(ErrorKind.HEADER, "プログラムヘッダーテーブルのサイズまたは位置が不正。")

    segments = []
    for index in range(phnum):
        row = PROGRAM_HEADER.unpack_from(elf, phoff + index * phentsize)
        seg_type, seg_flags, offset, vaddr, paddr, file_size, mem_size, align = row
        if seg_type in (PT_INTERP, PT_DYNAMIC):
            raise ElfError(ErrorKind.SEGMENT, "動的リンクを必要とするプログラムヘッダーは扱わない。", index)
        if seg_type != PT_LOAD:
            continue
        if file_size > mem_size or offset + file_size > len(elf):
            raise ElfError(ErrorKind.SEGMENT, "ロードするバイト範囲がファイルまたはメモリサイズを越える。", index)
        # PC とデータのアドレスは同じ物理アドレス空間で扱う。
        if vaddr != paddr or not RAM_BASE <= vaddr <= vaddr + mem_size <= RAM_END:
            raise ElfError(ErrorKind.SEGMENT, "仮想アドレスと物理アドレスが異なるか、指定 RAM の範囲外。", index)
        if align > 1 and ((align & (align - 1)) != 0 or vaddr % align != offset % align):
            raise ElfError(ErrorKind.SEGMENT, "PT_LOAD の整列条件を満たしていない。", index)
        if mem_size:
            segments.append(Segment(vaddr, mem_size, offset, file_size,
                                    bool(seg_flags & PF_X), bool(seg_flags & PF_W), index))

    segments.sort(key=lambda segment: segment.address)
    end = RAM_BASE
    for segment in segments:
        if segment.address < end:
            raise ElfError(ErrorKind.SEGMENT, "PT_LOAD のメモリ範囲が重複している。", segment.header_index)
        end = segment.address + segment.size
    if not any(segment.executable and segment.address <= entry
               and entry + 4 <= segment.address + segment.file_size for segment in segments):
        raise ElfError(ErrorKind.ENTRY, "入口を含む実行可能な初期化済み命令がない。")
    return tuple(segments)


def memory_image(elf: bytes, *, layout: RomLayout | None = None) -> bytes:
    """128 KiBのイメージを作る。layout を指定した場合は各領域を対応するバンクに限定する。"""
    segments = load_segments(elf)
    image = bytearray(RAM_SIZE)
    for segment in segments:
        if layout is not None:
            end = segment.address + segment.size
            if segment.writable:
                fits = (not segment.executable and segment.file_size == 0
                        and any(base <= segment.address < end <= limit
                                for base, limit in layout.writable_banks))
            else:
                fits = RAM_BASE <= segment.address < end <= layout.rom_end
            if not fits:
                raise ElfError(ErrorKind.SEGMENT, "ロード領域が起動 ROM またはゼロ初期化 RAM に収まらない。",
                               segment.header_index)
        start = segment.address - RAM_BASE
        image[start:start + segment.file_size] = elf[segment.offset:segment.offset + segment.file_size]
    return bytes(image)


def nonzero_words(image: bytes) -> Iterator[tuple[int, int]]:
    """RAM の非ゼロ64ビット語を、RAM 相対の語添字の昇順で返す。"""
    if len(image) != RAM_SIZE:
        raise ValueError(f"RAM イメージの長さは {RAM_SIZE} バイトである必要がある。")
    for index, (word,) in enumerate(struct.iter_unpack("<Q", image)):
        if word:
            yield index, word


def ncg2m_source(image: bytes) -> bytes:
    """ゼロ既定の語添字とデータ値の組を返す NCG2M 補助関数を作る。"""
    lines = [
        f"; RAM {RAM_BASE:#x}..{RAM_END:#x}、リセット PC {ENTRY:#x}。",
        "; 各行は64ビット語の添字と値。省略した語の値は0。",
        "(defn firmware-nonzero-words",
        "  (as-type (Fn () (List (List Nat)))",
        "    (fn ()",
        "      (list",
    ]
    lines.extend(f"        (list {index} {word})" for index, word in nonzero_words(image))
    lines.append("      ))))")
    return ("\n".join(lines) + "\n").encode("utf-8")


def word_bits(image: bytes) -> bytes:
    """語添字14ビット、値64ビットの順に並べた LSB-first の行を作る。"""
    lines = (
        f"{index:0{ADDRESS_BITS}b}"[::-1] + f"{word:0{WORD_BITS}b}"[::-1]
        for index, word in nonzero_words(image)
    )
    return "".join(f"{line}\n" for line in lines).encode("ascii")


def ncg2_rom_source(image: bytes, *, address_bits: int = ROM_ADDRESS_BITS) -> bytes:
    """イメージの先頭を読む NCG2 回路を生成し、同じ選択回路と NOR を共有する。"""
    if not 1 <= address_bits <= ADDRESS_BITS:
        raise ValueError("ROM の語添字はイメージを表せる幅にする。")
    word_count = 1 << address_bits
    words = dict(nonzero_words(image))
    if any(index >= word_count for index in words):
        raise ElfError(ErrorKind.SEGMENT, f"起動 ROM の先頭{word_count * WORD_SIZE}バイトより後ろに非ゼロの初期値がある。")

    lines = [
        "(namespace rv64",
        "  (export firmware-rom)",
        "",
        "  ;; 起動 ELF の初期化済みデータを、下位バイトから並べた64ビット語で読む。",
        "  ;; tools/elf_image.py --format ncg2-rom で更新する。",
        f"  (func firmware-rom (in (word-index (bits {address_bits}))) (out (word (bits 64)))",
        "    (def zero 0)",
        "    (def one 1)",
    ]
    for bit in range(address_bits):
        lines.append(f"    (def select-{bit} (bit word-index {bit}))")
    gates: dict[tuple[str, str], str] = {}

    def nor(left: str, right: str) -> str:
        if left == "one" or right == "one":
            return "zero"
        if left == right == "zero":
            return "one"
        if left > right:
            left, right = right, left
        key = (left, right)
        if key not in gates:
            name = f"gate-{len(gates)}"
            lines.append(f"    (def {name} (nor {left} {right}))")
            gates[key] = name
        return gates[key]

    def select(bit: int, low: str, high: str) -> str:
        if low == high:
            return low
        address = f"select-{bit}"
        inverse = nor(address, address)
        if low == "zero" and high == "one":
            return address
        if low == "one" and high == "zero":
            return inverse
        take_low = nor(address, nor(low, low))
        take_high = nor(inverse, nor(high, high))
        neither = nor(take_low, take_high)
        return nor(neither, neither)

    # 添字の下位ビットから組み合わせ、異なる語や出力ビットにも共通する回路を使う。
    output_bits = []
    for bit in range(WORD_BITS):
        stage = ["one" if (words.get(index, 0) >> bit) & 1 else "zero"
                 for index in range(word_count)]
        for address_bit in range(address_bits):
            stage = [select(address_bit, low, high) for low, high in zip(stage[::2], stage[1::2])]
        output_bits.append(stage[0])
    for byte in range(WORD_SIZE):
        signals = " ".join(output_bits[byte * 8:(byte + 1) * 8])
        lines.append(f"    (def byte-{byte} (concat {signals}))")
    bytes_in_order = " ".join(f"byte-{byte}" for byte in range(WORD_SIZE))
    lines.append(f"    (def word (concat {bytes_in_order}))))")
    return ("\n".join(lines) + "\n").encode("utf-8")


def write_output(path: Path, data: bytes) -> None:
    """書き込みが完了した成果物を出力先に置く。"""
    with tempfile.TemporaryDirectory(prefix=".elf-image-", dir=path.parent) as staging:
        candidate = Path(staging) / "image"
        candidate.write_bytes(data)
        candidate.replace(path)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("output", type=Path)
    encoders = {
        "image": bytes, "ncg2m": ncg2m_source, "word-bits": word_bits,
        "ncg2-rom": ncg2_rom_source,
    }
    parser.add_argument("--format", choices=encoders, default="image")
    parser.add_argument("--rom-layout", choices=ROM_LAYOUTS, help="ncg2-rom の物理アドレス配置。既定は boot。")
    args = parser.parse_args()
    if args.elf.resolve() == args.output.resolve():
        parser.error("出力先を入力 ELF と同じファイルにできない。")
    if args.rom_layout is not None and args.format != "ncg2-rom":
        parser.error("--rom-layout は --format ncg2-rom とともに指定する。")
    try:
        elf = args.elf.read_bytes()
        if args.format == "ncg2-rom":
            layout = ROM_LAYOUTS[args.rom_layout or "boot"]
            image = memory_image(elf, layout=layout)
            data = ncg2_rom_source(image, address_bits=layout.address_bits)
        else:
            data = encoders[args.format](memory_image(elf))
        write_output(args.output, data)
    except ElfError as error:
        location = "" if error.header_index is None else f" PT[{error.header_index}]"
        print(f"{error.kind.value}{location} {error}", file=sys.stderr)
        return 1
    except OSError as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
