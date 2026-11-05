import unittest

from boot_betafs import betafs_checks, completion


class BetafsBootTests(unittest.TestCase):
    def test_complete_markers_cannot_hide_a_kernel_fault(self):
        for fault in ("PANIC\n", "#PF fault\n", "#GP\n", "#DF\n"):
            self.assertEqual(completion("all markers\n" + fault, {"complete": True})[0], "failed")
            self.assertEqual(completion(fault, {"complete": False})[0], "failed")
        self.assertEqual(completion("USERDIAG: process=0x1 bytes=6 text=PANIC\\x0a\n",
                                    {"complete": True})[0], "passed")

    def test_phase_and_trusted_marker_counts_are_required(self):
        markers = [
            "BETAFS: native Raw Zstd LZ4 OpenZL and engine checks OK\n",
            "BETAFS: blocked lock owner death recovery OK\n",
            "BETAFS: service driver and backing retirement OK\n",
            "BETAFS: file sync_data sync_all and directory barriers OK phase=write\n",
        ]
        valid = "".join(markers)
        self.assertTrue(all(betafs_checks(valid, "format").values()))
        self.assertFalse(all(betafs_checks(valid, "open").values()))
        for marker in markers:
            self.assertFalse(all(betafs_checks(valid.replace(marker, ""), "format").values()))
            self.assertFalse(all(betafs_checks(valid + marker, "format").values()))
        self.assertFalse(any(betafs_checks("USERDIAG: process=0x1 bytes=0 text=" + markers[0], "format").values()))
