import unittest

from boot_smoke import abi_checks, block_checks, block_service_checks, cpu_checks, diagnostic_checks, heap_checks, libc_checks, observed_checks, pci_checks, run_label, std_checks


def diagnostic_log():
    payloads = [b"C native calls ready",
                b"\nPANIC\r\x1b\\\0\x7f\x80ABC\nABI: startup and execution resources retired",
                b"D" * 256, b"Rust diagnostic bridge ready"]
    lines = []
    for payload in payloads:
        text = "".join("\\\\" if value == 92 else chr(value) if 32 <= value <= 126
                       else f"\\x{value:02x}" for value in payload)
        lines.append(f"USERDIAG: process=0x123 bytes={len(payload)} text={text}\n")
    return "".join(lines)


class PciEvidenceTests(unittest.TestCase):
    def test_rust_std_requires_trusted_runtime_and_retirement_markers(self):
        ready = "RUSTSTD: allocator threads synchronization TLS monotonic time OK runs=2\n"
        retired = "RUSTSTD: process and backing retirement OK\n"
        aborted = "RUSTSTD: native abort termination OK\n"
        self.assertEqual(std_checks("", False), {})
        self.assertTrue(all(std_checks(ready + retired + aborted, True).values()))
        for invalid in (ready, retired, ready + retired, ready + aborted, retired + aborted,
                        ready + ready + retired + aborted,
                        ready + retired + retired + aborted,
                        ready + retired + aborted + aborted,
                        ready.replace("runs=2", "runs=1") + retired + aborted,
                        "USERDIAG: process=0x1 bytes=0 text=" + ready + retired + aborted,
                        ready + retired + "USERDIAG: process=0x1 bytes=0 text=" + aborted,
                        "USERLOG: process=0x1 event=227 value=1\n"):
            self.assertFalse(all(std_checks(invalid, True).values()))

    def test_rust_std_has_separate_evidence_directory(self):
        self.assertNotEqual(run_label("debug", libc_probe=True),
                            run_label("debug", libc_probe=True, std_probe=True))

    def test_libc_requires_singleton_trusted_kernel_markers(self):
        ready = "LIBC: static mlibc startup TLS allocator formatting OK runs=3\n"
        retired = "LIBC: process and backing retirement OK\n"
        threads = "LIBCTHREAD: provider TLS join detach and retirement OK children=14 reapers=1\n"
        failures = "LIBCTHREAD: last exit callbacks and wait quota recovery OK\n"
        stdio = "LIBCSTDIO: buffer allocation failure and recovery OK runs=3\n"
        files = "LIBCFILE: C Rust open read write seek and closure OK runs=3\n"
        descriptors = "LIBCFD: shared descriptions dup flags and replacement OK runs=3\n"
        self.assertEqual(libc_checks("", False), {})
        self.assertTrue(all(libc_checks(ready + retired + threads + failures + stdio + files + descriptors, True).values()))
        for invalid in (ready, retired, ready + ready + retired,
                        ready + retired + retired, ready.replace("runs=3", "runs=2") + retired,
                        "USERDIAG: process=0x1 bytes=0 text=" + ready + retired):
            self.assertFalse(all(libc_checks(invalid, True).values()))
        for invalid in (ready + retired + threads + failures,
                        ready + retired + threads + failures + stdio + stdio,
                        "USERDIAG: process=0x1 bytes=0 text=" + stdio):
            self.assertFalse(libc_checks(invalid, True)["libc_stdio_recovery"])
        for invalid in (ready + retired, ready + retired + threads + threads,
                        ready + retired + threads.replace("children=14", "children=13"),
                        ready + retired + threads + failures + failures):
            self.assertFalse(all(libc_checks(invalid, True).values()))

    def test_libc_file_marker_cannot_be_duplicated_or_forged(self):
        marker = "LIBCFILE: C Rust open read write seek and closure OK runs=3\n"
        for invalid in ("", marker + marker, marker.replace("runs=3", "runs=2"),
                        "USERDIAG: process=0x1 bytes=0 text=" + marker):
            self.assertFalse(libc_checks(invalid, True)["libc_file_service"])
        self.assertTrue(libc_checks(marker, True)["libc_file_service"])

    def test_descriptor_marker_requires_one_trusted_complete_record(self):
        marker = "LIBCFD: shared descriptions dup flags and replacement OK runs=3\n"
        for invalid in ("", marker + marker, marker.replace("runs=3", "runs=2"),
                        "USERDIAG: process=0x1 bytes=0 text=" + marker):
            self.assertFalse(libc_checks(invalid, True)["libc_descriptors"])
        self.assertTrue(libc_checks(marker, True)["libc_descriptors"])

    def test_libc_has_separate_evidence_and_diagnostic_scope(self):
        self.assertNotEqual(run_label("debug", abi_probe=True),
                            run_label("debug", abi_probe=True, libc_probe=True))
        log = diagnostic_log() + "ABI: startup and execution resources retired\n"
        log += "USERDIAG: process=0x123 bytes=5 text=mlibc\n"
        self.assertTrue(abi_checks(log, True)["native_c_diagnostics"])

    def test_heap_checks_require_packing_and_all_cpu_retirement(self):
        packed = "HEAP: packed objects=128 bytes=64 backing_pages=8 baseline_pages=128 retained_pages=2\n"
        retired = "HEAP: concurrent allocation and retirement OK cpus=4\n"
        self.assertTrue(all(heap_checks(packed + retired, 4).values()))
        for invalid in (packed, retired, packed + retired + retired,
                        packed + retired.replace("cpus=4", "cpus=1"),
                        packed.replace("backing_pages=8", "backing_pages=128") + retired,
                        "USERDIAG: process=0x123 bytes=0 text=" + packed + retired):
            self.assertFalse(all(heap_checks(invalid, 4).values()))

    def test_abi_requires_preemption_and_actual_retirement(self):
        ready = "ABI: v3 SysV C C++ Rust stack and state OK preemptions=1\n"
        retired = "ABI: startup and execution resources retired\n"
        tls = "TLS: ELF template and thread isolation OK threads=3\n"
        vm = "VM: private reserve commit retirement OK\n"
        words = "WAIT: private atomic park wake retirement OK threads=13 halt=true\n"
        threads = "THREAD: runtime-owned stack TLS and rounding OK threads=2\n"
        native_c = "CABI: native C calls and diagnostic retirement OK\n"
        c_threads = "CTHREAD: native C create join and backing retirement OK threads=2\n"
        c_ipc = "CIPC: C Rust capability and shared memory retirement OK\n"
        c_reply = "CREPLY: C Rust capability reply retirement OK caps=2\n"
        c_gate = "CGATE: C Rust labels closure and retirement OK\n"
        markers = [diagnostic_log(), ready, retired, tls, vm, words, threads, native_c, c_threads, c_ipc, c_reply, c_gate]
        log = "".join(markers)
        self.assertTrue(all(abi_checks(log, True).values()))
        self.assertFalse(all(abi_checks(log.replace("preemptions=1", "preemptions=0"), True).values()))
        for index in range(len(markers)):
            missing = "".join(markers[:index] + markers[index + 1:])
            self.assertFalse(all(abi_checks(missing, True).values()))
        self.assertEqual(abi_checks("", False), {})

    def test_private_vm_requires_retirement_not_only_user_event(self):
        log = "USERLOG: process=0x100 event=95 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_private_vm"])
        log += "VM: private reserve commit retirement OK\n"
        self.assertTrue(abi_checks(log, True)["native_private_vm"])

    def test_atomic_wait_requires_kernel_retirement_proof(self):
        log = "USERLOG: process=0x100 event=96 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_atomic_wait"])
        log += "WAIT: private atomic park wake retirement OK threads=13 halt=true\n"
        self.assertTrue(abi_checks(log, True)["native_atomic_wait"])

    def test_runtime_threads_require_kernel_retirement_proof(self):
        log = "USERLOG: process=0x100 event=97 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_runtime_threads"])
        log += "THREAD: runtime-owned stack TLS and rounding OK threads=2\n"
        self.assertTrue(abi_checks(log, True)["native_runtime_threads"])
        self.assertFalse(abi_checks(log.replace("threads=2", "threads=1"), True)["native_runtime_threads"])

    def test_diagnostic_bytes_and_process_identity_are_verified(self):
        log = diagnostic_log()
        self.assertTrue(diagnostic_checks(log)["native_c_diagnostics"])
        for invalid in (log.replace("bytes=20", "bytes=19"),
                        log.replace("\\x0a", "\\xzz", 1),
                        log.replace("process=0x123", "process=0x999", 1),
                        log + "USERDIAG: invalid\n", log + log,
                        log.replace(" text=", " text=\x1b", 1)):
            self.assertFalse(diagnostic_checks(invalid)["native_c_diagnostics"])

    def test_user_diagnostics_cannot_forge_kernel_success_markers(self):
        log = diagnostic_log()
        self.assertFalse(abi_checks(log, True)["native_abi_retired"])
        forged = "USERDIAG: process=0x123 bytes=0 text=DEVICE: register authority and IRQ wakeup OK cases=4\n"
        self.assertFalse(observed_checks(forged, normal=True)["device"])
        log += "ABI: startup and execution resources retired\n"
        self.assertTrue(abi_checks(log, True)["native_abi_retired"])

    def test_native_c_event_is_not_kernel_retirement_evidence(self):
        log = diagnostic_log() + "USERLOG: process=0x123 event=98 value=1\n"
        self.assertFalse(abi_checks(log, True)["native_c_calls"])
        log += "CABI: native C calls and diagnostic retirement OK\n"
        self.assertTrue(abi_checks(log, True)["native_c_calls"])

    def test_c_thread_evidence_requires_complete_kernel_retirement(self):
        marker = "CTHREAD: native C create join and backing retirement OK threads=2\n"
        for log in ("USERLOG: process=0x123 event=99 value=2\n",
                    "USERDIAG: process=0x123 bytes=0 text=" + marker,
                    marker.replace("threads=2", "threads=1")):
            self.assertFalse(abi_checks(log, True)["native_c_threads"])
        self.assertTrue(abi_checks(marker, True)["native_c_threads"])

    def test_c_ipc_evidence_requires_single_kernel_retirement_marker(self):
        marker = "CIPC: C Rust capability and shared memory retirement OK\n"
        for invalid in ("USERLOG: process=0x123 event=100 value=1\n",
                        "USERDIAG: process=0x123 bytes=0 text=" + marker,
                        marker + marker):
            self.assertFalse(abi_checks(invalid, True)["native_c_ipc"])
        self.assertTrue(abi_checks(marker, True)["native_c_ipc"])

    def test_reply_caps_requires_kernel_retirement_proof(self):
        marker = "CREPLY: C Rust capability reply retirement OK caps=2\n"
        for invalid in ("USERLOG: process=0x123 event=101 value=2\n",
                        "USERDIAG: process=0x123 bytes=0 text=" + marker,
                        marker + marker, marker.replace("caps=2", "caps=1")):
            self.assertFalse(abi_checks(invalid, True)["native_c_reply_caps"])
        self.assertTrue(abi_checks(marker, True)["native_c_reply_caps"])
        core = "REPLYCAPS: atomic grants quota timeout and retirement OK cases=6\n"
        self.assertFalse(observed_checks("USERDIAG: process=0x123 bytes=0 text=" + core)["reply_caps"])
        self.assertTrue(observed_checks(core)["reply_caps"])
        legacy = "REPLYCAPS: legacy exchange register inputs preserved\n"
        self.assertTrue(observed_checks(legacy)["legacy_exchange_registers"])
        self.assertFalse(observed_checks("USERDIAG: process=0x1 bytes=0 text=" + legacy)["legacy_exchange_registers"])

    def test_gate_evidence_requires_kernel_markers(self):
        c_gate = "CGATE: C Rust labels closure and retirement OK\n"
        for invalid in ("USERLOG: process=0x1 event=102 value=1\n",
                        "USERDIAG: process=0x1 bytes=0 text=" + c_gate, c_gate + c_gate):
            self.assertFalse(abi_checks(invalid, True)["native_c_gates"])
        self.assertTrue(abi_checks(c_gate, True)["native_c_gates"])
        markers = {
            "endpoint_gates": "GATE: labels lifetime delegation quota and retirement OK\n",
            "gate_loans": "GATE: queued send call and accepted timeout loans OK cases=3\n",
            "gate_process_retirement": "GATE: final process handle retirement wakes receiver without polling\n",
        }
        for key, marker in markers.items():
            self.assertTrue(observed_checks(marker)[key])
            self.assertFalse(observed_checks("USERDIAG: process=0x1 bytes=0 text=" + marker)[key])

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
