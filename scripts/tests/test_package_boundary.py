import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("package_boundary", Path(__file__).resolve().parents[1] / "check-package-boundary.py")
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)


class PackageBoundaryTests(unittest.TestCase):
    def graph(self, edges, members=("nana-package", "nana-packager"), features=None, kinds=None):
        names = sorted({*members, *edges, *(child for children in edges.values() for child in children)})
        kind = lambda parent, child: (kinds or {}).get((parent, child))
        return {
            "workspace_members": list(members),
            "packages": [{"id": name, "name": name} for name in names],
            "resolve": {
                "nodes": [
                    {
                        "id": name,
                        "features": (features or {}).get(name, []),
                        "deps": [
                            {"pkg": child, "dep_kinds": [{"kind": kind(name, child)}]}
                            for child in edges.get(name, [])
                        ],
                    }
                    for name in names
                ]
            },
        }

    def test_clean_graph_passes(self):
        data = self.graph(
            {"nana-package": ["blake3", "chacha20poly1305"], "nana-packager": ["nana-package", "zstd"]},
            features={"blake3": ["std", "pure"]},
        )
        self.assertEqual(boundary.check(data), [])

    def test_c_or_rng_crate_in_the_reader_is_rejected(self):
        data = self.graph({"nana-package": ["helper"], "helper": ["zstd-sys"]})
        self.assertTrue(any("nana-package -> helper -> zstd-sys" in f for f in boundary.check(data)))
        data = self.graph({"nana-package": ["getrandom"]})
        self.assertTrue(any("getrandom" in f for f in boundary.check(data)))

    def test_blake3_without_pure_is_rejected(self):
        data = self.graph({"nana-package": ["blake3"]}, features={"blake3": ["std"]})
        self.assertTrue(any("pure" in f for f in boundary.check(data)))

    def test_embeddable_runtime_must_not_reach_packaging(self):
        data = self.graph(
            {"nana-ui-runtime": ["nana-package"]},
            members=("nana-package", "nana-packager", "nana-ui-runtime"),
        )
        self.assertTrue(any("nana-ui-runtime -> nana-package" in f for f in boundary.check(data)))

    def test_packager_only_as_a_dev_dependency(self):
        members = ("nana-package", "nana-packager", "app")
        data = self.graph({"app": ["nana-packager"]}, members=members)
        self.assertTrue(any("app -> nana-packager" in f for f in boundary.check(data)))
        data = self.graph({"app": ["nana-packager"]}, members=members, kinds={("app", "nana-packager"): "dev"})
        self.assertEqual(boundary.check(data), [])


if __name__ == "__main__":
    unittest.main()
