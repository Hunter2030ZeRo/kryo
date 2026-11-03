import unittest

from boot_smoke import abi_checks, block_checks, block_service_checks, cpu_checks, pci_checks, run_label


class PciEvidenceTests(unittest.TestCase):
    def test_abi_requires_preemption_and_actual_retirement(self):
        ready = "ABI: v3 SysV C C++ Rust stack and state OK preemptions=1\n"
        retired = "ABI: startup and execution resources retired\n"
        tls = "TLS: ELF template and thread isolation OK threads=3\n"
        vm = "VM: private reserve commit retirement OK\n"
        words = "WAIT: private atomic park wake retirement OK threads=11 halt=true\n"
        self.assertTrue(all(abi_checks(ready + retired + tls + vm + words, True).values()))
        for log in (ready + tls + vm + words, retired + tls + vm + words, ready.replace("=1", "=0") + retired + tls + vm + words, ready + retired + vm + words, ready + retired + tls + words, ready + retired + tls + vm):
            self.assertFalse(all(abi_checks(log, True).values()))
        self.assertEqual(abi_checks("", False), {})

    def test_private_vm_requires_retirement_not_only_user_event(self):
        log = "USERLOG: process=0x100 event=95 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_private_vm"])
        log += "VM: private reserve commit retirement OK\n"
        self.assertTrue(abi_checks(log, True)["native_private_vm"])

    def test_atomic_wait_requires_kernel_retirement_proof(self):
        log = "USERLOG: process=0x100 event=96 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_atomic_wait"])
        log += "WAIT: private atomic park wake retirement OK threads=11 halt=true\n"
        self.assertTrue(abi_checks(log, True)["native_atomic_wait"])

    def test_abi_requires_three_thread_tls_proof(self):
        log = "ABI: v3 SysV C C++ Rust stack and state OK preemptions=2\n"
        log += "ABI: startup and execution resources retired\n"
        log += "TLS: ELF template and thread isolation OK threads=2\n"
        self.assertFalse(abi_checks(log, True)["native_tls"])
        self.assertTrue(abi_checks(log.replace("threads=2", "threads=3"), True)["native_tls"])

    def test_abi_evidence_does_not_replace_normal_boot(self):
        self.assertNotEqual(run_label("debug"), run_label("debug", abi_probe=True))

    def test_cpu_baseline_requires_each_logical_cpu_once(self):
        log = "".join(f"CPU: x86-64-v3 hardware admitted cpu={cpu}\n" for cpu in (2, 0, 3, 1))
        self.assertTrue(cpu_checks(log, 4)["cpu_baseline"])
        for invalid in ("", log.replace("cpu=3", "cpu=2"), log + log, log.replace("cpu=3", "cpu=4")):
            self.assertFalse(cpu_checks(invalid, 4)["cpu_baseline"])

    def test_service_checks_bind_events_to_published_driver(self):
        log = "BLOCK IPC: service dispatched id=0xabc\nBLOCK IPC: accepted cancellation retains loan\nBLOCK IPC: clients complete; service waiting\n"
        log += "USERLOG: process=0xabc event=91 value=1\nUSERLOG: process=0xabc event=92 value=1\n"
        for mode in (0, 1, 3): log += f"USERLOG: process=0xdef event=93 value={mode}\n"
        self.assertTrue(all(block_service_checks(log, True).values()))
        self.assertFalse(all(block_service_checks(log.replace("event=92", "event=99"), True).values()))
        self.assertFalse(all(block_service_checks(log.replace("id=0xabc", "id=0x777"), True).values()))

    def test_service_boot_checks_require_actual_retirement_marker(self):
        checks = block_service_checks("", False)
        self.assertFalse(checks["block_service_retired"])
        self.assertTrue(block_service_checks("BLOCK IPC: graceful stop and resource retirement OK", False)["block_service_retired"])

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
