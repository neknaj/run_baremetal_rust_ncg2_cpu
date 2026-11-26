"""実行する ELF へ起動要求を与え、QEMU の PC 列と終了結果を記録する。"""

import argparse
from pathlib import Path
import re
import socket
import struct
import subprocess
import tempfile
import time


SOURCE = b"(using nor)\n(func invert (in a) (out b) (def b (nor a a)))"
EXPECTED_RESULT = (1, 0x07E00113)
REQUEST_ADDRESS = 0x2BF00
RESULT_ADDRESS = 0x2FFF0
HALT_ADDRESS = 0x10068


def exchange(connection, reader, message: str) -> str:
    """GDB リモートの1要求を送り、検査済みの応答を返す。"""
    payload = message.encode("ascii")
    connection.sendall(b"$" + payload + f"#{sum(payload) % 256:02x}".encode("ascii"))
    while (start := reader.read(1)) != b"$":
        if not start:
            raise ValueError("QEMU からの応答が途中で切れた。")
    response = bytearray()
    while (byte := reader.read(1)) != b"#":
        if not byte:
            raise ValueError("QEMU の応答が途中で終わった。")
        response.extend(byte)
    if int(reader.read(2), 16) != sum(response) % 256:
        raise ValueError("QEMU の応答のチェックサムが異なる。")
    connection.sendall(b"+")
    return response.decode("ascii")


def wait_for_debugger(process: subprocess.Popen, port: int) -> socket.socket:
    """起動した QEMU のデバッガー接続を待つ。"""
    for _ in range(100):
        try:
            return socket.create_connection(("127.0.0.1", port), timeout=10)
        except ConnectionRefusedError:
            if process.poll() is not None:
                raise ValueError(f"QEMU が終了した。終了コード {process.returncode}。") from None
            time.sleep(0.05)
    raise TimeoutError("QEMU のデバッガーに接続できない。")


def verify(elf: Path, trace: Path, qemu: str) -> tuple[int, int, int]:
    """固定の入力を実行し、照合できた場合だけ参照ログを出力先へ置く。"""
    request = struct.pack("<QQ128s", len(SOURCE), 0, SOURCE)
    trace.parent.mkdir(parents=True, exist_ok=True)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix=".qemu-trace-", dir=trace.parent) as staging:
        log = Path(staging) / "qemu-trace.log"
        process = subprocess.Popen([
            qemu, "-cpu", "rv64,a=false,c=false,f=false,d=false,zawrs=false,zfa=false",
            "-one-insn-per-tb", "-d", "exec,nochain", "-D", log, "-g", str(port), elf,
        ])
        try:
            with wait_for_debugger(process, port) as connection, connection.makefile("rb") as reader:
                reply = exchange(connection, reader, f"M{REQUEST_ADDRESS:x},{len(request):x}:{request.hex()}")
                if reply != "OK":
                    raise ValueError(f"入力要求をセットできない。応答 {reply}。")
                reply = exchange(connection, reader, f"Z0,{HALT_ADDRESS:x},4")
                if reply != "OK" or not exchange(connection, reader, "c").startswith("T05"):
                    raise ValueError("終了ループに到達しなかった。")
                registers = exchange(connection, reader, "g")
                # RV64 の汎用レジスタ32個に続けて、PC が64ビットで並ぶ。
                pc = int.from_bytes(bytes.fromhex(registers[512:528]), "little")
                if pc != HALT_ADDRESS:
                    raise ValueError(f"停止した PC が異なる。PC {pc:#x}。")
                reply = exchange(connection, reader, f"m{RESULT_ADDRESS:x},10")
                status, info = struct.unpack("<QQ", bytes.fromhex(reply))
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
        if (status, info) != EXPECTED_RESULT:
            raise ValueError(f"終了結果が異なる。status={status:#x} information={info:#x}。")
        pcs = re.findall(rb"^Trace \d+: 0x[0-9a-f]+ \[[0-9a-f]+/([0-9a-f]+)/", log.read_bytes(), re.M)
        if not pcs or int(pcs[0], 16) != 0x10000:
            raise ValueError("参照ログから起動時の PC を取得できない。")
        count = len(pcs) - (int(pcs[-1], 16) == HALT_ADDRESS)
        log.replace(trace)
    return status, info, count


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--qemu", default="qemu-riscv64")
    args = parser.parse_args()
    if args.elf.resolve() == args.trace.resolve():
        parser.error("ELF と参照ログのパスは別に指定する。")
    status, info, count = verify(args.elf, args.trace, args.qemu)
    print(f"status={status:#x} information={info:#x} 命令数={count}")


if __name__ == "__main__":
    main()
