"""裸 RV64IM の除算フィクスチャを QEMU で実行し、命令列と結果 RAM を照合する。"""

import argparse
from pathlib import Path
import re
import socket
import struct
import subprocess
import tempfile

from elf_image import load_segments, write_output
from verify_selfhost_qemu import exchange, wait_for_debugger


ENTRY = 0x10000
HALT = 0x10068
RESULT = 0x103F00
RAM_END = 0x104000
ISA_MASK = 0xFE00707F
DIVISION = {
    0x02004033, 0x02005033, 0x02006033, 0x02007033,
    0x0200403B, 0x0200503B, 0x0200603B, 0x0200703B,
}
MULTIPLY = {0x02000033, 0x02003033}
EXPECTED = Path(__file__).resolve().parent.parent / "verification" / "fixtures" / "division-results.inc"
TRACE_PC = re.compile(rb"^Trace \d+: 0x[0-9a-f]+ \[[0-9a-f]+/([0-9a-f]+)/", re.M)


def expected_ram() -> tuple[int, ...]:
    """期待値22語、予約領域のゼロ、起動時の結果2語を返す。"""
    values = []
    for line in EXPECTED.read_text(encoding="utf-8").splitlines():
        directive = line.split("#", 1)[0].split()
        if directive:
            name, value = directive
            if name != ".quad":
                raise ValueError(f"期待値に .quad 以外の指定がある。{line}")
            values.append(int(value, 16))
    if len(values) != 22:
        raise ValueError("除算の期待値は22語必要。")
    return (*values, *(0,) * (30 - len(values)), 1, len(values))


def check_trace(image: bytes, trace: bytes) -> int:
    """実行した命令語から、8符号化、乗算2種と x0 宛てを確認する。"""
    pcs = [int(pc, 16) for pc in TRACE_PC.findall(trace)]
    if not pcs or pcs[0] != ENTRY:
        raise ValueError("QEMU の参照ログが入口から始まらない。")
    if pcs[-1] == HALT:
        pcs.pop()
    if HALT in pcs:
        raise ValueError("終了位置に達した後も実行を続けた。")

    segments = [segment for segment in load_segments(image, image_end=RAM_END) if segment.executable]
    seen = set()
    wrote_x0 = False
    for pc in pcs:
        segment = next(
            (item for item in segments if item.address <= pc and pc + 4 <= item.address + item.file_size),
            None,
        )
        if segment is None or pc % 4:
            raise ValueError(f"実行した命令が ELF の命令領域にない。PC={pc:#x}。")
        position = segment.offset + pc - segment.address
        instruction = int.from_bytes(image[position:position + 4], "little")
        encoding = instruction & ISA_MASK
        seen.add(encoding)
        if encoding in DIVISION and (instruction >> 7) & 31 == 0:
            wrote_x0 = True
    if not (DIVISION | MULTIPLY) <= seen or not wrote_x0:
        raise ValueError("除算8種、MUL/MULHU、x0 宛ての除算をすべて実行できていない。")
    return len(pcs)


def verify(elf: Path, trace: Path, qemu: str) -> tuple[int, int, int]:
    """停止位置と結果 RAM を検査し、照合した PC 列を保存する。"""
    expectation = expected_ram()
    trace.parent.mkdir(parents=True, exist_ok=True)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix=".division-qemu-", dir=trace.parent) as staging:
        log = Path(staging) / "qemu.log"
        process = subprocess.Popen([
            qemu, "-cpu", "rv64,a=false,c=false,f=false,d=false,zawrs=false,zfa=false",
            "-one-insn-per-tb", "-d", "exec,nochain", "-D", log, "-g", str(port), elf,
        ])
        try:
            with wait_for_debugger(process, port) as connection, connection.makefile("rb") as reader:
                reply = exchange(connection, reader, f"Z0,{HALT:x},4")
                if reply != "OK" or not exchange(connection, reader, "c").startswith("T05"):
                    raise ValueError("除算フィクスチャが終了位置に到達しなかった。")
                registers = exchange(connection, reader, "g")
                pc = int.from_bytes(bytes.fromhex(registers[512:528]), "little")
                if pc != HALT:
                    raise ValueError(f"停止した PC が異なる。PC={pc:#x}。")
                reply = exchange(connection, reader, f"m{RESULT:x},{RAM_END - RESULT:x}")
                actual = struct.unpack("<32Q", bytes.fromhex(reply))
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
        for index, (got, want) in enumerate(zip(actual, expectation)):
            if got != want:
                raise ValueError(
                    f"RAM {RESULT + 8 * index:#x}: 実際値 {got:#018x}、期待値 {want:#018x}。"
                    f"状態 {actual[-2]:#x}、情報 {actual[-1]:#x}。"
                )
        retired = check_trace(elf.read_bytes(), log.read_bytes())
        log.replace(trace)
    return actual[-2], actual[-1], retired


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--qemu", default="qemu-riscv64")
    parser.add_argument("--result-out", type=Path, help="状態と情報をリトルエンディアンで保存する。")
    args = parser.parse_args()
    paths = [path.resolve() for path in (args.elf, args.trace, args.result_out) if path is not None]
    if len(paths) != len(set(paths)) or EXPECTED.resolve() in paths:
        parser.error("ELF、ログ、結果のパスをそれぞれ別に指定する。")
    try:
        status, information, retired = verify(args.elf, args.trace, args.qemu)
        if args.result_out is not None:
            args.result_out.parent.mkdir(parents=True, exist_ok=True)
            write_output(args.result_out, struct.pack("<QQ", status, information))
    except (OSError, ValueError, struct.error, subprocess.SubprocessError) as error:
        parser.exit(1, f"{error}\n")
    print(f"status={status:#x} information={information:#x} 命令数={retired}")


if __name__ == "__main__":
    main()
