import unittest

from boot_smoke import block_checks, pci_checks, run_label


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

    def test_block_mode_requires_both_instances_and_correct_readonly_mode(self):
        log = "BLOCK: Ring3 commands and private DMA retirement OK cases=2\n"
        log += "BLOCK: driver complete mode=0 readonly=true\n"
        self.assertFalse(all(block_checks(log, True, True).values()))
        log += "BLOCK: driver complete mode=1 readonly=true\n"
        self.assertTrue(all(block_checks(log, True, True).values()))
        self.assertFalse(all(block_checks(log, False, True).values()))

    def test_boot_tests_require_cleanup_retry_evidence(self):
        log = "BLOCK: Ring3 commands and private DMA retirement OK cases=2\n"
        log += "BLOCK: driver complete mode=0 readonly=false\nBLOCK: driver complete mode=1 readonly=false\n"
        self.assertFalse(all(block_checks(log, False, False).values()))
        log += "BLOCK: cleanup retry retains queue and memory\n"
        self.assertTrue(all(block_checks(log, False, False).values()))

    def test_block_modes_have_separate_evidence_directories(self):
        rw = run_label("debug", block_probe=True)
        ro = run_label("debug", block_probe=True, block_readonly=True)
        self.assertNotEqual(rw, ro)
        self.assertNotEqual(rw, run_label("debug", pci_probe=True))


if __name__ == "__main__":
    unittest.main()
