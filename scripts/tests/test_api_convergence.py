import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "api_convergence", Path(__file__).resolve().parents[1] / "check-api-convergence.py"
)
convergence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(convergence)


class LegacyCompatCallTests(unittest.TestCase):
    def test_product_callers_are_rejected_and_retained_paths_stay_open(self):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root, True)
        allowed = root / "crates/nana-ui-runtime/src/framework"
        allowed.mkdir(parents=True)
        (allowed / "virtualize.rs").write_text(
            "pub fn materialize_virtual_list() {}\n"
            "cx.materialize_virtual_list_retained_in();\n",
            encoding="utf-8",
        )
        (allowed / "tests.rs").write_text(
            'Subscription::new("window.events", stream);\n',
            encoding="utf-8",
        )
        product = root / "examples/app/src"
        product.mkdir(parents=True)
        (product / "main.rs").write_text(
            "context.materialize_virtual_list(list, items);\n"
            "context.materialize_virtual_list_retained_in();\n"
            'Subscription::new("id", stream);\n',
            encoding="utf-8",
        )

        self.assertEqual(
            convergence.legacy_compat_calls(root),
            [
                "examples/app/src/main.rs:1: legacy virtual materializer",
                "examples/app/src/main.rs:3: Subscription::new",
            ],
        )
