import importlib.util
from pathlib import Path
import sys
import tomllib
import unittest

spec = importlib.util.spec_from_file_location(
    "publish_crates", Path(__file__).resolve().parents[1] / "publish-crates.py"
)
publish_crates = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = publish_crates
spec.loader.exec_module(publish_crates)


class PublishCratesTests(unittest.TestCase):
    def test_all_37_crates_resolve_non_empty_versions(self):
        self.assertEqual(len(publish_crates.ALL_CRATES), 37)
        ws_version = publish_crates.read_workspace_version()
        by_name = {crate.name: crate for crate in publish_crates.ALL_CRATES}
        for crate in publish_crates.ALL_CRATES:
            self.assertTrue(crate.version, f"{crate.name} resolved empty version")
            self.assertTrue(crate.manifest.is_file(), f"missing manifest: {crate.manifest}")
        self.assertEqual(by_name["nana-ui"].version, ws_version)
        self.assertEqual(by_name["nana-icons-tabler"].version, ws_version)

    def test_topological_layers_respect_all_non_dev_dependencies(self):
        root_manifest = tomllib.loads((publish_crates.ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        ws_deps = root_manifest.get("workspace", {}).get("dependencies", {})
        by_name = {crate.name: crate for crate in publish_crates.ALL_CRATES}

        def resolve_pkg_name(dep_key: str, spec_val, standalone: bool) -> str:
            if not isinstance(spec_val, dict):
                return dep_key
            if not standalone and spec_val.get("workspace") is True:
                ws_key = spec_val.get("package", dep_key)
                ws_spec = ws_deps.get(ws_key)
                if isinstance(ws_spec, dict) and "package" in ws_spec:
                    return ws_spec["package"]
                return ws_key
            return spec_val.get("package", dep_key)

        for crate in publish_crates.ALL_CRATES:
            manifest = tomllib.loads(crate.manifest.read_text(encoding="utf-8"))
            dep_tables = [
                manifest.get("dependencies", {}),
                manifest.get("build-dependencies", {}),
            ]
            for target_data in manifest.get("target", {}).values():
                dep_tables.append(target_data.get("dependencies", {}))
                dep_tables.append(target_data.get("build-dependencies", {}))

            for table in dep_tables:
                for dep_key, dep_spec in table.items():
                    pkg_name = resolve_pkg_name(dep_key, dep_spec, crate.standalone)
                    if pkg_name in by_name:
                        dep_layer = by_name[pkg_name].layer
                        self.assertLess(
                            dep_layer,
                            crate.layer,
                            f"{crate.name} (layer {crate.layer}) depends on {pkg_name} (layer {dep_layer})",
                        )

    def test_verify_tag_matches_workspace_version(self):
        ws_version = publish_crates.read_workspace_version()
        ok_v, _ = publish_crates.verify_tag(f"v{ws_version}")
        self.assertTrue(ok_v)
        ok_bare, _ = publish_crates.verify_tag(ws_version)
        self.assertTrue(ok_bare)
        bad, msg = publish_crates.verify_tag("v0.0.0-wrong")
        self.assertFalse(bad)
        self.assertIn(ws_version, msg)


if __name__ == "__main__":
    unittest.main()
