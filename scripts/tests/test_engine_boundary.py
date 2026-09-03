import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("boundary", Path(__file__).resolve().parents[1] / "check-engine-boundary.py")
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)

class EngineBoundaryTests(unittest.TestCase):
    def graph(self, edges, versions=None):
        names = sorted({"nana-ui-runtime", *edges, *(child for children in edges.values() for child in children)})
        return {
            "workspace_members": ["nana-ui-runtime"],
            "packages": [{"id": name, "name": name.split("@")[0], "version": (versions or {}).get(name, "1.0.0")} for name in names],
            "resolve": {"nodes": [{"id": name, "deps": [{"pkg": child, "dep_kinds": [{"kind": None}]} for child in edges.get(name, [])]} for name in names]},
        }
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
    def test_ci_requires_real_targets_and_features(self):
        packages = {"host": {"targets": [{"name": "gpu", "kind": ["bin"]}], "features": {"gpu": []}}}
        self.assertEqual(boundary.check_cargo_commands("cargo run -p host --features gpu --bin gpu", packages, "ci"), [])
        self.assertEqual(len(boundary.check_cargo_commands("cargo run -p host --features live2d --bin removed", packages, "ci")), 2)

if __name__ == "__main__":
    unittest.main()
