import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "check_test_hygiene", ROOT / "scripts" / "check-test-hygiene.py"
)
hygiene = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(hygiene)


def make_pkg(name: str, tests: int = 1, bin_test: bool = False, ex_test: bool = False) -> dict:
    targets = [
        {"name": f"test_{i}", "kind": ["test"], "test": True}
        for i in range(tests)
    ]
    targets.append({"name": f"{name}-bin", "kind": ["bin"], "test": bin_test})
    targets.append({"name": f"{name}-ex", "kind": ["example"], "test": ex_test})
    return {"name": name, "targets": targets}


class TestHygieneTests(unittest.TestCase):
    def test_clean_metadata_and_sources_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            test_file = root / "crates" / "demo" / "tests" / "contracts.rs"
            test_file.parent.mkdir(parents=True)
            test_file.write_text(
                """
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[test]
fn bounded_and_isolated() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let _dir = std::env::temp_dir().join(format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
    std::thread::sleep(Duration::from_millis(10));
}

#[test]
#[ignore = "soak"]
fn slow_soak_can_sleep() {
    std::thread::sleep(Duration::from_secs(5));
}
"""
            )
            meta = {"packages": [make_pkg("demo", tests=2)]}
            self.assertEqual(hygiene.evaluate(root, meta), [])

    def test_rejects_unlisted_bin_and_example_with_test_true(self):
        meta = {
            "packages": [
                make_pkg("alpha", tests=1, bin_test=True, ex_test=True),
            ]
        }
        errs = hygiene.check_metadata_targets(meta)
        self.assertTrue(any("[[bin]] `alpha-bin`" in e for e in errs))
        self.assertTrue(any("[[example]] `alpha-ex`" in e for e in errs))

    def test_rejects_excess_per_package_and_workspace_test_targets(self):
        meta_pkg = {"packages": [make_pkg("alpha", tests=5)]}
        errs_pkg = hygiene.check_metadata_targets(meta_pkg)
        self.assertTrue(any("5 integration test targets" in e for e in errs_pkg))

        meta_ws = {
            "packages": [make_pkg(f"p{i}", tests=4) for i in range(8)]
        }
        errs_ws = hygiene.check_metadata_targets(meta_ws)
        self.assertTrue(any("workspace has 32 integration test targets" in e for e in errs_ws))

    def test_rejects_issue_test_filename_but_allows_non_test_issue_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bad = root / "crates" / "demo" / "tests" / "issue999.rs"
            bad.parent.mkdir(parents=True)
            bad.write_text("#[test] fn t() {}\n")

            bad_world = root / "crates" / "nana-ui-runtime" / "src" / "world" / "issue999.rs"
            bad_world.parent.mkdir(parents=True)
            bad_world.write_text("#![cfg(test)]\n")

            ok_prod = root / "crates" / "nana-ui-runtime" / "src" / "layout_engine" / "issue258.rs"
            ok_prod.parent.mkdir(parents=True)
            ok_prod.write_text("pub fn ok() {}\n")

            errs = hygiene.check_source_hygiene(root)
            self.assertEqual(len(errs), 2)
            self.assertTrue(any("crates/demo/tests/issue999.rs" in e for e in errs))
            self.assertTrue(any("world/issue999.rs" in e for e in errs))

    def test_rejects_unbounded_poll_sleep_pid_tmpdir_and_nested_cargo(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bad = root / "crates" / "demo" / "tests" / "hangs.rs"
            bad.parent.mkdir(parents=True)
            bad.write_text(
                """
use std::process::Command;
use std::time::Duration;

#[test]
fn bad_patterns() {
    let _ = wgpu::PollType::wait_indefinitely();
    let _ = wgpu::PollType::Wait { submission_index: None, timeout: None };
    std::thread::sleep(Duration::from_secs(5));
    let _dir = std::env::temp_dir().join(format!("x-{}", std::process::id()));
    let _ = Command::new("cargo").arg("build");
}
"""
            )
            errs = hygiene.check_source_hygiene(root)
            self.assertTrue(any("wait_indefinitely" in e for e in errs), errs)
            self.assertTrue(any("without `timeout: Some" in e for e in errs), errs)
            self.assertTrue(any("blocking sleep >= 2s" in e for e in errs), errs)
            self.assertTrue(any("atomic sequence counter" in e for e in errs), errs)
            self.assertTrue(any("nested `cargo build/test/run`" in e for e in errs), errs)


if __name__ == "__main__":
    unittest.main()
