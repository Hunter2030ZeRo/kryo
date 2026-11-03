import struct
import tempfile
import unittest
from pathlib import Path

from build_user import load_image


class MappedHeaderTests(unittest.TestCase):
    def image(self, path, *, section_offset=512, entry=0x401000):
        data = bytearray(512)
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<QQ", data, 24, entry, 64)
        struct.pack_into("<Q", data, 40, section_offset)
        struct.pack_into("<HHH", data, 52, 64, 56, 1)
        struct.pack_into("<HHH", data, 58, 64, 3, 2)
        struct.pack_into("<IIQQQQQQ", data, 64, 1, 4, 0, 0x400000, 0x400000, 512, 512, 4096)
        path.write_bytes(data)

    def test_only_section_table_fields_can_change_in_mapped_header(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory) / "first", Path(directory) / "second"
            self.image(first)
            self.image(second, section_offset=256)
            self.assertNotEqual(load_image(first), load_image(second))
            self.assertEqual(load_image(first, normalize_sections=True), load_image(second, normalize_sections=True))
            data = bytearray(second.read_bytes())
            struct.pack_into("<HHH", data, 58, 0, 0, 0)
            second.write_bytes(data)
            self.assertEqual(load_image(first, normalize_sections=True), load_image(second, normalize_sections=True))

    def test_runtime_header_program_headers_and_payload_remain_exact(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory) / "first", Path(directory) / "second"
            self.image(first)
            expected = load_image(first, normalize_sections=True)
            for offset in (16, 24, 64 + 4, 64 + 16, 200):
                data = bytearray(first.read_bytes())
                data[offset] ^= 1
                second.write_bytes(data)
                self.assertNotEqual(expected, load_image(second, normalize_sections=True))


if __name__ == "__main__":
    unittest.main()
