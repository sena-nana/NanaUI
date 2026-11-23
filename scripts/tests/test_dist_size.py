import contextlib
import importlib.util
import io
import json
import shutil
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("dist_size", Path(__file__).resolve().parents[1] / "check-dist-size.py")
dist_size = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dist_size)


class DistSizeTests(unittest.TestCase):
    def test_growth_within_the_tolerance_passes_and_past_it_fails(self):
        self.assertTrue(dist_size.judge(1_050_000, 1_000_000, 0.05)[0])
        ok, line = dist_size.judge(1_050_001, 1_000_000, 0.05)
        self.assertFalse(ok)
        self.assertIn("over the 5% limit of 1,050,000 B", line)

    def test_a_large_shrink_passes_and_asks_for_a_re_record(self):
        ok, line = dist_size.judge(900_000, 1_000_000, 0.05)
        self.assertTrue(ok)
        self.assertIn("--update", line)

    def run_gate(self, size, recorded, *extra):
        directory = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, directory, True)
        artifact = directory / "app"
        artifact.write_bytes(b"\0" * size)
        baseline = directory / "dist-size.json"
        baseline.write_text(json.dumps({"tolerance": 0.05, "platforms": {"test-os": {str(artifact): recorded}}}))
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = dist_size.main(["--baseline", str(baseline), "--platform", "test-os", *extra])
        return code, json.loads(baseline.read_text())["platforms"]["test-os"][str(artifact)]

    def test_gate_exit_codes(self):
        self.assertEqual(self.run_gate(1000, 1000)[0], 0)
        self.assertEqual(self.run_gate(1100, 1000)[0], 1)
        code, _ = self.run_gate(1000, 1000, "--platform", "other-os")
        self.assertEqual(code, 2)

    def test_update_records_the_measured_size(self):
        self.assertEqual(self.run_gate(1100, 1000, "--update"), (0, 1100))


if __name__ == "__main__":
    unittest.main()
