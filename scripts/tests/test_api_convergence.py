import importlib.util
import tempfile
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "api_convergence", Path(__file__).resolve().parents[1] / "check-api-convergence.py"
)
convergence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(convergence)


class LegacyCompatCallTests(unittest.TestCase):
    def test_retained_materializers_are_not_legacy_calls(self):
        retained = "\n".join(
            [
                "cx.materialize_virtual_list_retained_in(",
                "cx.materialize_virtual_list_retained_with(",
                "cx.materialize_virtual_table_retained_in(",
                "cx.materialize_virtual_tree_retained_in(",
            ]
        )
        self.assertIsNone(convergence.LEGACY_MATERIALIZE.search(retained))

    def test_unplaced_materializers_and_subscription_are_legacy_calls(self):
        self.assertIsNotNone(convergence.LEGACY_MATERIALIZE.search("cx.materialize_virtual_list("))
        self.assertIsNotNone(
            convergence.LEGACY_MATERIALIZE.search("cx.materialize_virtual_table_in(")
        )
        self.assertIsNotNone(convergence.LEGACY_MATERIALIZE.search("materialize_virtual_tree"))
        self.assertIsNotNone(convergence.LEGACY_SUBSCRIPTION.search('Subscription::new("id", stream)'))

    def test_new_product_caller_is_rejected_and_allowlisted_files_are_not(self):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(root, True))
        allowed = root / "crates/nana-ui-runtime/src/framework"
        allowed.mkdir(parents=True)
        (allowed / "virtualize.rs").write_text(
            "pub fn materialize_virtual_list() {}\n", encoding="utf-8"
        )
        (allowed / "tests.rs").write_text(
            'Subscription::new("window.events", stream);\n', encoding="utf-8"
        )
        product = root / "examples/app/src"
        product.mkdir(parents=True)
        (product / "main.rs").write_text(
            "context.materialize_virtual_list(list, items);\n", encoding="utf-8"
        )

        hits = convergence.legacy_compat_calls(root)

        self.assertEqual(hits, ["examples/app/src/main.rs:1: legacy virtual materializer"])
