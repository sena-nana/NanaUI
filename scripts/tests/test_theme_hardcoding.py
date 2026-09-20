import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "theme_audit", Path(__file__).resolve().parents[1] / "audit-theme-hardcoding.py"
)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class ThemeHardcodingTests(unittest.TestCase):
    """The inventory has to classify, not just count.

    A number that goes up for the wrong reason is worse than no number: it
    trains people to re-record the baseline instead of reading it.
    """

    def scratch(self, source):
        root = Path(tempfile.mkdtemp())
        (root / "widget.rs").write_text(source, encoding="utf-8")
        self.addCleanup(shutil.rmtree, root, True)
        return root

    def test_comments_strings_and_test_modules_are_not_design_values(self):
        stripped = audit.strip_rust(
            'let a = "radius: 9.0"; // radius: 8.0\n'
            "/* radius: 7.0 */\n"
            "#[cfg(test)]\nmod tests { const R: f32 = 6.0; }\n"
            "const KEPT: f32 = 5.0;\n"
        )
        self.assertNotIn("9.0", stripped)
        self.assertNotIn("8.0", stripped)
        self.assertNotIn("7.0", stripped)
        self.assertNotIn("6.0", stripped)
        self.assertIn("5.0", stripped)

    def test_a_field_initializer_counts_and_a_local_binding_does_not(self):
        counted = audit.scan_body("    border_radius: Some(6.0),\n")
        self.assertEqual(counted["counts"]["design_number"], 1)
        # `let width = 0.0` is geometry maths. Counting it would bury the
        # design values under the layout engine.
        local = audit.scan_body("    let width = 0.0;\n")
        self.assertEqual(local["counts"]["design_number"], 0)

    def test_a_value_read_from_the_token_authority_is_the_denominator(self):
        scanned = audit.scan_body(
            "    min_height: Some(UI_METRICS.control_height),\n"
            "    padding_left: Some(space::MD),\n"
        )
        self.assertEqual(scanned["counts"]["design_number"], 0)
        self.assertEqual(scanned["token_reads"], 2)

    def test_naming_a_padding_or_height_step_is_a_token_read(self):
        scanned = audit.scan_body(
            "    control_padding_x: Some(ControlPadding::Field),\n"
            "    control_height: Some(ControlHeight::Min(ControlSize::Medium)),\n"
        )
        self.assertEqual(scanned["counts"]["design_number"], 0)
        self.assertGreaterEqual(scanned["token_reads"], 2)

    def test_roles_and_states_are_recorded_as_intent(self):
        scanned = audit.scan_body(
            "    effective_style.interaction.hovered.background = "
            "Some(SemanticColorRole::Hover);\n"
            "    effective_style.interaction.pressed.background = "
            "Some(SemanticColorRole::Active);\n"
        )
        self.assertEqual(scanned["counts"]["color_role"], 2)
        self.assertEqual(scanned["roles"], ["Active", "Hover"])
        self.assertEqual(scanned["states"], ["hovered", "pressed"])
        # Intent is not a local value: it moves into a recipe, it does not
        # disappear.
        self.assertEqual(scanned["counts"]["design_number"], 0)

    def test_component_bodies_and_shared_authority_are_scanned_separately(self):
        root = self.scratch(
            "impl ComponentView for Widget {\n"
            "    fn project(&self) {\n"
            "        let _ = SemanticColorRole::Accent;\n"
            "    }\n"
            "}\n"
            "fn shared_paint() {\n"
            "    let style = Style { corner_radius: 4.0 };\n"
            "}\n"
        )
        components = audit.scan_components(root)
        self.assertEqual([entry["component"] for entry in components], ["Widget"])
        self.assertEqual(components[0]["counts"]["color_role"], 1)
        self.assertEqual(components[0]["counts"]["design_number"], 0)

        shared = audit.scan_shared_authority(root)
        self.assertEqual([entry["file"] for entry in shared][0].endswith("widget.rs"), True)
        self.assertEqual(shared[0]["counts"]["design_number"], 1)
        self.assertEqual(shared[0]["counts"]["color_role"], 0)

    def test_a_constructor_is_part_of_the_component_not_shared_authority(self):
        """The defect this caught: `ListItem` declares every interaction state
        in `ListItem::new`, and a trait-body-only scan reported it as styling
        none of them."""
        root = self.scratch(
            """
impl ComponentView for Row {
    fn project(&self) {}
}
impl Row {
    fn new() -> Self {
        let interaction = InteractionStyle {
            hovered: SemanticPaint { background: Some(SemanticColorRole::Hover) },
        };
    }
}
"""
        )
        components = audit.scan_components(root)
        self.assertEqual([entry["component"] for entry in components], ["Row"])
        self.assertEqual(components[0]["states"], ["hovered"])
        self.assertEqual(components[0]["counts"]["color_role"], 1)
        # And it must not be counted a second time as shared authority.
        self.assertEqual(audit.scan_shared_authority(root), [])

    def test_a_trait_impl_for_a_non_component_stays_shared_authority(self):
        root = self.scratch(
            """
impl Default for Helper {
    fn default() -> Self {
        Self { corner_radius: 4.0 }
    }
}
"""
        )
        self.assertEqual(audit.scan_components(root), [])
        shared = audit.scan_shared_authority(root)
        self.assertEqual(shared[0]["counts"]["design_number"], 1)


    def test_the_committed_baseline_is_current(self):
        """`--check` is only useful if the recorded numbers are the real ones."""
        baseline_path = (
            audit.REPO_ROOT
            / "docs/performance-data/theme-audit-2026-09-19/theme-hardcoding.json"
        )
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
        report = audit.scan()
        for key in ("color_literal", "design_number", "motion"):
            self.assertLessEqual(
                report["totals"][key],
                baseline["totals"][key],
                f"components gained {key}; move it onto the Theme or re-record the baseline",
            )
            self.assertLessEqual(
                report["shared_totals"][key],
                baseline["shared_totals"][key],
                f"shared paint/geometry gained {key}",
            )
        self.assertLessEqual(
            report["module_constants"], baseline["module_constants"]
        )


if __name__ == "__main__":
    unittest.main()
