"""ELF の入力検査とオフライン出力の境界を確認する。"""

import re
import struct
import unittest

from elf_image import ErrorKind, ElfError, memory_image, ncg2_rom_source, ncg2m_source, word_bits


BASE = 0x10000
END = 0x30000


def executable(segments: list[tuple[int, ...]], payload: bytes, entry: int = BASE) -> bytes:
    """セクションヘッダーを持たない最小 ELF を独立に組み立てる。"""
    ident = b"\x7fELF\x02\x01\x01" + bytes(9)
    header = struct.pack("<16sHHIQQQIHHHHHH", ident, 2, 243, 1, entry, 64, 0, 0, 64, 56, len(segments), 0, 0, 0)
    table = b"".join(struct.pack("<IIQQQQQQ", *segment) for segment in segments)
    return (header + table).ljust(0x200, b"\0") + payload


class LoaderTests(unittest.TestCase):
    def assert_error(self, fixture: bytes, kind: ErrorKind, index: int | None = None):
        with self.assertRaises(ElfError) as outcome:
            memory_image(fixture)
        self.assertEqual((outcome.exception.kind, outcome.exception.header_index), (kind, index))

    def test_zero_fill_and_byte_order_across_segments(self):
        code = (1, 5, 0x200, BASE, BASE, 4, 8, 1)
        data = (1, 6, 0x204, BASE + 15, BASE + 15, 2, 9, 1)
        payload = b"\x13\x00\x00\x00\x80\x02"
        image = memory_image(executable([code, data], payload))
        self.assertEqual(image, b"\x13" + bytes(14) + b"\x80\x02" + bytes(END - BASE - 17))
        self.assertEqual(
            re.findall(rb"\(list (\d+) (\d+)\)", ncg2m_source(image)),
            [(b"0", b"19"), (b"1", b"9223372036854775808"), (b"2", b"2")],
        )
        self.assertEqual(word_bits(image).splitlines()[1], b"10" + b"0" * 75 + b"1")

    def test_rejects_wrong_kind_and_truncated_program_headers(self):
        code = (1, 5, 0x200, BASE, BASE, 4, 4, 1)
        fixture = bytearray(executable([code], b"1234"))
        self.assert_error(fixture[:100], ErrorKind.HEADER)
        fixture[5] = 2
        fixture[16:18] = b"\0\x02"
        fixture[18:20] = b"\0\xf3"
        fixture[20:24] = b"\0\0\0\x01"
        self.assert_error(fixture, ErrorKind.TARGET)

    def test_rejects_invalid_load_ranges(self):
        code = (1, 5, 0x200, BASE, BASE, 4, 4, 1)
        for row in (
            (1, 6, 0x204, BASE + 16, BASE + 16, 2, 1, 1),
            (1, 6, 0x206, BASE + 16, BASE + 16, 2, 2, 1),
            (1, 6, 0x204, END - 1, END - 1, 2, 2, 1),
            (1, 6, 0x204, BASE + 16, BASE + 24, 2, 2, 1),
            (1, 6, 0x204, BASE + 2, BASE + 2, 2, 2, 1),
            (1, 6, 0x204, BASE + 16, BASE + 16, 2, 2, 4096),
            (3, 0, 0, 0, 0, 0, 0, 0),
        ):
            with self.subTest(row=row):
                self.assert_error(executable([code, row], b"123456"), ErrorKind.SEGMENT, 1)

    def test_rejects_missing_initial_instruction(self):
        self.assert_error(executable([(1, 6, 0x200, BASE, BASE, 4, 4, 1)], b"1234"), ErrorKind.ENTRY)
        self.assert_error(executable([(1, 5, 0x200, BASE, BASE, 3, 4, 1)], b"123"), ErrorKind.ENTRY)

    def test_image_round_trips_to_both_word_formats(self):
        code = (1, 5, 0x200, BASE, BASE, 8, 8, 1)
        data = (1, 6, 0x208, END - 24, END - 24, 8, 24, 1)
        payload = bytes.fromhex("130000006f0000000100000002000080")
        image = memory_image(executable([code, data], payload))
        sources = [(int(i), int(value)) for i, value in re.findall(rb"\(list (\d+) (\d+)\)", ncg2m_source(image))]
        bits = [(int(row[:14][::-1], 2), int(row[14:][::-1], 2)) for row in word_bits(image).splitlines()]
        self.assertEqual(sources, bits)
        reconstructed = bytearray(END - BASE)
        for index, value in bits:
            reconstructed[index * 8:(index + 1) * 8] = value.to_bytes(8, "little")
        self.assertEqual(reconstructed, image)
        self.assertEqual(image[-16:], bytes(16))

    def test_rom_uses_file_bytes_and_separate_zeroed_banks(self):
        code = (1, 5, 0x200, BASE, BASE, 8, 8, 1)
        bss = (1, 6, 0x208, 0x11000, 0x11000, 0, 8, 1)
        stack = (1, 6, 0x208, 0x2C000, 0x2C000, 0, 0x4000, 1)
        image = memory_image(executable([code, bss, stack], bytes(range(8))), for_rom=True)
        self.assertIn(b"(def word-0 #x0706050403020100)", ncg2_rom_source(image))
        self.assertIn(b"(def word-1 #x0000000000000000)", ncg2_rom_source(image))

    def test_rom_rejects_bytes_and_zero_init_outside_their_banks(self):
        code = (1, 5, 0x200, BASE, BASE, 4, 4, 1)
        for row in (
            (1, 5, 0x204, BASE + 0xFC, BASE + 0xFC, 0, 8, 1),
            (1, 6, 0x204, 0x11000, 0x11000, 4, 8, 1),
            (1, 6, 0x204, 0x11FF8, 0x11FF8, 0, 16, 1),
            (1, 6, 0x204, 0x12000, 0x12000, 0, 8, 1),
        ):
            with self.subTest(row=row), self.assertRaises(ElfError) as outcome:
                memory_image(executable([code, row], bytes(8)), for_rom=True)
            self.assertEqual((outcome.exception.kind, outcome.exception.header_index),
                             (ErrorKind.SEGMENT, 1))


if __name__ == "__main__":
    unittest.main()
