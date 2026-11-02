import unittest

from boot_smoke import pci_checks, run_label


class PciEvidenceTests(unittest.TestCase):
    function = (
        "PCI: function bus=1 slot=0 function=0 vendor=0x1af4 device=0x1042 "
        "class=0x01 command=0x0000 bars=6\n"
    )
    summary = "PCI: inventory complete functions=1 virtio_block=1 command_unchanged=true\n"

    def test_modern_block_device_behind_bridge(self):
        self.assertTrue(all(pci_checks(self.function + self.summary, True).values()))

    def test_probe_requires_nonzero_bus(self):
        checks = pci_checks(self.function.replace("bus=1", "bus=0") + self.summary, True)
        self.assertTrue(checks["pci_inventory"])
        self.assertFalse(checks["pci_storage_behind_bridge"])

    def test_missing_or_changed_command_is_not_success(self):
        for summary in ("", self.summary.replace("true", "false")):
            self.assertFalse(pci_checks(self.function + summary)["pci_inventory"])

    def test_duplicate_functions_or_wrong_counts_fail(self):
        for log in (self.function * 2 + self.summary.replace("functions=1", "functions=2"),
                    self.function + self.summary.replace("functions=1", "functions=2"),
                    self.function + self.summary.replace("virtio_block=1", "virtio_block=2")):
            self.assertFalse(pci_checks(log)["pci_inventory"])

    def test_no_storage_device_is_valid_without_fixture(self):
        log = self.function.replace("vendor=0x1af4", "vendor=0x8086")
        log += self.summary.replace("virtio_block=1", "virtio_block=0")
        self.assertTrue(pci_checks(log)["pci_inventory"])
        self.assertFalse(pci_checks(log, True)["pci_storage_behind_bridge"])

    def test_probe_evidence_does_not_overwrite_ordinary_run(self):
        plain = run_label("debug", "max", True, 4, "129G")
        self.assertNotEqual(plain, run_label("debug", "max", True, 4, "129G", True))


if __name__ == "__main__":
    unittest.main()
