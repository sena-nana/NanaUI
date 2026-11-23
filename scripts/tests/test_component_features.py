import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("component_features", Path(__file__).resolve().parents[1] / "check-component-features.py")
features = importlib.util.module_from_spec(spec)
spec.loader.exec_module(features)


class ComponentFeatureTableTests(unittest.TestCase):
    def test_table_matches_the_components_feature(self):
        self.assertEqual(features.table_problems(list(features.FAMILIES)), [])

    def test_a_family_without_a_row_and_a_stale_row_are_both_reported(self):
        listed = [family for family in features.FAMILIES if family != "qr-code"] + ["new-family"]
        problems = features.table_problems(listed)
        self.assertEqual(len(problems), 2)
        self.assertIn("`new-family` is in nana-ui's `components`", problems[0])
        self.assertIn("`qr-code` is not in nana-ui's `components`", problems[1])

    def test_every_probe_names_what_it_expects_missing(self):
        for family, row in features.FAMILIES.items():
            with self.subTest(family=family):
                self.assertEqual(row.probe is None, row.missing is None)
                self.assertEqual(row.probe is None, row.error is None)
                if row.probe:
                    self.assertIn(row.missing, row.probe)
                else:
                    self.assertTrue(row.dependencies, "a family without an API probe needs a dependency check")

    def test_tree_crates_reads_names_not_substrings(self):
        crates = features.tree_crates("nana-ui v0.1.0 (/x)\nswash-like v1.0.0\nqrcode v0.14.1\n")
        self.assertEqual(crates, {"nana-ui", "swash-like", "qrcode"})
        self.assertNotIn("swash", crates)


if __name__ == "__main__":
    unittest.main()
