//! Issue #90 font layer, against committed fixtures only.
//!
//! Every face here comes from `crates/nana-text/fonts/` or the bundled Noto
//! Sans SC weights in `nana-ui-core`, and every fallback policy is built in
//! the test. Nothing reads the machine's installed fonts; that lives in
//! `font_system_platform_acceptance.rs`.

use nana_text::font::{
    FaceDescriptor, FallbackPolicy, FamilyList, FamilyName, FontChoiceReason, FontError, FontQuery,
    FontStretch, FontStyle, FontSystem, FontVariations, FontWeight, GenericFamily, LanguageTag,
    font_blob,
};
use nana_text::{FontId, ScriptTag, TextStyle};
use nana_ui_core::FontVariationSetting;
use nana_ui_core::fonts::{UI_FONT_BOLD, UI_FONT_MEDIUM, UI_FONT_REGULAR, UI_FONT_SEMIBOLD};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{Size, Tag};
use skrifa::{FontRef, MetadataProvider};
use std::path::PathBuf;
use std::sync::Arc;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fonts")
        .join(format!("{name}.ttf"))
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn register(system: &mut FontSystem, name: &str) -> FontId {
    let registration = system
        .register_bytes(font_blob(fixture(name)), &FaceDescriptor::default())
        .unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(registration.faces.len(), 1);
    registration.faces[0]
}

fn register_static(system: &mut FontSystem, bytes: &'static [u8]) -> FontId {
    system
        .register_bytes(font_blob(bytes), &FaceDescriptor::default())
        .expect("bundled face registers")
        .faces[0]
}

fn query(families: &str) -> FontQuery {
    FontQuery {
        families: FamilyList::parse_css(families),
        ..FontQuery::default()
    }
}

fn postscript(system: &FontSystem, font: Option<FontId>) -> String {
    let font = font.expect("a face was selected");
    system
        .describe(font)
        .expect("live face")
        .post_script_name
        .to_string()
}

// ---- matching -----------------------------------------------------------

#[test]
fn the_preferred_family_is_primary_and_later_families_form_the_chain() {
    let mut system = FontSystem::hermetic();
    let sc = register_static(&mut system, UI_FONT_REGULAR);
    let kr = register(&mut system, "noto-sans-kr");
    register(&mut system, "noto-sans-arabic");

    let selection = system.select(&query(
        r#""Noto Sans KR", "Does Not Exist", 'Noto Sans SC'"#,
    ));
    assert_eq!(selection.primary, Some(kr));
    assert_eq!(selection.fallback_chain, vec![sc]);
    let tried: Vec<(&str, bool)> = selection
        .families
        .iter()
        .map(|family| (family.name.as_ref(), family.face.is_some()))
        .collect();
    assert_eq!(
        tried,
        [
            ("Noto Sans KR", true),
            ("Does Not Exist", false),
            ("Noto Sans SC", true)
        ],
        "debug output names every family tried and whether it resolved"
    );
}

#[test]
fn weight_matching_picks_the_css_nearest_static_face() {
    let mut system = FontSystem::hermetic();
    // Registered out of weight order on purpose.
    for bytes in [
        UI_FONT_BOLD,
        UI_FONT_REGULAR,
        UI_FONT_SEMIBOLD,
        UI_FONT_MEDIUM,
    ] {
        register_static(&mut system, bytes);
    }
    let pick = |system: &mut FontSystem, weight: f32| {
        let mut query = query("Noto Sans SC");
        query.weight = FontWeight::new(weight);
        let selection = system.select(&query);
        postscript(system, selection.primary)
    };
    assert_eq!(pick(&mut system, 400.0), "NotoSansSC-Regular");
    assert_eq!(pick(&mut system, 500.0), "NotoSansSC-Medium");
    assert_eq!(pick(&mut system, 600.0), "NotoSansSC-SemiBold");
    assert_eq!(pick(&mut system, 700.0), "NotoSansSC-Bold");
    // 650 is above 500, so heavier wins over the closer lighter face.
    assert_eq!(pick(&mut system, 650.0), "NotoSansSC-Bold");
    // 450 checks heavier faces up to 500 before any lighter one.
    assert_eq!(pick(&mut system, 450.0), "NotoSansSC-Medium");
    // Below everything: the lightest available.
    assert_eq!(pick(&mut system, 100.0), "NotoSansSC-Regular");
    assert_eq!(pick(&mut system, 1000.0), "NotoSansSC-Bold");
}

#[test]
fn style_and_stretch_descriptors_narrow_before_weight() {
    let mut system = FontSystem::hermetic();
    let bytes = font_blob(fixture("noto-sans-kr"));
    let face = |system: &mut FontSystem, descriptor: FaceDescriptor| {
        system
            .register_bytes(Arc::clone(&bytes), &descriptor)
            .unwrap()
            .faces[0]
    };
    let upright = face(&mut system, FaceDescriptor::family("Styled"));
    let italic_bold = face(
        &mut system,
        FaceDescriptor {
            style: Some(FontStyle::Italic),
            weight: Some((700.0, 700.0)),
            ..FaceDescriptor::family("Styled")
        },
    );
    let condensed = face(
        &mut system,
        FaceDescriptor {
            stretch: Some((75.0, 75.0)),
            ..FaceDescriptor::family("Styled")
        },
    );

    let mut italic = query("Styled");
    italic.style = FontStyle::Italic;
    italic.weight = FontWeight(300.0);
    assert_eq!(
        system.select(&italic).primary,
        Some(italic_bold),
        "style is narrowed before weight"
    );

    let mut oblique = query("Styled");
    oblique.style = FontStyle::Oblique;
    assert_eq!(system.select(&oblique).primary, Some(italic_bold));

    let mut narrow = query("Styled");
    narrow.stretch = FontStretch(80.0);
    assert_eq!(system.select(&narrow).primary, Some(condensed));

    let mut wide = query("Styled");
    wide.stretch = FontStretch(120.0);
    assert_eq!(
        system.select(&wide).primary,
        Some(upright),
        "above 100% prefers the nearest wider face, then narrower"
    );
}

#[test]
fn a_variable_weight_axis_matches_as_a_range() {
    let mut system = FontSystem::hermetic();
    let axes = register(&mut system, "nana-test-axes");
    let described = system.describe(axes).unwrap();
    assert_eq!(described.weight, (100.0, 900.0));
    assert_eq!(described.stretch, (50.0, 200.0));
    let instance_names: Vec<Option<&str>> = described
        .named_instances
        .iter()
        .map(|instance| instance.name.as_deref())
        .collect();
    assert_eq!(instance_names, [Some("Regular"), Some("Bold")]);

    let mut heavy = query("NanaTestAxes");
    heavy.weight = FontWeight(850.0);
    heavy.style = FontStyle::Oblique;
    let selection = system.select(&heavy);
    assert_eq!(selection.primary, Some(axes));
    let instance = system
        .instance(axes, &heavy, &FontVariations::default())
        .unwrap();
    assert_eq!(instance.coord(*b"wght"), Some(850.0));
    assert_eq!(instance.coord(*b"slnt"), Some(-14.0), "oblique drives slnt");
    assert!(!instance.key.synthesis.bold);
    assert!(!instance.key.synthesis.oblique);
}

#[test]
fn selection_is_deterministic_across_systems_and_later_registrations_win_ties() {
    let build = || {
        let mut system = FontSystem::hermetic();
        let first = register_static(&mut system, UI_FONT_REGULAR);
        let second = register_static(&mut system, UI_FONT_REGULAR);
        register(&mut system, "noto-sans-kr");
        (system, first, second)
    };
    let (mut a, _, a_second) = build();
    let (mut b, _, b_second) = build();
    let query = query("Noto Sans SC, Noto Sans KR");
    let from_a = a.select(&query);
    let from_b = b.select(&query);
    assert_eq!(
        from_a.primary,
        Some(a_second),
        "the later duplicate shadows"
    );
    assert_eq!(from_a.primary, from_b.primary);
    assert_eq!(from_a.fallback_chain, from_b.fallback_chain);
    assert_eq!(a_second, b_second);
}

#[test]
fn the_same_query_is_answered_from_the_cache_until_the_generation_moves() {
    let mut system = FontSystem::hermetic();
    register_static(&mut system, UI_FONT_REGULAR);
    let query = query("Noto Sans SC");
    let first = system.select(&query);
    let second = system.select(&query);
    assert!(
        Arc::ptr_eq(&first, &second),
        "a hit reuses the same selection"
    );
    let counters = system.counters();
    assert_eq!(
        (counters.font_query_hits, counters.font_query_misses),
        (1, 1)
    );

    register(&mut system, "noto-sans-kr");
    let third = system.select(&query);
    assert!(!Arc::ptr_eq(&first, &third));
    assert_eq!(system.counters().font_query_misses, 2);
    assert!(!system.is_current(&first));
    assert!(system.is_current(&third));
}

#[test]
fn generic_families_resolve_through_the_policy() {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Missing UI", "Noto Sans KR"]);
    let mut system = FontSystem::with_policy(policy);
    let kr = register(&mut system, "noto-sans-kr");
    let selection = system.select(&query("Nope, system-ui"));
    assert_eq!(selection.primary, Some(kr));
    assert_eq!(
        selection.families.last().map(|family| &family.requested),
        Some(&FamilyName::Generic(GenericFamily::SystemUi))
    );
}

// ---- fallback -----------------------------------------------------------

fn cjk_policy() -> FallbackPolicy {
    let mut policy = FallbackPolicy::empty();
    policy
        .push_script_rule(ScriptTag(*b"Hani"), Some("ja"), ["Test JP"])
        .push_script_rule(ScriptTag(*b"Hani"), None, ["Noto Sans SC"])
        .push_script_rule(ScriptTag(*b"Hang"), None, ["Noto Sans KR"])
        .set_emoji_families(["NanaTestColor", "Noto Emoji"]);
    policy
}

#[test]
fn mixed_latin_and_cjk_follow_the_family_chain_before_script_policy() {
    let mut system = FontSystem::with_policy(cjk_policy());
    let vf = register(&mut system, "nana-test-vf");
    let sc = register_static(&mut system, UI_FONT_REGULAR);
    let selection = system.select(&query("NanaTestVF, Noto Sans SC"));
    let assignments = system.resolve_text(&selection, "A世界", None);
    let summary: Vec<(std::ops::Range<usize>, Option<FontId>, FontChoiceReason)> = assignments
        .iter()
        .map(|assignment| {
            (
                assignment.range.clone(),
                assignment.font,
                assignment.reason.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (0..1, Some(vf), FontChoiceReason::Primary),
            (1..7, Some(sc), FontChoiceReason::FamilyChain { index: 0 }),
        ]
    );
}

#[test]
fn cjk_fallback_uses_the_script_policy_and_honours_the_language_hint() {
    let mut system = FontSystem::with_policy(cjk_policy());
    let vf = register(&mut system, "nana-test-vf");
    let sc = register_static(&mut system, UI_FONT_REGULAR);
    let jp = system
        .register_bytes(
            font_blob(UI_FONT_REGULAR),
            &FaceDescriptor::family("Test JP"),
        )
        .unwrap()
        .faces[0];
    let kr = register(&mut system, "noto-sans-kr");

    let selection = system.select(&query("NanaTestVF"));
    assert_eq!(selection.primary, Some(vf));

    let chinese = system.resolve_text(&selection, "A中。", None);
    assert_eq!(chinese.len(), 2);
    assert_eq!(chinese[1].font, Some(sc));
    assert_eq!(
        chinese[1].range,
        1..7,
        "the ideographic full stop inherits Han"
    );
    assert_eq!(
        chinese[1].reason,
        FontChoiceReason::ScriptPolicy {
            script: ScriptTag(*b"Hani"),
            family: Arc::from("Noto Sans SC"),
        }
    );

    let japanese = system.resolve_text(&selection, "中", LanguageTag::new("ja-JP").as_ref());
    assert_eq!(
        japanese[0].font,
        Some(jp),
        "a ja hint prefers the Japanese face"
    );

    let korean = system.resolve_text(&selection, "한글", None);
    assert_eq!(korean[0].font, Some(kr));
}

#[test]
fn emoji_presentation_prefers_a_colour_face_and_falls_back_to_monochrome() {
    let mut system = FontSystem::with_policy(cjk_policy());
    register(&mut system, "nana-test-vf");
    let mono = register(&mut system, "noto-emoji");
    let color = register(&mut system, "nana-test-color");
    assert!(system.describe(color).unwrap().color_glyphs);
    assert!(!system.describe(mono).unwrap().color_glyphs);

    // The monochrome face is in the family chain and covers 🔥 too, but emoji
    // presentation asks for colour first.
    let selection = system.select(&query("NanaTestVF, Noto Emoji"));
    let fire = system.resolve_text(&selection, "🔥", None);
    assert_eq!(fire[0].font, Some(color));
    assert!(fire[0].emoji);
    assert_eq!(
        fire[0].reason,
        FontChoiceReason::EmojiPolicy {
            family: Arc::from("NanaTestColor")
        }
    );

    // The colour face lacks 👩 and 💻, so the ZWJ sequence stays one cluster on
    // the monochrome face; the joiner itself needs no coverage.
    let coder = system.resolve_text(&selection, "👩\u{200D}💻", None);
    assert_eq!(coder.len(), 1);
    assert_eq!(coder[0].font, Some(mono));
    assert_eq!(coder[0].range, 0..11);
}

#[test]
fn a_glyph_nothing_covers_is_reported_missing_and_counted() {
    let mut system = FontSystem::with_policy(cjk_policy());
    let vf = register(&mut system, "nana-test-vf");
    register_static(&mut system, UI_FONT_REGULAR);
    let selection = system.select(&query("NanaTestVF"));
    system.reset_counters();

    let assignments = system.resolve_text(&selection, "Aก", None);
    assert_eq!(assignments.len(), 2);
    assert_eq!(assignments[0].font, Some(vf));
    assert_eq!(assignments[1].font, None);
    assert_eq!(assignments[1].reason, FontChoiceReason::Missing);

    let counters = system.counters();
    assert_eq!(counters.font_fallback_attempts, 1);
    assert_eq!(counters.font_fallback_misses, 1);
    // Thai has no script rule and the cluster has its own script, so the
    // policy phase could only have probed the chain beyond the primary:
    // nothing. What it did probe is the database scan that runs last, over
    // both registered faces, and that is what makes "missing" mean "this
    // machine does not have it" rather than "no list happened to name it".
    assert_eq!(counters.fallback_candidates_examined, 2);
}

#[test]
fn coverage_is_cached_per_face_within_a_byte_budget() {
    let mut system = FontSystem::with_policy(cjk_policy());
    register(&mut system, "nana-test-vf");
    register_static(&mut system, UI_FONT_REGULAR);
    let selection = system.select(&query("NanaTestVF"));
    system.reset_counters();

    system.resolve_text(&selection, "中文", None);
    let first = system.counters();
    assert_eq!(first.coverage_cache_misses, 2, "primary and the Han face");
    system.resolve_text(&selection, "中文字", None);
    let second = system.counters();
    assert_eq!(second.coverage_cache_misses, 2, "no cmap is walked twice");
    assert!(second.coverage_cache_hits > first.coverage_cache_hits);
    assert!(system.coverage_cache_bytes() > 0);
    assert!(system.coverage_cache_bytes() <= system.coverage_budget_bytes());

    system.set_coverage_budget(0);
    assert_eq!(system.coverage_cache_len(), 0);
    assert_eq!(system.counters().coverage_cache_evictions, 2);
}

// ---- variable fonts / #41 -------------------------------------------------

#[derive(Default)]
struct Bounds {
    x_min: f32,
    x_max: f32,
    started: bool,
}

impl Bounds {
    fn add(&mut self, x: f32) {
        if self.started {
            self.x_min = self.x_min.min(x);
            self.x_max = self.x_max.max(x);
        } else {
            (self.x_min, self.x_max, self.started) = (x, x, true);
        }
    }
}

impl OutlinePen for Bounds {
    fn move_to(&mut self, x: f32, _: f32) {
        self.add(x);
    }
    fn line_to(&mut self, x: f32, _: f32) {
        self.add(x);
    }
    fn quad_to(&mut self, cx: f32, _: f32, x: f32, _: f32) {
        self.add(cx);
        self.add(x);
    }
    fn curve_to(&mut self, cx0: f32, _: f32, cx1: f32, _: f32, x: f32, _: f32) {
        self.add(cx0);
        self.add(cx1);
        self.add(x);
    }
    fn close(&mut self) {}
}

/// Horizontal extent of `ch` at the instance's coordinates, read back through
/// an independent outline reader so the coordinates are proven to reach the
/// face rather than merely to be recorded.
fn glyph_extent(
    system: &FontSystem,
    instance: &nana_text::font::FontInstance,
    ch: char,
) -> (f32, f32) {
    let data = system.face_data(instance.font()).unwrap();
    let font = FontRef::from_index(data.bytes(), data.index()).unwrap();
    let location = font.axes().location(
        instance
            .coords()
            .iter()
            .map(|coord| (Tag::new(&coord.tag), coord.value)),
    );
    let glyph = font.charmap().map(ch).unwrap();
    let outline = font.outline_glyphs().get(glyph).unwrap();
    let mut bounds = Bounds::default();
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), &location),
            &mut bounds,
        )
        .unwrap();
    (bounds.x_min, bounds.x_max)
}

fn style_with(variations: &[FontVariationSetting], weight: u16) -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from("NanaTestVF")),
        font_weight: weight,
        variations: variations.to_vec(),
        ..TextStyle::default()
    }
}

#[test]
fn custom_bevl_and_wdth_axes_reach_the_outline_and_never_become_wght() {
    let mut system = FontSystem::hermetic();
    let vf = register(&mut system, "nana-test-vf");

    let resolve = |system: &FontSystem, settings: &[FontVariationSetting]| {
        let style = style_with(settings, 400);
        let query = FontQuery::from_style(&style, None);
        system
            .instance(
                vf,
                &query,
                &FontVariations::from_settings(&style.variations),
            )
            .unwrap()
    };

    let plain = resolve(&system, &[]);
    assert!(plain.coords().is_empty());

    let bevl = resolve(&system, &[FontVariationSetting::new(*b"BEVL", 42.0)]);
    assert_eq!(bevl.coord(*b"BEVL"), Some(42.0));
    assert_eq!(bevl.coord(*b"wght"), None, "BEVL is not a weight");
    assert!(bevl.ignored_axes.is_empty());
    assert_ne!(
        bevl.key, plain.key,
        "the axis value is part of the cache key"
    );
    assert_ne!(
        glyph_extent(&system, &bevl, 'A'),
        glyph_extent(&system, &plain, 'A')
    );

    let wide = resolve(&system, &[FontVariationSetting::new(*b"wdth", 150.0)]);
    assert_eq!(wide.coord(*b"wdth"), Some(150.0));
    assert_ne!(
        glyph_extent(&system, &wide, 'A'),
        glyph_extent(&system, &plain, 'A')
    );

    // This face has no wght axis: an explicit wght is reported and dropped,
    // and an unknown tag likewise, instead of either being redirected.
    let unknown = resolve(
        &system,
        &[
            FontVariationSetting::new(*b"wght", 800.0),
            FontVariationSetting::new(*b"XXXX", 3.0),
        ],
    );
    assert!(unknown.coords().is_empty());
    assert_eq!(unknown.ignored_axes, vec![*b"XXXX", *b"wght"]);
    // The declared weight is still a weight request: the face cannot reach it,
    // so it is honestly reported as needing synthetic bold.
    assert!(unknown.key.synthesis.bold);
}

#[test]
fn an_explicit_wght_outranks_font_weight_for_selection_and_coordinates() {
    let mut system = FontSystem::hermetic();
    let axes = register(&mut system, "nana-test-axes");
    let style = TextStyle {
        font_family: Some(Arc::from("NanaTestAxes")),
        font_weight: 700,
        variations: vec![FontVariationSetting::new(*b"wght", 300.0)],
        ..TextStyle::default()
    };
    let query = FontQuery::from_style(&style, None);
    assert_eq!(query.weight, FontWeight(300.0));
    assert_eq!(system.select(&query).primary, Some(axes));
    let variations = FontVariations::from_settings(&style.variations);
    let instance = system.instance(axes, &query, &variations).unwrap();
    assert_eq!(instance.coord(*b"wght"), Some(300.0));

    // font-weight alone drives the axis when no wght is declared, and the
    // outline follows it.
    let bold_query = FontQuery::from_style(
        &TextStyle {
            variations: Vec::new(),
            ..style
        },
        None,
    );
    let bold = system
        .instance(axes, &bold_query, &FontVariations::default())
        .unwrap();
    assert_eq!(bold.coord(*b"wght"), Some(700.0));
    let (_, light_max) = glyph_extent(&system, &instance, 'A');
    let (_, bold_max) = glyph_extent(&system, &bold, 'A');
    assert!(bold_max > light_max, "{bold_max} > {light_max}");
}

#[test]
fn a_static_face_asked_for_bold_or_italic_reports_synthesis() {
    let mut system = FontSystem::hermetic();
    let kr = register(&mut system, "noto-sans-kr");
    let mut query = query("Noto Sans KR");
    query.weight = FontWeight::BOLD;
    query.style = FontStyle::Italic;
    let instance = system
        .instance(kr, &query, &FontVariations::default())
        .unwrap();
    assert!(instance.key.synthesis.bold);
    assert!(instance.key.synthesis.oblique);
}

// ---- lifetime and generations -------------------------------------------

#[test]
fn the_font_system_and_face_data_can_cross_threads() {
    fn send<T: Send>() {}
    fn send_sync<T: Send + Sync>() {}
    send::<FontSystem>();
    send_sync::<nana_text::font::FontData>();
    send_sync::<nana_text::font::FontSelection>();
    send_sync::<nana_text::font::FontInstance>();

    let mut system = FontSystem::hermetic();
    let kr = register(&mut system, "noto-sans-kr");
    let data = system.face_data(kr).unwrap();
    let worker =
        std::thread::spawn(move || FontRef::from_index(data.bytes(), data.index()).is_ok());
    assert!(worker.join().unwrap());
}

#[test]
fn unregistering_retires_handles_and_selections_but_not_bytes_in_use() {
    let mut system = FontSystem::hermetic();
    let sc = register_static(&mut system, UI_FONT_REGULAR);
    let kr_registration = system
        .register_bytes(
            font_blob(fixture("noto-sans-kr")),
            &FaceDescriptor::default(),
        )
        .unwrap();
    let kr = kr_registration.faces[0];
    let query = query("Noto Sans KR, Noto Sans SC");
    let before = system.select(&query);
    assert_eq!(before.primary, Some(kr));
    assert!(system.covers(sc, '中'));
    assert!(system.covers(kr, '한'));
    let held = system.face_data(kr).unwrap();
    let generation = system.generation();

    system.unregister(kr_registration.source).unwrap();
    assert_eq!(
        system.generation(),
        generation.bumped(),
        "one mutation, one bump"
    );
    assert!(!system.contains(kr));
    assert!(system.describe(kr).is_none());
    assert!(
        system
            .instance(kr, &query, &FontVariations::default())
            .is_none()
    );
    assert!(!system.is_current(&before));
    assert_eq!(system.select(&query).primary, Some(sc));
    assert_eq!(
        system.unregister(kr_registration.source),
        Err(FontError::UnknownSource),
        "a retired source cannot be unregistered twice"
    );

    // The worker's bytes survive the unregister, and their release is observable.
    assert!(FontRef::from_index(held.bytes(), held.index()).is_ok());
    assert_eq!(system.retired_font_data_alive(), 1);
    drop(held);
    assert_eq!(system.retired_font_data_alive(), 0);

    // The surviving face's coverage was not thrown away by the generation bump.
    system.reset_counters();
    assert!(system.covers(sc, '中'));
    assert_eq!(system.counters().coverage_cache_hits, 1);

    // The freed slot is reissued at a higher generation, never as the old id.
    let reissued = register(&mut system, "noto-sans-kr");
    assert_eq!(reissued.index(), kr.index());
    assert_ne!(reissued, kr);
    assert_eq!(system.counters().coverage_cache_misses, 0);
    assert!(system.covers(reissued, '한'));
    assert_eq!(system.counters().coverage_cache_misses, 1);
}

#[test]
fn replacing_a_source_is_one_generation_step_and_keeps_the_old_face_on_error() {
    let mut system = FontSystem::hermetic();
    let registration = system
        .register_bytes(
            font_blob(fixture("noto-sans-kr")),
            &FaceDescriptor::family("Brand"),
        )
        .unwrap();
    let old = registration.faces[0];
    let generation = system.generation();

    assert_eq!(
        system.replace_bytes(
            registration.source,
            font_blob(b"not a font".to_vec()),
            &FaceDescriptor::family("Brand")
        ),
        Err(FontError::Unrecognized)
    );
    assert!(system.contains(old));
    assert_eq!(system.generation(), generation);

    let replaced = system
        .replace_bytes(
            registration.source,
            font_blob(fixture("noto-sans-arabic")),
            &FaceDescriptor::family("Brand"),
        )
        .unwrap();
    assert_eq!(replaced.generation, generation.bumped());
    assert!(!system.contains(old));
    let new = replaced.faces[0];
    assert_ne!(new, old);
    let selection = system.select(&query("Brand"));
    assert_eq!(selection.primary, Some(new));
    assert!(system.covers(new, 'ع'));
    assert!(!system.covers(new, '한'));
}

#[test]
fn file_memory_and_malformed_registrations() {
    let mut system = FontSystem::hermetic();
    let from_file = system
        .register_file(fixture_path("noto-emoji"), &FaceDescriptor::default())
        .unwrap();
    let described = system.describe(from_file.faces[0]).unwrap();
    assert_eq!(described.origin, nana_text::font::FontOrigin::File);
    assert_eq!(described.families, vec![Arc::<str>::from("Noto Emoji")]);
    assert!(described.path.is_some());

    assert_eq!(
        system.register_bytes(font_blob(Vec::new()), &FaceDescriptor::default()),
        Err(FontError::Empty)
    );
    assert_eq!(
        system.register_bytes(font_blob(vec![0u8; 64]), &FaceDescriptor::default()),
        Err(FontError::Unrecognized)
    );
    assert!(matches!(
        system.register_file(fixture_path("does-not-exist"), &FaceDescriptor::default()),
        Err(FontError::Io(_))
    ));
    assert_eq!(system.counters().font_faces_registered, 1);
    assert_eq!(system.counters().font_generation, system.generation().get());
}

/// A NaN weight cannot come from `FontWeight::new`, but it can come from a
/// deserialized query or a hand-built struct — and a type that hashes by
/// canonical bits while comparing by raw float would key the selection cache
/// with an entry it can never find again.
#[test]
fn a_non_finite_weight_or_stretch_still_equals_itself() {
    use std::collections::HashMap;
    use std::hash::{BuildHasher, RandomState};

    for (weight, stretch) in [
        (FontWeight(f32::NAN), FontStretch(f32::NAN)),
        (FontWeight(400.0), FontStretch(100.0)),
    ] {
        assert_eq!(weight, weight, "{weight:?} must equal itself");
        assert_eq!(stretch, stretch, "{stretch:?} must equal itself");
        let hasher = RandomState::new();
        assert_eq!(hasher.hash_one(weight), hasher.hash_one(weight));

        let mut map: HashMap<FontWeight, u32> = HashMap::new();
        map.insert(weight, 1);
        map.insert(weight, 2);
        assert_eq!(map.len(), 1, "{weight:?} keyed two entries");
        assert_eq!(map.get(&weight), Some(&2));
    }
    assert_eq!(
        FontWeight::new(f32::NAN),
        FontWeight::NORMAL,
        "the constructor still rejects it outright"
    );
}

/// A codepoint no policy family names, in a face the database happens to hold.
///
/// The policy is a *preference* order, not the set of faces that exist. A
/// check mark, a box-drawing rune or a dingbat lives in a face no generic,
/// script or symbol list mentions, and rendering `.notdef` for it while the
/// database has the glyph is the worst of both answers.
#[test]
fn a_codepoint_no_policy_family_names_is_still_found_in_the_database() {
    let mut system = FontSystem::with_policy(FallbackPolicy::empty());
    // The only face the policy names cannot cover the text, and it is not the
    // one that can: there is no path to the second face except a scan.
    let vf = register(&mut system, "nana-test-vf");
    let sc = register_static(&mut system, UI_FONT_REGULAR);
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["NanaTestVF"]);
    system.set_policy(policy);

    let selection = system.select(&query("NanaTestVF"));
    assert_eq!(selection.primary, Some(vf));
    let resolved = system.resolve_text(&selection, "中", None);
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].font,
        Some(sc),
        "the scan finds the only face that covers it: {:?}",
        resolved[0].reason
    );
    assert!(
        matches!(resolved[0].reason, FontChoiceReason::LastResort { .. }),
        "and says it got there last: {:?}",
        resolved[0].reason
    );

    // Nothing covers a private-use codepoint, and saying so is still the
    // answer — the scan must not invent a face.
    let missing = system.resolve_text(&selection, "\u{f8ff}", None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].font, None);
    assert_eq!(missing[0].reason, FontChoiceReason::Missing);
}
