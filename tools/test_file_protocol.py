import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class FileProtocolTests(unittest.TestCase):
    def test_c_and_rust_publish_the_same_wire_values(self):
        rust = (ROOT / "interfaces/src/file.rs").read_text()
        header = (ROOT / "userspace/libsys/include/cryolite/file-protocol.h").read_text()
        values = {name: int(value, 0) for name, value in re.findall(
            r"^#define KRYO_FILE_(\w+) (0x[0-9a-fA-F]+|[0-9]+)[UL]*$", header, re.M)}
        for name, value in re.findall(r"^pub const (\w+): u(?:32|64) = (0x[0-9a-fA-F]+|[0-9]+);$", rust, re.M):
            self.assertEqual(values[name], int(value, 0), name)
        for enum in ("Operation", "Status"):
            body = re.search(r"pub enum " + enum + r" \{([^}]+)\}", rust).group(1)
            for name, value in re.findall(r"(\w+) = ([0-9]+)", body):
                macro = re.sub(r"(?<!^)(?=[A-Z])", "_", name).upper()
                self.assertEqual(values[macro], int(value), macro)
