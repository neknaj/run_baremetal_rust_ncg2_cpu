"""実行する ELF へ起動要求を与え、QEMU の PC 列と終了結果を記録する。"""

import argparse
from pathlib import Path
import re
import socket
import struct
import subprocess
import tempfile
import time

from elf_image import write_output


SOURCE = b"(using nor)\n(func invert (in a) (out b) (def b (nor a a)))"
REQUEST_FORMAT = struct.Struct("<QQ128s")
REQUEST = REQUEST_FORMAT.pack(len(SOURCE), 0, SOURCE)
PROFILES = {
    # 要求の物理アドレス、結果の物理アドレス、期待する結果2語。
    "syntax": (0x2BF00, 0x2FFF0, (1, 0x07E00113)),
    "compile-circuit": (0x51F00, 0x103FF0, (1, 0x2B800113)),
}
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


def verify(
    elf: Path, trace: Path, qemu: str, profile: str = "syntax", *, request: bytes | None = None
) -> tuple[int, int, int]:
    """要求を実行して結果を返し、終了位置を確認してから参照ログを保存する。

    要求を省略した場合は、同じプロファイルの既知の結果とも照合する。
    """
    request_address, result_address, expected = PROFILES[profile]
    if request is None:
        data = REQUEST
    else:
        if len(request) != REQUEST_FORMAT.size:
            raise ValueError(f"起動要求には{REQUEST_FORMAT.size}バイト必要。")
        data = request
        expected = None
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
                # Windows のボリュームへ WSL から PC 列を書くと、停止応答まで10秒を超える。
                connection.settimeout(120)
                reply = exchange(connection, reader, f"M{request_address:x},{len(data):x}:{data.hex()}")
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
                reply = exchange(connection, reader, f"m{result_address:x},10")
                status, info = struct.unpack("<QQ", bytes.fromhex(reply))
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
        if expected is not None and (status, info) != expected:
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
    parser.add_argument("--profile", choices=PROFILES, default="syntax")
    parser.add_argument("--source", type=Path, help="起動要求に含める回路ソース。最大128バイト。")
    parser.add_argument("--inputs", type=int, help="回路に与える入力ビットを下位から詰めた整数。既定は0。")
    parser.add_argument("--request-out", type=Path, help="QEMU へ書いた要求を UART 転送用に保存する。")
    parser.add_argument("--result-out", type=Path, help="QEMU の結果2語をリトルエンディアンで保存する。")
    args = parser.parse_args()
    paths = [path.resolve() for path in (args.elf, args.trace, args.source, args.request_out, args.result_out)
             if path is not None]
    if len(paths) != len(set(paths)):
        parser.error("入力と出力はそれぞれ別のパスで指定する。")
    request = None
    if args.source is not None or args.inputs is not None:
        inputs = args.inputs if args.inputs is not None else 0
        max_inputs = 1 if args.profile == "compile-circuit" else 3
        if not 0 <= inputs <= max_inputs:
            parser.error(f"このプロファイルの入力は0から{max_inputs}まで。")
        try:
            source = SOURCE
            if args.source is not None:
                with args.source.open("rb") as source_file:
                    source = source_file.read(129)
        except OSError as error:
            parser.error(f"回路ソースを読み込めない。{error}")
        if len(source) > 128:
            parser.error("回路ソースが128バイトを超える。")
        request = REQUEST_FORMAT.pack(len(source), inputs, source)
    status, info, count = verify(args.elf, args.trace, args.qemu, args.profile, request=request)
    if args.request_out is not None:
        args.request_out.parent.mkdir(parents=True, exist_ok=True)
        write_output(args.request_out, REQUEST if request is None else request)
    if args.result_out is not None:
        args.result_out.parent.mkdir(parents=True, exist_ok=True)
        write_output(args.result_out, struct.pack("<QQ", status, info))
    print(f"status={status:#x} information={info:#x} 命令数={count}")


if __name__ == "__main__":
    main()
