import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "generate_ucd_tables", ROOT / "scripts" / "generate-ucd-tables.py"
)
ucd = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(ucd)


def write_synthetic_ucd_dir(
    root: Path,
    *,
    version: str = "17.0.0",
    emoji_version: str | None = None,
    vo_missing: str = "U",
) -> Path:
    emoji_ver = emoji_version or ".".join(version.split(".")[:2])
    (root / "extracted").mkdir(parents=True, exist_ok=True)
    (root / "emoji").mkdir(parents=True, exist_ok=True)

    (root / "extracted" / "DerivedGeneralCategory.txt").write_text(
        "\n".join(
            [
                f"# DerivedGeneralCategory-{version}.txt",
                "0020          ; Zs # SPACE",
                "0021          ; Po # EXCLAMATION MARK",
                "0041..0043    ; Lu # LATIN CAPITAL LETTER A..C",
                "D7FF          ; Lo # BMP end before surrogates",
                "D800..DFFF    ; Cs # Surrogates",
                "E000          ; Co # Private use after surrogates",
                "FFFC          ; So # OBJECT REPLACEMENT CHARACTER",
                "FFFD          ; So # REPLACEMENT CHARACTER",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    (root / "DerivedCoreProperties.txt").write_text(
        "\n".join(
            [
                f"# DerivedCoreProperties-{version}.txt",
                "200C..200D    ; Default_Ignorable_Code_Point",
                "D7FF..E000    ; Default_Ignorable_Code_Point",
                "FE0F          ; Default_Ignorable_Code_Point",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    (root / "Scripts.txt").write_text(
        "\n".join(
            [
                f"# Scripts-{version}.txt",
                "# @missing: 0000..10FFFF; Unknown",
                "0020          ; Common",
                "0041..0042    ; Latin",
                "0043          ; Latin",
                "0300..0301    ; Inherited",
                "4E00..4E02    ; Han",
                "D7FF..E000    ; Han",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    (root / "PropertyValueAliases.txt").write_text(
        "\n".join(
            [
                f"# PropertyValueAliases-{version}.txt",
                "sc ; Hani ; Han",
                "sc ; Latn ; Latin",
                "sc ; Zinh ; Inherited",
                "sc ; Zyyy ; Common",
                "sc ; Zzzz ; Unknown",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    (root / "VerticalOrientation.txt").write_text(
        "\n".join(
            [
                f"# VerticalOrientation-{version}.txt",
                f"# @missing: 0000..10FFFF; {vo_missing}",
                "0041..0043    ; R",
                "4E00..4E02    ; U",
                "300C          ; Tr",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    (root / "emoji" / "emoji-data.txt").write_text(
        "\n".join(
            [
                "# emoji-data.txt",
                f"# Version: {emoji_ver}",
                "231A..231B    ; Emoji_Presentation",
                "1F600         ; Emoji_Presentation",
            ]
        )
        + "\n",
        encoding="utf-8",
    )
    return root


class UcdTablesTests(unittest.TestCase):
    def test_merge_and_surrogate_exclusion(self):
        ranges = ucd.merge_codepoints_to_ranges(
            [0xE001, 0x0041, 0xD800, 0xDFFF, 0xD7FF, 0x0043, 0x0042, 0xE000]
        )
        self.assertEqual(ranges, [(0x0041, 0x0043), (0xD7FF, 0xD7FF), (0xE000, 0xE001)])
        packed = ucd.pack_bool_ranges(ranges)
        self.assertEqual(packed[0], (0x0041 << 21) | 0x0043)

    def test_synthetic_tables_packing_and_dynamic_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            ucd_dir = write_synthetic_ucd_dir(Path(tmp), version="16.0.0")
            texts, _ = ucd.load_ucd_files(
                ucd_dir, Path(tmp) / "cache", allow_download=False, verify_sha=False
            )
            tables = ucd.build_tables(texts)

            self.assertEqual(tables["unicode_version"], (16, 0, 0))
            self.assertEqual(
                tables["default_ignorable"],
                [
                    (0x200C << 21) | 0x200D,
                    (0xD7FF << 21) | 0xD7FF,
                    (0xE000 << 21) | 0xE000,
                    (0xFE0F << 21) | 0xFE0F,
                ],
            )
            self.assertEqual(
                tables["vertical_rotated"],
                [(0x0041 << 21) | 0x0043],
            )
            # 0020 (Zs), 0021 (Po), D800..DFFF (Cs), FFFC (object placeholder) excluded.
            self.assertEqual(
                tables["emphasis_allowed"],
                [
                    (0x0041 << 21) | 0x0043,
                    (0xD7FF << 21) | 0xD7FF,
                    (0xE000 << 21) | 0xE000,
                    (0xFFFD << 21) | 0xFFFD,
                ],
            )
            self.assertEqual(
                tables["emoji_presentation"],
                [(0x231A << 21) | 0x231B, (0x1F600 << 21) | 0x1F600],
            )
            # Zyyy/Zinh/Zzzz excluded; Latn appears first, then Hani; adjacent 0041..0042 + 0043 merged;
            # D7FF..E000 split across surrogates.
            self.assertEqual(tables["script_tags"], ["Latn", "Hani"])
            self.assertEqual(
                tables["script_ranges"],
                [
                    (0x0041 << 32) | (0x0043 << 8) | 0,
                    (0x4E00 << 32) | (0x4E02 << 8) | 1,
                    (0xD7FF << 32) | (0xD7FF << 8) | 1,
                    (0xE000 << 32) | (0xE000 << 8) | 1,
                ],
            )

    def test_vertical_missing_default_r_excludes_non_r_and_surrogates(self):
        with tempfile.TemporaryDirectory() as tmp:
            ucd_dir = write_synthetic_ucd_dir(Path(tmp), vo_missing="R")
            texts, _ = ucd.load_ucd_files(
                ucd_dir, Path(tmp) / "cache", allow_download=False, verify_sha=False
            )
            ranges = ucd.parse_vertical_rotated(texts["VerticalOrientation.txt"])
            # 0x300C (Tr), 0x4E00..0x4E02 (U), and 0xD800..0xDFFF (surrogates) are excluded.
            self.assertEqual(
                ranges,
                [
                    (0x0000, 0x300B),
                    (0x300D, 0x4DFF),
                    (0x4E03, 0xD7FF),
                    (0xE000, 0x10FFFF),
                ],
            )

    def test_version_mismatch_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            ucd_dir = write_synthetic_ucd_dir(Path(tmp), version="17.0.0", emoji_version="16.0")
            texts, _ = ucd.load_ucd_files(
                ucd_dir, Path(tmp) / "cache", allow_download=False, verify_sha=False
            )
            with self.assertRaises(ValueError):
                ucd.build_tables(texts)

    def test_cli_generate_and_check_round_trip(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            ucd_dir = write_synthetic_ucd_dir(tmp_path / "ucd", version="16.0.0")
            out_rs = tmp_path / "ucd_tables.rs"
            empty_cache = tmp_path / "empty-cache"

            # Without --allow-unverified, synthetic files must fail SHA-256 verification.
            with self.assertRaises(SystemExit):
                ucd.main(
                    [
                        "--ucd-dir",
                        str(ucd_dir),
                        "--cache-dir",
                        str(empty_cache),
                        "--output",
                        str(out_rs),
                    ]
                )

            rc = ucd.main(
                [
                    "--ucd-dir",
                    str(ucd_dir),
                    "--cache-dir",
                    str(empty_cache),
                    "--output",
                    str(out_rs),
                    "--allow-unverified",
                ]
            )
            self.assertEqual(rc, 0)
            generated = out_rs.read_text(encoding="utf-8")
            self.assertIn("// @generated by scripts/generate-ucd-tables.py from Unicode 16.0.0 UCD", generated)
            self.assertIn("pub(super) const UNICODE_VERSION: (u8, u8, u8) = (16, 0, 0);", generated)

            # --check against --ucd-dir passes when unchanged.
            self.assertEqual(
                ucd.main(
                    [
                        "--check",
                        "--ucd-dir",
                        str(ucd_dir),
                        "--cache-dir",
                        str(empty_cache),
                        "--output",
                        str(out_rs),
                        "--allow-unverified",
                    ]
                ),
                0,
            )

            # Mutating the generated file causes --check to fail.
            out_rs.write_text(generated.replace("(16, 0, 0)", "(17, 0, 0)"), encoding="utf-8")
            self.assertEqual(
                ucd.main(
                    [
                        "--check",
                        "--ucd-dir",
                        str(ucd_dir),
                        "--cache-dir",
                        str(empty_cache),
                        "--output",
                        str(out_rs),
                        "--allow-unverified",
                    ]
                ),
                1,
            )

    def test_offline_check_rejects_tampered_provenance_or_invariants(self):
        committed = ucd.DEFAULT_OUTPUT.read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            copy = tmp_path / "ucd_tables.rs"
            empty_cache = tmp_path / "no-cache"

            # Valid copy passes offline check.
            copy.write_text(committed, encoding="utf-8")
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(empty_cache), "--output", str(copy)]),
                0,
            )

            # Missing @generated header fails.
            copy.write_text(committed.replace("@generated", "edited"), encoding="utf-8")
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(empty_cache), "--output", str(copy)]),
                1,
            )

            # Wrong UNICODE_VERSION fails.
            copy.write_text(committed.replace("(17, 0, 0)", "(16, 0, 0)"), encoding="utf-8")
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(empty_cache), "--output", str(copy)]),
                1,
            )

            # Tampered SHA provenance fails.
            first_sha = ucd.UCD_FILES[0][1]
            copy.write_text(committed.replace(first_sha, "0" * 64), encoding="utf-8")
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(empty_cache), "--output", str(copy)]),
                1,
            )

            # Direct invariant check catches surrogate leak.
            bad = committed.replace("0x00015A000AD,", f"0x{(0xD800 << 21) | 0xD800:011X},")
            self.assertTrue(any("surrogate" in err for err in ucd.verify_table_invariants(bad)))

    def test_committed_ucd_tables_provenance_and_version(self):
        self.assertEqual(ucd.verify_output_offline(ucd.DEFAULT_OUTPUT), [])
        text = ucd.DEFAULT_OUTPUT.read_text(encoding="utf-8")
        self.assertIn("// @generated by scripts/generate-ucd-tables.py from Unicode 17.0.0 UCD", text)
        self.assertIn("pub(super) const UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);", text)
        self.assertIn("// Regenerate: python3 scripts/generate-ucd-tables.py\n", text)
        self.assertIn("// Verify:     python3 scripts/generate-ucd-tables.py --check\n", text)

    def test_offline_check_requires_regeneration_command(self):
        committed = ucd.DEFAULT_OUTPUT.read_text(encoding="utf-8")
        stripped = committed.replace(f"// Regenerate: {ucd.REGENERATE_COMMAND}\n", "")
        self.assertNotEqual(stripped, committed)
        with tempfile.TemporaryDirectory() as tmp:
            copy = Path(tmp) / "ucd_tables.rs"
            copy.write_text(stripped, encoding="utf-8")
            self.assertTrue(
                any("Regenerate" in err for err in ucd.verify_output_offline(copy))
            )

    def test_unicode_crate_lockfile_alignment(self):
        self.assertEqual(ucd.verify_unicode_crate_alignment(ucd.DEFAULT_LOCKFILE), [])
        lock_text = ucd.DEFAULT_LOCKFILE.read_text(encoding="utf-8")
        with tempfile.TemporaryDirectory() as tmp:
            lock_copy = Path(tmp) / "Cargo.lock"
            # Exact lockfile passes --check --lockfile.
            lock_copy.write_text(lock_text, encoding="utf-8")
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(Path(tmp) / "no-cache"), "--lockfile", str(lock_copy)]),
                0,
            )
            # Bumping a companion Unicode crate without updating pins/docs fails --check.
            drifted = lock_text.replace(
                'name = "harfrust"\nversion = "0.12.0"',
                'name = "harfrust"\nversion = "0.13.0"',
            )
            lock_copy.write_text(drifted, encoding="utf-8")
            errs = ucd.verify_unicode_crate_alignment(lock_copy)
            self.assertTrue(
                any("harfrust" in err and "Unicode 17.0.0" in err and "third-party.md" in err for err in errs),
                errs,
            )
            self.assertEqual(
                ucd.main(["--check", "--cache-dir", str(Path(tmp) / "no-cache"), "--lockfile", str(lock_copy)]),
                1,
            )


if __name__ == "__main__":
    unittest.main()
