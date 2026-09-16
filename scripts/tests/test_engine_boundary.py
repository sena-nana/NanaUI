import importlib.util
import shutil
import tempfile
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).resolve().parents[1] / "check-engine-boundary.py")
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)

class EngineBoundaryTests(unittest.TestCase):
    def graph(self, edges, versions=None, root="nana-ui-runtime"):
        names = sorted({root, *edges, *(child for children in edges.values() for child in children)})
        return {
            "workspace_members": [root],
            "packages": [{"id": name, "name": name.split("@")[0], "version": (versions or {}).get(name, "1.0.0")} for name in names],
            "resolve": {"nodes": [{"id": name, "deps": [{"pkg": child, "dep_kinds": [{"kind": None}]} for child in edges.get(name, [])]} for name in names]},
        }
    def text_crate(self, source):
        root = Path(tempfile.mkdtemp())
        (root / "src").mkdir()
        (root / "src" / "lib.rs").write_text(source, encoding="utf-8")
        self.addCleanup(shutil.rmtree, root, True)
        return root
    def test_transitive_backend_dependency_is_rejected(self):
        failures = boundary.check_dependency_graph(self.graph({"nana-ui-runtime": ["helper"], "helper": ["wgpu"]}))
        self.assertTrue(any("nana-ui-runtime -> helper -> wgpu" in failure for failure in failures))
    def test_dev_only_dependency_is_not_a_product_edge(self):
        data = self.graph({"nana-ui-runtime": ["wgpu"]})
        next(node for node in data["resolve"]["nodes"] if node["id"] == "nana-ui-runtime")["deps"][0]["dep_kinds"][0]["kind"] = "dev"
        self.assertEqual(boundary.check_dependency_graph(data), [])
    def test_multiple_wgpu_major_versions_are_rejected(self):
        data = self.graph({"host": ["wgpu@29", "wgpu@30"]}, {"wgpu@29": "29.0.0", "wgpu@30": "30.0.1"})
        self.assertTrue(any("multiple WGPU" in failure for failure in boundary.check_dependency_graph(data)))
    def test_nana_text_must_not_have_a_product_edge_to_the_engine_it_replaces(self):
        failures = boundary.check_dependency_graph(
            self.graph({"nana-text": ["helper"], "helper": ["cosmic-text"]}, root="nana-text")
        )
        self.assertTrue(any("nana-text -> helper -> cosmic-text" in failure for failure in failures))
    def test_nana_text_may_keep_a_dev_only_edge_to_the_reference_engine(self):
        # The cosmic reference engine lives in tests/ during the migration.
        data = self.graph({"nana-text": ["cosmic-text"]}, root="nana-text")
        next(node for node in data["resolve"]["nodes"] if node["id"] == "nana-text")["deps"][0]["dep_kinds"][0]["kind"] = "dev"
        self.assertEqual(boundary.check_dependency_graph(data), [])
    def test_other_crates_may_still_depend_on_cosmic_text(self):
        # Only nana-text is held to this rule; nana-ui is the product backend.
        self.assertEqual(
            boundary.check_dependency_graph(self.graph({"nana-ui": ["cosmic-text"]}, root="nana-ui")),
            [],
        )
    def test_naming_the_reference_engine_in_nana_text_sources_is_rejected(self):
        root = self.text_crate("pub fn shape(buffer: &cosmic_text::Buffer) {}\n")
        failures = boundary.check_text_engine_sources(root)
        self.assertTrue(any("names cosmic_text" in failure for failure in failures))
    def test_comments_may_describe_the_reference_engine_without_tripping_the_rule(self):
        root = self.text_crate("//! Not cosmic_text and not cryoglyph.\npub fn shape() {}\n")
        self.assertEqual(boundary.check_text_engine_sources(root), [])
    def test_nana_text_may_name_typography_but_not_the_style_model(self):
        root = self.text_crate("use nana_ui_core::{DirSpec, LineBreakSpec};\npub struct A(DirSpec, LineBreakSpec);\n")
        self.assertEqual(boundary.check_text_engine_sources(root), [])
        root = self.text_crate("use nana_ui_core::LayoutStyle;\npub struct A(LayoutStyle);\n")
        failures = boundary.check_text_engine_sources(root)
        self.assertTrue(any("nana_ui_core::LayoutStyle" in failure for failure in failures))
    def test_a_product_crate_reaching_a_reference_only_crate_is_rejected(self):
        # The package name, not the lib target name: a synthetic graph using
        # "css-parity" would pass while the real rule never fires.
        data = self.graph({"nana-ui": ["nana-css-parity"]}, root="nana-ui")
        failures = boundary.check_reference_only_packages(data)
        self.assertTrue(any("nana-css-parity is reachable from nana-ui" in failure for failure in failures))
        self.assertIn("nana-css-parity", boundary.REFERENCE_ONLY_PACKAGES)
    def test_ci_requires_real_targets_and_features(self):
        packages = {"host": {"targets": [{"name": "gpu", "kind": ["bin"]}], "features": {"gpu": []}}}
        self.assertEqual(boundary.check_cargo_commands("cargo run -p host --features gpu --bin gpu", packages, "ci"), [])
        self.assertEqual(len(boundary.check_cargo_commands("cargo run -p host --features live2d --bin removed", packages, "ci")), 2)

if __name__ == "__main__":
    unittest.main()
