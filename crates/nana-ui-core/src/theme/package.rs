//! Pure-data versioned `ThemePackage` schema, validation, and inheritance (#109).
//!
//! A [`ThemePackage`] is a purely declarative serialization envelope over
//! [`ThemePatch`]: it carries no scripts, no shaders, no filesystem commands,
//! and no runtime CSS parser. Parsing, schema checking, inheritance resolution,
//! and bounds validation all finish before producing a [`CompiledTheme`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    CompiledTheme, ThemeAppearance, ThemeCompileError, ThemeDefinition, ThemeGeneration, ThemeId,
    ThemePatch, ThemeRegistry, ThemeSchemaVersion,
};

/// Resource limits rejecting malformed or unbounded theme packages fail-closed.
pub const MAX_PACKAGE_BYTES: usize = 256 * 1024;
pub const MAX_COLOR_OVERRIDES: usize = 128;
pub const MAX_INHERITANCE_DEPTH: usize = 8;
pub const MAX_METADATA_STR_LEN: usize = 256;
pub const MAX_FAMILY_PATCHES: usize = 32;

/// Pure-data versioned theme package authoring format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemePackage {
    pub schema_version: ThemeSchemaVersion,
    pub id: ThemeId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<ThemeId>,
    #[serde(default)]
    pub patch: ThemePatch,
}

/// Fail-closed validation/compilation errors for [`ThemePackage`].
#[derive(Debug, Clone, PartialEq)]
pub enum ThemePackageError {
    SizeLimitExceeded {
        bytes: usize,
        max: usize,
    },
    Parse {
        message: String,
        line: usize,
        column: usize,
    },
    UnsupportedSchema {
        theme: ThemeId,
        found: ThemeSchemaVersion,
        supported: ThemeSchemaVersion,
    },
    InvalidId {
        id: String,
        reason: &'static str,
    },
    MetadataTooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    EntryLimitExceeded {
        field: &'static str,
        count: usize,
        max: usize,
    },
    MissingBase {
        theme: ThemeId,
        base: ThemeId,
    },
    CircularBase {
        cycle: Vec<ThemeId>,
    },
    InheritanceDepthExceeded {
        theme: ThemeId,
        depth: usize,
        max: usize,
    },
    DuplicateId {
        id: ThemeId,
    },
    Compile(ThemeCompileError),
}

impl From<ThemeCompileError> for ThemePackageError {
    fn from(error: ThemeCompileError) -> Self {
        Self::Compile(error)
    }
}

impl std::fmt::Display for ThemePackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SizeLimitExceeded { bytes, max } => {
                write!(f, "theme package size {bytes} bytes exceeds limit {max}")
            }
            Self::Parse {
                message,
                line,
                column,
            } => write!(f, "theme package parse error at {line}:{column}: {message}"),
            Self::UnsupportedSchema {
                theme,
                found,
                supported,
            } => write!(
                f,
                "theme package `{theme}` uses schema {found}, supported is {supported}"
            ),
            Self::InvalidId { id, reason } => {
                write!(f, "invalid theme package id `{id}`: {reason}")
            }
            Self::MetadataTooLong { field, len, max } => {
                write!(
                    f,
                    "theme package field `{field}` length {len} exceeds {max}"
                )
            }
            Self::EntryLimitExceeded { field, count, max } => {
                write!(
                    f,
                    "theme package `{field}` entry count {count} exceeds {max}"
                )
            }
            Self::MissingBase { theme, base } => {
                write!(
                    f,
                    "theme package `{theme}` references missing base `{base}`"
                )
            }
            Self::CircularBase { cycle } => {
                let chain = cycle
                    .iter()
                    .map(ThemeId::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "circular theme base inheritance: {chain}")
            }
            Self::InheritanceDepthExceeded { theme, depth, max } => write!(
                f,
                "theme package `{theme}` inheritance depth {depth} exceeds limit {max}"
            ),
            Self::DuplicateId { id } => write!(f, "duplicate theme package id `{id}`"),
            Self::Compile(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ThemePackageError {}

/// Source provenance captured during package compilation for DevTools diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemePackageProvenance {
    /// Inheritance chain from root base down to the package itself.
    pub base_chain: Vec<ThemeId>,
    /// Ordered overrides contributed by each package layer in the chain.
    pub patch_entries: Vec<(ThemeId, Vec<(String, String)>)>,
}

/// Output of compiling a [`ThemePackage`].
#[derive(Debug, Clone)]
pub struct CompiledThemePackage {
    pub definition: ThemeDefinition,
    pub compiled: Arc<CompiledTheme>,
    pub provenance: ThemePackageProvenance,
}

/// Result of explicit pre-load schema migration (`migrate_theme_package_json`).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemePackageMigration {
    pub original_schema: ThemeSchemaVersion,
    pub migrated: ThemePackage,
    pub notes: Vec<&'static str>,
}

impl ThemePackage {
    pub fn new(id: ThemeId, name: impl Into<String>) -> Self {
        Self {
            schema_version: ThemeSchemaVersion::CURRENT,
            id,
            name: name.into(),
            description: None,
            author: None,
            base: None,
            patch: ThemePatch::new(),
        }
    }

    #[must_use]
    pub fn with_base(mut self, base: ThemeId) -> Self {
        self.base = Some(base);
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    #[must_use]
    pub fn with_patch(mut self, patch: ThemePatch) -> Self {
        self.patch = patch;
        self
    }

    /// Parse and validate a JSON theme package fail-closed.
    ///
    /// Unknown fields, unsupported schema versions, oversized payloads, and
    /// out-of-range metadata/entry counts are rejected here before any base
    /// resolution or compilation runs.
    pub fn from_json(json: &str) -> Result<Self, ThemePackageError> {
        if json.len() > MAX_PACKAGE_BYTES {
            return Err(ThemePackageError::SizeLimitExceeded {
                bytes: json.len(),
                max: MAX_PACKAGE_BYTES,
            });
        }
        let pkg: Self = serde_json::from_str(json).map_err(|err| ThemePackageError::Parse {
            message: err.to_string(),
            line: err.line(),
            column: err.column(),
        })?;
        pkg.validate_envelope()?;
        Ok(pkg)
    }

    /// Deterministic JSON serialization.
    pub fn to_json_pretty(&self) -> Result<String, ThemePackageError> {
        serde_json::to_string_pretty(self).map_err(|err| ThemePackageError::Parse {
            message: err.to_string(),
            line: 0,
            column: 0,
        })
    }

    pub fn validate_envelope(&self) -> Result<(), ThemePackageError> {
        if !self.schema_version.is_supported() {
            return Err(ThemePackageError::UnsupportedSchema {
                theme: self.id.clone(),
                found: self.schema_version,
                supported: ThemeSchemaVersion::CURRENT,
            });
        }
        let id_str = self.id.as_str();
        if id_str.trim().is_empty() {
            return Err(ThemePackageError::InvalidId {
                id: id_str.to_owned(),
                reason: "theme id must not be empty",
            });
        }
        if id_str.len() > MAX_METADATA_STR_LEN {
            return Err(ThemePackageError::MetadataTooLong {
                field: "id",
                len: id_str.len(),
                max: MAX_METADATA_STR_LEN,
            });
        }
        if !id_str
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
        {
            return Err(ThemePackageError::InvalidId {
                id: id_str.to_owned(),
                reason: "allowed characters are ASCII alphanumeric and `.:_-`",
            });
        }
        if self.name.trim().is_empty() {
            return Err(ThemePackageError::InvalidId {
                id: id_str.to_owned(),
                reason: "theme name must not be empty",
            });
        }
        for (field, value) in [
            ("name", self.name.as_str()),
            ("description", self.description.as_deref().unwrap_or("")),
            ("author", self.author.as_deref().unwrap_or("")),
        ] {
            if value.len() > MAX_METADATA_STR_LEN {
                return Err(ThemePackageError::MetadataTooLong {
                    field,
                    len: value.len(),
                    max: MAX_METADATA_STR_LEN,
                });
            }
        }
        if let Some(colors) = &self.patch.colors {
            let count = colors.iter().count();
            if count > MAX_COLOR_OVERRIDES {
                return Err(ThemePackageError::EntryLimitExceeded {
                    field: "patch.colors",
                    count,
                    max: MAX_COLOR_OVERRIDES,
                });
            }
        }
        if let Some(recipes) = &self.patch.recipes {
            if recipes.families.len() > MAX_FAMILY_PATCHES {
                return Err(ThemePackageError::EntryLimitExceeded {
                    field: "patch.recipes.families",
                    count: recipes.families.len(),
                    max: MAX_FAMILY_PATCHES,
                });
            }
        }
        Ok(())
    }

    pub fn compile(
        &self,
        registry: &ThemeRegistry,
    ) -> Result<CompiledThemePackage, ThemePackageError> {
        let mut results = compile_theme_packages(std::slice::from_ref(self), registry)?;
        Ok(results.remove(0))
    }
}

/// Explicit schema migration entry point.
///
/// [`ThemePackage::from_json`] always fails closed on unknown or legacy schema
/// versions; callers that want to migrate pre-1.0 drafts (`0.x`) onto
/// [`ThemeSchemaVersion::CURRENT`] call this explicit entry instead of having
/// the runtime guess.
pub fn migrate_theme_package_json(json: &str) -> Result<ThemePackageMigration, ThemePackageError> {
    if json.len() > MAX_PACKAGE_BYTES {
        return Err(ThemePackageError::SizeLimitExceeded {
            bytes: json.len(),
            max: MAX_PACKAGE_BYTES,
        });
    }
    let mut value: serde_json::Value =
        serde_json::from_str(json).map_err(|err| ThemePackageError::Parse {
            message: err.to_string(),
            line: err.line(),
            column: err.column(),
        })?;
    let Some(obj) = value.as_object_mut() else {
        return Err(ThemePackageError::Parse {
            message: "expected top-level JSON object".into(),
            line: 1,
            column: 1,
        });
    };
    let schema_val =
        obj.get("schema_version")
            .cloned()
            .ok_or_else(|| ThemePackageError::Parse {
                message: "missing `schema_version`".into(),
                line: 1,
                column: 1,
            })?;
    let schema: ThemeSchemaVersion =
        serde_json::from_value(schema_val).map_err(|err| ThemePackageError::Parse {
            message: err.to_string(),
            line: 1,
            column: 1,
        })?;
    let id_preview = obj
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    if schema.major > ThemeSchemaVersion::CURRENT.major
        || (schema.major == 1 && !schema.is_supported())
    {
        return Err(ThemePackageError::UnsupportedSchema {
            theme: ThemeId::from_owned(id_preview),
            found: schema,
            supported: ThemeSchemaVersion::CURRENT,
        });
    }
    let mut notes = Vec::new();
    if schema.major == 0 {
        obj.insert(
            "schema_version".into(),
            serde_json::to_value(ThemeSchemaVersion::CURRENT).expect("schema serializes"),
        );
        notes.push("upgraded pre-1.0 draft schema to 1.0");
    }
    let pkg: ThemePackage =
        serde_json::from_value(value).map_err(|err| ThemePackageError::Parse {
            message: err.to_string(),
            line: 1,
            column: 1,
        })?;
    pkg.validate_envelope()?;
    Ok(ThemePackageMigration {
        original_schema: schema,
        migrated: pkg,
        notes,
    })
}

/// Validate and compile a batch of packages together, resolving cross-package
/// `base` chains, detecting cycles, and capturing provenance.
pub fn compile_theme_packages(
    packages: &[ThemePackage],
    registry: &ThemeRegistry,
) -> Result<Vec<CompiledThemePackage>, ThemePackageError> {
    let mut by_id: BTreeMap<ThemeId, &ThemePackage> = BTreeMap::new();
    for pkg in packages {
        pkg.validate_envelope()?;
        if by_id.insert(pkg.id.clone(), pkg).is_some() {
            return Err(ThemePackageError::DuplicateId { id: pkg.id.clone() });
        }
    }

    let mut memo: BTreeMap<ThemeId, CompiledThemePackage> = BTreeMap::new();
    let mut out = Vec::with_capacity(packages.len());
    for pkg in packages {
        let mut visiting = Vec::new();
        let compiled = resolve_package(&pkg.id, &by_id, registry, &mut visiting, &mut memo)?;
        out.push(compiled);
    }
    Ok(out)
}

fn resolve_package(
    id: &ThemeId,
    by_id: &BTreeMap<ThemeId, &ThemePackage>,
    registry: &ThemeRegistry,
    visiting: &mut Vec<ThemeId>,
    memo: &mut BTreeMap<ThemeId, CompiledThemePackage>,
) -> Result<CompiledThemePackage, ThemePackageError> {
    if let Some(existing) = memo.get(id) {
        return Ok(existing.clone());
    }
    if let Some(pos) = visiting.iter().position(|seen| seen == id) {
        let mut cycle = visiting[pos..].to_vec();
        cycle.push(id.clone());
        return Err(ThemePackageError::CircularBase { cycle });
    }
    if visiting.len() >= MAX_INHERITANCE_DEPTH {
        return Err(ThemePackageError::InheritanceDepthExceeded {
            theme: id.clone(),
            depth: visiting.len() + 1,
            max: MAX_INHERITANCE_DEPTH,
        });
    }
    let pkg = by_id
        .get(id)
        .copied()
        .ok_or_else(|| ThemePackageError::MissingBase {
            theme: visiting.last().cloned().unwrap_or_else(|| id.clone()),
            base: id.clone(),
        })?;

    visiting.push(id.clone());
    let (base_def, mut provenance) =
        match &pkg.base {
            None => {
                let mode = pkg.patch.appearance.unwrap_or(ThemeAppearance::Dark);
                let mut def = ThemeDefinition::for_appearance(mode);
                def.generation = ThemeGeneration::FIRST;
                (def, ThemePackageProvenance::default())
            }
            Some(base_id) if by_id.contains_key(base_id) => {
                let parent = resolve_package(base_id, by_id, registry, visiting, memo)?;
                (parent.definition, parent.provenance)
            }
            Some(base_id) => {
                let def =
                    registry
                        .definition(base_id)
                        .ok_or_else(|| ThemePackageError::MissingBase {
                            theme: pkg.id.clone(),
                            base: base_id.clone(),
                        })?;
                let prov = registry.provenance(base_id).cloned().unwrap_or_else(|| {
                    ThemePackageProvenance {
                        base_chain: vec![base_id.clone()],
                        patch_entries: Vec::new(),
                    }
                });
                (def, prov)
            }
        };
    visiting.pop();

    let mut definition = pkg.patch.apply_to_definition(&base_def);
    definition.id = pkg.id.clone();
    definition.display_name = pkg.name.clone().into();
    definition.schema = pkg.schema_version;
    definition.generation = ThemeGeneration::FIRST;

    provenance.base_chain.push(pkg.id.clone());
    let described = pkg.patch.describe_overrides();
    if !described.is_empty() {
        provenance.patch_entries.push((pkg.id.clone(), described));
    }

    let compiled = Arc::new(definition.compile()?);
    let result = CompiledThemePackage {
        definition,
        compiled,
        provenance,
    };
    memo.insert(id.clone(), result.clone());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantics::ButtonKind;
    use crate::style_model::{SemanticColor, SemanticColorRole};
    use crate::theme::controls::{
        InputRecipe, MenuRecipe, SelectRecipe, TabsRecipe, TreeViewRecipe,
    };
    use crate::theme::dependency::{ThemeCategoryGenerations, ThemeDependencyMask};
    use crate::theme::patch::{MotionPatch, ThemeMetricsPatch, TypographyPatch};
    use crate::theme::recipe::ButtonVariantRecipe;
    use crate::theme::tokens::{AccentRamp, MotionTokens, TextWeight};

    #[test]
    fn theme_dependency_diff_classifies_categories_precisely() {
        let dark = ThemeDefinition::NANA_DARK.compile().unwrap();
        let light = ThemeDefinition::NANA_LIGHT.compile().unwrap();
        let diff_ld = ThemeCategoryGenerations::diff_themes(&dark, &light);
        assert!(diff_ld.contains(ThemeDependencyMask::PALETTE));
        assert!(diff_ld.contains(ThemeDependencyMask::ACCENT));
        assert!(diff_ld.contains(ThemeDependencyMask::EFFECT));
        assert!(!diff_ld.intersects(ThemeDependencyMask::METRICS));
        assert!(!diff_ld.intersects(ThemeDependencyMask::TYPOGRAPHY_ANY));
        assert!(!diff_ld.intersects(ThemeDependencyMask::MOTION_POLICY));
        assert!(!diff_ld.intersects(ThemeDependencyMask::RECIPE_ANY));

        // Accent-only change touches ACCENT, not PALETTE / METRICS / TYPOGRAPHY / MOTION.
        let coral = AccentRamp {
            base: SemanticColor::rgb8(240, 120, 100),
            strong: SemanticColor::rgb8(210, 90, 75),
            ..AccentRamp::DARK
        };
        let accent_only = ThemeDefinition::NANA_DARK
            .with_accent(coral)
            .compile()
            .unwrap();
        let diff_acc = ThemeCategoryGenerations::diff_themes(&dark, &accent_only);
        assert_eq!(diff_acc, ThemeDependencyMask::ACCENT);

        // Motion-only change touches only MOTION_POLICY.
        let mut motion_def = ThemeDefinition::NANA_DARK;
        motion_def.motion = MotionTokens {
            hover_color_ms: 200,
            ..MotionTokens::DEFAULT
        };
        let motion_only = motion_def.compile().unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &motion_only),
            ThemeDependencyMask::MOTION_POLICY
        );

        // Weight-only change -> TYPOGRAPHY_SHAPE; size-only -> TYPOGRAPHY_LAYOUT.
        let mut weight_def = ThemeDefinition::NANA_DARK;
        weight_def.typography.medium = 550;
        let weight_only = weight_def.compile().unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &weight_only),
            ThemeDependencyMask::TYPOGRAPHY_SHAPE
        );

        let mut size_def = ThemeDefinition::NANA_DARK;
        size_def.typography.body = 15.0;
        let size_only = size_def.compile().unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &size_only),
            ThemeDependencyMask::TYPOGRAPHY_LAYOUT
        );

        // Input / Select / Tabs / TreeView / Menu recipe deltas isolate to their recipe mask.
        let input_only = ThemeDefinition::NANA_DARK
            .with_input(InputRecipe {
                invalid_focus_border_width: 3.0,
                ..InputRecipe::DEFAULT
            })
            .compile()
            .unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &input_only),
            ThemeDependencyMask::RECIPE_INPUT
        );
        let select_only = ThemeDefinition::NANA_DARK
            .with_select(SelectRecipe {
                checkmark_width: 20.0,
                ..SelectRecipe::DEFAULT
            })
            .compile()
            .unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &select_only),
            ThemeDependencyMask::RECIPE_SELECT
        );
        let tabs_only = ThemeDefinition::NANA_DARK
            .with_tabs(TabsRecipe {
                font_weight: TextWeight::Bold,
                ..TabsRecipe::DEFAULT
            })
            .compile()
            .unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &tabs_only),
            ThemeDependencyMask::RECIPE_TABS
        );
        let tree_only = ThemeDefinition::NANA_DARK
            .with_tree_view(TreeViewRecipe {
                selected_weight: 700,
                ..TreeViewRecipe::DEFAULT
            })
            .compile()
            .unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &tree_only),
            ThemeDependencyMask::RECIPE_TREE_VIEW
        );
        let menu_only = ThemeDefinition::NANA_DARK
            .with_menu(MenuRecipe {
                mark_width: 18.0,
                ..MenuRecipe::DEFAULT
            })
            .compile()
            .unwrap();
        assert_eq!(
            ThemeCategoryGenerations::diff_themes(&dark, &menu_only),
            ThemeDependencyMask::RECIPE_MENU
        );
    }

    #[test]
    fn package_minimal_patch_over_nana_dark_and_round_trip() {
        let pkg = ThemePackage::new(ThemeId::new("acme.dark.minimal"), "Acme Minimal")
            .with_base(ThemeDefinition::NANA_DARK.id)
            .with_description("Minimal accent patch")
            .with_author("NanaUI")
            .with_patch(ThemePatch::new().with_color(
                SemanticColorRole::Accent,
                SemanticColor::rgb8(100, 200, 180),
            ));
        let json = pkg.to_json_pretty().unwrap();
        let parsed = ThemePackage::from_json(&json).unwrap();
        assert_eq!(parsed, pkg);
        assert_eq!(parsed.to_json_pretty().unwrap(), json);

        let mut registry = ThemeRegistry::default();
        let compiled = registry.register_package_json(&json).unwrap();
        assert_eq!(compiled.id().as_str(), "acme.dark.minimal");
        assert_eq!(
            compiled.palette().accent,
            SemanticColor::rgb8(100, 200, 180)
        );
        assert_eq!(
            compiled.palette().background,
            ThemeDefinition::NANA_DARK.tokens.palette.background
        );
        let prov = registry.provenance(&compiled.id()).unwrap();
        assert_eq!(prov.base_chain.len(), 2);
        assert_eq!(prov.base_chain[0], ThemeDefinition::NANA_DARK.id);
    }

    #[test]
    fn package_accent_radius_density_button_and_typography_patches() {
        let mut registry = ThemeRegistry::default();
        let pkg = ThemePackage::new(ThemeId::new("acme.custom"), "Acme Custom")
            .with_base(ThemeDefinition::NANA_DARK.id)
            .with_patch(
                ThemePatch::new()
                    .with_accent(AccentRamp {
                        base: SemanticColor::rgb8(220, 100, 80),
                        ..AccentRamp::DARK
                    })
                    .with_metrics_patch(ThemeMetricsPatch {
                        radius_sm: Some(10.0),
                        control_height: Some(26.0),
                        ..ThemeMetricsPatch::default()
                    })
                    .with_typography_patch(TypographyPatch {
                        body: Some(15.0),
                        medium: Some(550),
                        ..TypographyPatch::default()
                    })
                    .with_motion_patch(MotionPatch {
                        hover_color_ms: Some(90),
                        ..MotionPatch::default()
                    })
                    .with_button_variant(
                        ButtonKind::Primary,
                        ButtonVariantRecipe {
                            foreground: SemanticColorRole::Text,
                            background: Some(SemanticColorRole::Highlight),
                            border: Some(SemanticColorRole::BorderStrong),
                            hovered_background: SemanticColorRole::HighlightHover,
                            pressed_background: SemanticColorRole::HighlightPressed,
                        },
                    ),
            );
        let json = pkg.to_json_pretty().unwrap();
        let compiled = registry.register_package_json(&json).unwrap();
        assert_eq!(compiled.palette().accent, SemanticColor::rgb8(220, 100, 80));
        assert_eq!(compiled.metrics().radius_sm, 10.0);
        assert_eq!(compiled.metrics().control_height, 26.0);
        assert_eq!(compiled.typography().body, 15.0);
        assert_eq!(compiled.typography().medium, 550);
        assert_eq!(compiled.motion().hover_color_ms, 90);
        assert_eq!(
            compiled
                .recipes()
                .button()
                .variant(ButtonKind::Primary)
                .background,
            Some(SemanticColorRole::Highlight)
        );
        // Unpatched variants retain base NanaDark values.
        assert_eq!(
            compiled.recipes().button().variant(ButtonKind::Ghost),
            ThemeDefinition::NANA_DARK
                .compile()
                .unwrap()
                .recipes()
                .button()
                .variant(ButtonKind::Ghost)
        );
    }

    #[test]
    fn package_rejects_invalid_schema_unknown_fields_cycles_and_bad_values() {
        let registry = ThemeRegistry::default();

        // Unknown schema fails closed on from_json.
        let future_schema = r#"{
            "schema_version": { "major": 2, "minor": 0 },
            "id": "acme.future",
            "name": "Future"
        }"#;
        assert!(matches!(
            ThemePackage::from_json(future_schema),
            Err(ThemePackageError::UnsupportedSchema { .. })
        ));

        // Pre-1.0 draft fails closed on from_json, succeeds through explicit migration.
        let draft_schema = r#"{
            "schema_version": { "major": 0, "minor": 9 },
            "id": "acme.draft",
            "name": "Draft",
            "base": "nana.dark"
        }"#;
        assert!(matches!(
            ThemePackage::from_json(draft_schema),
            Err(ThemePackageError::UnsupportedSchema { .. })
        ));
        let migrated = migrate_theme_package_json(draft_schema).unwrap();
        assert_eq!(
            migrated.migrated.schema_version,
            ThemeSchemaVersion::CURRENT
        );
        assert!(migrated.migrated.compile(&registry).is_ok());

        // Unknown field fails closed.
        let unknown_field = r#"{
            "schema_version": { "major": 1, "minor": 0 },
            "id": "acme.bad",
            "name": "Bad",
            "script": "alert(1)"
        }"#;
        assert!(matches!(
            ThemePackage::from_json(unknown_field),
            Err(ThemePackageError::Parse { .. })
        ));

        // Negative length fails fail-closed compilation.
        let neg_radius = ThemePackage::new(ThemeId::new("acme.neg"), "Negative")
            .with_base(ThemeDefinition::NANA_DARK.id)
            .with_patch(ThemePatch::new().with_metrics_patch(ThemeMetricsPatch {
                radius_md: Some(-4.0),
                ..ThemeMetricsPatch::default()
            }));
        assert!(matches!(
            neg_radius.compile(&registry),
            Err(ThemePackageError::Compile(
                ThemeCompileError::NegativeLength {
                    token: "metrics.radius_md",
                    ..
                }
            ))
        ));

        // Missing base fails cleanly.
        let missing_base = ThemePackage::new(ThemeId::new("acme.orphan"), "Orphan")
            .with_base(ThemeId::new("nonexistent.base"));
        assert!(matches!(
            missing_base.compile(&registry),
            Err(ThemePackageError::MissingBase { .. })
        ));

        // Circular base A -> B -> A detected cleanly.
        let pkg_a =
            ThemePackage::new(ThemeId::new("cycle.a"), "A").with_base(ThemeId::new("cycle.b"));
        let pkg_b =
            ThemePackage::new(ThemeId::new("cycle.b"), "B").with_base(ThemeId::new("cycle.a"));
        assert!(matches!(
            compile_theme_packages(&[pkg_a, pkg_b], &registry),
            Err(ThemePackageError::CircularBase { .. })
        ));

        // Size limit enforced before JSON parsing.
        let huge = " ".repeat(MAX_PACKAGE_BYTES + 1);
        assert!(matches!(
            ThemePackage::from_json(&huge),
            Err(ThemePackageError::SizeLimitExceeded { .. })
        ));
    }

    #[test]
    fn theme_patch_compose_and_apply_to_compiled_validates() {
        let base = ThemeDefinition::NANA_DARK.compile().unwrap();
        let outer = ThemePatch::new().with_metrics_patch(ThemeMetricsPatch {
            radius_sm: Some(2.0),
            control_height: Some(28.0),
            ..ThemeMetricsPatch::default()
        });
        let inner = ThemePatch::new()
            .with_metrics_patch(ThemeMetricsPatch {
                radius_sm: Some(8.0),
                ..ThemeMetricsPatch::default()
            })
            .with_color(SemanticColorRole::Text, SemanticColor::rgb8(255, 255, 255));
        let merged = inner.over(&outer);
        let patched = merged.apply_to_compiled(&base).unwrap();
        assert_eq!(patched.metrics().radius_sm, 8.0);
        assert_eq!(patched.metrics().control_height, 28.0);
        assert_eq!(patched.palette().text, SemanticColor::rgb8(255, 255, 255));

        let invalid = ThemePatch::new().with_typography_patch(TypographyPatch {
            body: Some(0.0),
            ..TypographyPatch::default()
        });
        assert!(matches!(
            invalid.apply_to_compiled(&base),
            Err(ThemeCompileError::FontSizeOutOfRange {
                token: "typography.body",
                ..
            })
        ));
    }
}
