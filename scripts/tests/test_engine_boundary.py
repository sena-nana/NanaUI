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
    def test_a_dev_only_edge_to_a_reference_engine_is_allowed(self):
        data = self.graph({"nana-text": ["cosmic-text"]}, root="nana-text")
        next(node for node in data["resolve"]["nodes"] if node["id"] == "nana-text")["deps"][0]["dep_kinds"][0]["kind"] = "dev"
        self.assertEqual(boundary.check_dependency_graph(data), [])
    def test_no_product_crate_may_reach_a_replaced_text_engine(self):
        # Issue #99 §11: the rule is the whole workspace's, not nana-text's.
        for engine in ("cosmic-text", "cryoglyph", "glyphon"):
            failures = boundary.check_dependency_graph(self.graph({"nana-ui": ["helper"], "helper": [engine]}, root="nana-ui"))
            self.assertTrue(any(f"nana-ui -> helper -> {engine}" in failure for failure in failures), failures)
    def test_a_renamed_fork_of_a_replaced_engine_is_still_rejected(self):
        failures = boundary.check_dependency_graph(self.graph({"nana-ui": ["nana-cryoglyph"]}, root="nana-ui"))
        self.assertTrue(any("nana-ui -> nana-cryoglyph" in failure for failure in failures), failures)
    def test_a_build_edge_is_a_product_edge(self):
        data = self.graph({"nana-ui": ["cosmic-text"]}, root="nana-ui")
        next(node for node in data["resolve"]["nodes"] if node["id"] == "nana-ui")["deps"][0]["dep_kinds"][0]["kind"] = "build"
        self.assertTrue(boundary.check_dependency_graph(data))
    def test_a_reference_only_crate_may_keep_a_replaced_engine(self):
        # A dev/test comparison tool is the sanctioned home for one; product
        # crates reaching that tool are caught by the reference-only rule.
        data = self.graph({"nana-css-parity": ["cosmic-text"]}, root="nana-css-parity")
        self.assertEqual(boundary.check_dependency_graph(data), [])
        data = self.graph({"nana-ui": ["nana-css-parity"], "nana-css-parity": ["cosmic-text"]}, root="nana-ui")
        self.assertTrue(any("nana-ui -> nana-css-parity -> cosmic-text" in failure for failure in boundary.check_dependency_graph(data)))
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
    def text_crate_files(self, files):
        root = Path(tempfile.mkdtemp())
        for relative, source in files.items():
            path = root / "src" / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source, encoding="utf-8")
        self.addCleanup(shutil.rmtree, root, True)
        return root
    def gpu_failures(self, files):
        return boundary.check_gpu_contract_sources(self.text_crate_files(files))
    def test_gpu_contract_rejects_wgpu_in_public_signatures(self):
        failures = self.gpu_failures({
            "lib.rs": (
                "pub fn device(&self) -> &wgpu::Device { todo!() }\n"
                "pub struct Context { pub queue: wgpu::Queue, raw: wgpu::Queue }\n"
                "pub use wgpu;\n"
                "pub type Format = wgpu::TextureFormat;\n"
                "pub struct Wrap(pub wgpu::Texture);\n"
            ),
        })
        self.assertEqual(len(failures), 5, failures)
    def test_gpu_contract_sees_enum_payloads_and_trait_methods(self):
        failures = self.gpu_failures({
            "lib.rs": (
                "pub enum Frame { Ready(wgpu::SurfaceTexture), Retry }\n"
                "pub trait Renderer { fn draw(&self, pass: &mut wgpu::RenderPass<'_>); }\n"
            ),
        })
        self.assertEqual(len(failures), 2, failures)
    def test_gpu_contract_allows_crate_private_and_interop_gated_items(self):
        failures = self.gpu_failures({
            "lib.rs": (
                "pub(crate) fn device() -> &'static wgpu::Device { todo!() }\n"
                "pub struct Format(pub(crate) wgpu::TextureFormat);\n"
                "pub const RGBA: Format = Format(wgpu::TextureFormat::Rgba8Unorm);\n"
                "#[cfg(feature = \"wgpu-interop\")]\n"
                "pub fn raw(&self) -> &wgpu::Device { todo!() }\n"
                "#[cfg(feature = \"wgpu-interop\")]\n"
                "impl Format { pub fn wgpu(self) -> wgpu::TextureFormat { self.0 } }\n"
                "#[cfg(feature = \"wgpu-interop\")]\n"
                "mod wgpu_interop;\n"
                "#[cfg(test)]\n"
                "mod tests { pub fn fixture() -> wgpu::Device { todo!() } }\n"
                "// pub fn prose(device: &wgpu::Device) mentions the backend in a comment\n"
            ),
            "wgpu_interop.rs": "pub fn device() -> &'static wgpu::Device { todo!() }\n",
            "bin/bench.rs": "pub fn device() -> wgpu::Device { todo!() }\n",
        })
        self.assertEqual(failures, [])
    def test_an_ungated_interop_module_is_rejected(self):
        failures = self.gpu_failures({"lib.rs": "mod wgpu_interop;\n"})
        self.assertTrue(any("declares mod wgpu_interop" in failure for failure in failures), failures)
    def test_only_framework_sources_reach_the_backend_through_framework(self):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root, True)
        (root / "src").mkdir()
        (root / "examples").mkdir()
        (root / "src" / "lib.rs").write_text("use nana_gpu::__framework;\n", encoding="utf-8")
        (root / "examples" / "demo.rs").write_text("use nana_gpu::__framework;\n", encoding="utf-8")
        manifest = str(root / "Cargo.toml")
        framework = boundary.check_gpu_framework_users({"name": "nana-ui", "manifest_path": manifest})
        self.assertEqual(len(framework), 1, framework)
        self.assertIn("examples", framework[0])
        consumer = boundary.check_gpu_framework_users({"name": "an-app", "manifest_path": manifest})
        self.assertEqual(len(consumer), 2, consumer)
        owner = boundary.check_gpu_framework_users({"name": "nana-gpu", "manifest_path": manifest})
        self.assertEqual(owner, [])
    def test_font_backends_are_named_only_from_their_private_module(self):
        root = self.text_crate_files({
            "lib.rs": "pub mod font;\n",
            "font/mod.rs": "mod discovery;\nmod face;\n",
            "font/discovery.rs": "fn scan() { let _ = fontdb::Database::new(); }\n",
            "font/face.rs": "use skrifa::FontRef;\n",
        })
        self.assertEqual(boundary.check_text_engine_sources(root), [])
    def test_a_font_backend_type_outside_its_module_is_rejected(self):
        root = self.text_crate_files({
            "lib.rs": "pub fn id() -> fontdb::ID { todo!() }\n",
            "font/face.rs": "pub fn raw() -> read_fonts::FontRef<'static> { todo!() }\n",
        })
        failures = boundary.check_text_engine_sources(root)
        self.assertTrue(any("lib.rs names fontdb" in failure for failure in failures))
        self.assertTrue(any("face.rs names read_fonts" in failure for failure in failures))
    def test_making_a_backend_module_public_is_rejected(self):
        root = self.text_crate_files({
            "font/mod.rs": "pub mod discovery;\n",
            "font/discovery.rs": "pub use fontdb::ID;\n",
        })
        failures = boundary.check_text_engine_sources(root)
        self.assertTrue(any("makes discovery public" in failure for failure in failures))
    def test_the_shaper_backends_are_pinned_to_their_own_modules(self):
        root = self.text_crate_files({
            "shaping/mod.rs": "mod opentype;\nmod bidi;\n",
            "shaping/opentype.rs": "use harfrust::UnicodeBuffer;\n",
            "shaping/bidi.rs": "use unicode_bidi::BidiInfo;\n",
            "shaping/shaper.rs": "fn level() -> unicode_bidi::Level { todo!() }\n",
        })
        failures = boundary.check_text_engine_sources(root)
        self.assertEqual(len(failures), 1, failures)
        self.assertIn("shaper.rs names unicode_bidi", failures[0])
    def test_the_line_breaker_is_pinned_to_its_own_module(self):
        root = self.text_crate_files({
            "layout/mod.rs": "mod breaks;\nmod lines;\n",
            "layout/breaks.rs": "use unicode_linebreak::linebreaks;\n",
            "layout/lines.rs": "fn at() -> unicode_linebreak::BreakClass { todo!() }\n",
        })
        failures = boundary.check_text_engine_sources(root)
        self.assertEqual(len(failures), 1, failures)
        self.assertIn("lines.rs names unicode_linebreak", failures[0])
    def test_making_the_line_breaker_module_public_is_rejected(self):
        root = self.text_crate_files({
            "layout/mod.rs": "pub mod breaks;\n",
            "layout/breaks.rs": "pub use unicode_linebreak::BreakOpportunity;\n",
        })
        failures = boundary.check_text_engine_sources(root)
        self.assertTrue(any("makes breaks public" in failure for failure in failures))
    def test_nana_text_may_name_the_text_align_keyword(self):
        root = self.text_crate("use nana_ui_core::TextAlignSpec;\npub struct A(TextAlignSpec);\n")
        self.assertEqual(boundary.check_text_engine_sources(root), [])
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
