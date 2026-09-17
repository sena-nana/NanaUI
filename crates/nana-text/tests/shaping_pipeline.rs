//! Issue #91 shaping pipeline, against committed fixtures only.

use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, font_blob};
use nana_text::shaping::{ShapeCacheBudget, ShapeRequest, ShapedText, Shaper};
use nana_text::{
    CompositionSegment, FontId, GlyphFlags, RunDirection, ScriptTag, TextConstraints, TextScale,
    TextSource, TextSpan, TextStyle,
};
use nana_ui_core::fonts::UI_FONT_REGULAR;
use nana_ui_core::{DirSpec, FontFeatureSetting, FontVariationSetting};
use std::path::PathBuf;
use std::sync::Arc;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fonts")
        .join(format!("{name}.ttf"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Registers faces in order; the returned ids line up with `names`.
fn system(names: &[&str]) -> (FontSystem, Vec<FontId>) {
    let mut fonts = FontSystem::with_policy(FallbackPolicy::empty());
    let ids = names
        .iter()
        .map(|name| {
            let bytes = if *name == "noto-sans-sc" {
                font_blob(UI_FONT_REGULAR)
            } else {
                font_blob(fixture(name))
            };
            fonts
                .register_bytes(bytes, &FaceDescriptor::default())
                .unwrap()
                .faces[0]
        })
        .collect();
    (fonts, ids)
}

fn style(families: &str) -> TextStyle {
    TextStyle {
        font_family: Some(Arc::from(families)),
        font_size_px: 20.0,
        ..TextStyle::default()
    }
}

fn shape(
    shaper: &mut Shaper,
    fonts: &mut FontSystem,
    source: &TextSource,
    style: &TextStyle,
    direction: DirSpec,
) -> Arc<ShapedText> {
    let constraints = TextConstraints {
        base_direction: direction,
        ..TextConstraints::default()
    };
    shaper.shape(fonts, &ShapeRequest::new(source, style, &constraints))
}

/// Every glyph's cluster lies inside its run, clusters are byte offsets on
/// char boundaries, and the runs tile the text apart from paragraph breaks.
fn assert_source_mapping(text: &str, shaped: &ShapedText) {
    let mut covered = vec![false; text.len()];
    for run in &shaped.runs {
        assert!(
            run.source.start < run.source.end,
            "empty run {:?}",
            run.source
        );
        for byte in run.source.clone() {
            assert!(!covered[byte], "byte {byte} is in two runs");
            covered[byte] = true;
        }
        for glyph in &run.glyphs {
            let (start, end) = (glyph.cluster as usize, glyph.cluster_end as usize);
            assert!(run.source.start <= start && start < end && end <= run.source.end);
            assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
        }
        let sum: f32 = run.glyphs.iter().map(|glyph| glyph.advance_px).sum();
        assert!((sum - run.advance_px).abs() < 1e-3);
    }
    for (byte, covered) in covered.iter().enumerate() {
        assert!(
            *covered || text[byte..].starts_with('\n'),
            "byte {byte} unshaped"
        );
    }
}

#[test]
fn ligatures_merge_clusters_and_disabling_liga_splits_them() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("office");
    let on = shape(
        &mut shaper,
        &mut fonts,
        &source,
        &style("Noto Sans SC"),
        DirSpec::Ltr,
    );
    assert_source_mapping("office", &on);
    let glyphs = &on.runs[0].glyphs;
    assert!(glyphs.len() < 6, "ffi ligates: {} glyphs", glyphs.len());
    let ligature = glyphs.iter().find(|glyph| glyph.cluster == 1).unwrap();
    assert_eq!(ligature.cluster_end, 4, "one cluster covers f, f, i");

    let mut no_liga = style("Noto Sans SC");
    no_liga.features = vec![FontFeatureSetting::new(*b"liga", 0)];
    let off = shape(&mut shaper, &mut fonts, &source, &no_liga, DirSpec::Ltr);
    assert_eq!(off.runs[0].glyphs.len(), 6);
    assert_eq!(
        shaper.counters().shape_cache_misses,
        2,
        "features are in the key"
    );
}

#[test]
fn combining_marks_stay_in_their_base_cluster() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let text = "e\u{301}x";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("Noto Sans SC"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    for glyph in &shaped.runs[0].glyphs {
        if glyph.cluster == 0 {
            assert_eq!(glyph.cluster_end, 3, "e + U+0301 is one cluster");
        }
    }
}

#[test]
fn arabic_shapes_contextually_and_right_to_left() {
    let (mut fonts, _) = system(&["noto-sans-arabic"]);
    let mut shaper = Shaper::default();
    let arabic = style("Noto Sans Arabic");
    let word = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new("بب"),
        &arabic,
        DirSpec::Rtl,
    );
    let alone = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new("ب"),
        &arabic,
        DirSpec::Rtl,
    );
    let run = &word.runs[0];
    assert_eq!(run.direction, RunDirection::Rtl);
    assert_eq!(run.bidi_level, 1);
    assert_eq!(run.script, ScriptTag::ARABIC);
    // Visual order: the second (logical) letter comes first.
    assert_eq!(run.glyphs[0].cluster, 2);
    // Noto builds beh from a shared skeleton plus a dot glyph, so compare the
    // whole joined sequence against two isolated letters, ids and positions.
    let form = |glyphs: &[nana_text::ShapedGlyph]| -> Vec<(u32, i32, i32)> {
        glyphs
            .iter()
            .map(|glyph| {
                (
                    glyph.glyph_id,
                    (glyph.offset_x_px * 64.0) as i32,
                    (glyph.advance_px * 64.0) as i32,
                )
            })
            .collect()
    };
    let isolated = form(&alone.runs[0].glyphs);
    let mut unjoined = isolated.clone();
    unjoined.extend(isolated);
    assert_ne!(
        form(&run.glyphs),
        unjoined,
        "joined forms differ from isolated ones"
    );
}

#[test]
fn mixed_bidi_keeps_levels_per_run_and_reorders_visually() {
    let (mut fonts, ids) = system(&["noto-sans-sc", "noto-sans-arabic"]);
    let (sc, arabic) = (ids[0], ids[1]);
    let mut shaper = Shaper::default();
    let text = "abc العربية def";
    let chain = style("Noto Sans SC, Noto Sans Arabic");
    let ltr = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &chain,
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &ltr);
    let summary: Vec<(u8, FontId)> = ltr
        .runs
        .iter()
        .map(|run| (run.bidi_level, run.font))
        .collect();
    assert_eq!(summary.first(), Some(&(0, sc)));
    assert!(summary.contains(&(1, arabic)));
    assert_eq!(summary.last(), Some(&(0, sc)));
    let arabic_run = ltr.runs.iter().position(|run| run.font == arabic).unwrap();
    assert!(
        ltr.runs[arabic_run]
            .glyphs
            .iter()
            .all(|glyph| !glyph.flags.contains(GlyphFlags::MISSING))
    );

    // Latin 0 | Arabic 1 | Latin 0, and Latn | Arab | Latn: the separating
    // spaces inherit a neighbour's level and script instead of opening runs.
    let counters = shaper.counters();
    assert_eq!((counters.bidi_runs, counters.script_runs), (3, 3));

    let rtl = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &chain,
        DirSpec::Rtl,
    );
    assert_eq!(rtl.paragraphs[0].base_level, 1);
    assert_eq!(
        rtl.runs[0].bidi_level, 2,
        "Latin nests at 2 in an RTL paragraph"
    );
    let order = rtl.visual_order(0..rtl.runs.len());
    assert_eq!(
        rtl.runs[order[0]].source.start,
        rtl.runs.last().unwrap().source.start,
        "the logically last run is visually first in an RTL paragraph"
    );
}

#[test]
fn rtl_numbers_and_punctuation_get_their_bidi_levels() {
    let (mut fonts, _) = system(&["noto-sans-arabic", "noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let text = "عربي 123!";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("Noto Sans Arabic, Noto Sans SC"),
        DirSpec::Rtl,
    );
    assert_source_mapping(text, &shaped);
    let level_at = |byte: usize| {
        shaped
            .runs
            .iter()
            .find(|run| run.source.contains(&byte))
            .map(|run| run.bidi_level)
    };
    let digits = text.find('1').unwrap();
    assert_eq!(level_at(digits), Some(2), "European digits nest as LTR");
    assert_eq!(
        level_at(text.find('!').unwrap()),
        Some(1),
        "trailing punctuation follows the paragraph"
    );
}

#[test]
fn cjk_after_latin_falls_back_by_coverage_and_flags_the_fallback_face() {
    let (mut fonts, ids) = system(&["nana-test-vf", "noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let text = "A中文";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("NanaTestVF, Noto Sans SC"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    assert_eq!(shaped.runs.len(), 2);
    assert_eq!(shaped.runs[0].font, ids[0]);
    assert_eq!(shaped.runs[1].font, ids[1]);
    assert_eq!(shaped.runs[1].script, ScriptTag(*b"Hani"));
    assert!(
        shaped.runs[1]
            .glyphs
            .iter()
            .all(|glyph| glyph.flags.contains(GlyphFlags::FALLBACK_FONT))
    );
    assert!(
        shaped.runs[0]
            .glyphs
            .iter()
            .all(|glyph| !glyph.flags.contains(GlyphFlags::FALLBACK_FONT))
    );
}

#[test]
fn an_emoji_zwj_sequence_is_one_cluster_and_one_glyph() {
    let (mut fonts, _) = system(&["noto-emoji"]);
    let mut shaper = Shaper::default();
    let text = "👩\u{200D}💻";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("Noto Emoji"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    let glyphs = &shaped.runs[0].glyphs;
    assert_eq!(glyphs.len(), 1);
    assert_eq!((glyphs[0].cluster, glyphs[0].cluster_end), (0, 11));
}

#[test]
fn variation_axes_reach_shaping_and_the_cache_key() {
    let (mut fonts, _) = system(&["nana-test-vf"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("A");
    let advance = |shaper: &mut Shaper, fonts: &mut FontSystem, wdth: Option<f32>| {
        let mut vf = style("NanaTestVF");
        vf.variations = wdth
            .map(|value| vec![FontVariationSetting::new(*b"wdth", value)])
            .unwrap_or_default();
        shape(shaper, fonts, &source, &vf, DirSpec::Ltr).runs[0].advance_px
    };
    let normal = advance(&mut shaper, &mut fonts, None);
    let wide = advance(&mut shaper, &mut fonts, Some(200.0));
    assert!(wide > normal, "{wide} > {normal}");
    assert_eq!(shaper.counters().shape_cache_misses, 2);
}

#[test]
fn a_run_names_the_instance_its_glyphs_were_shaped_at() {
    // A rasterizer drawing these glyphs needs the coordinates the advances
    // were measured with, not a second resolution from the style.
    let (mut fonts, ids) = system(&["nana-test-vf"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("A");
    let mut vf = style("NanaTestVF");
    vf.variations = vec![FontVariationSetting::new(*b"wdth", 200.0)];
    let shaped = shape(&mut shaper, &mut fonts, &source, &vf, DirSpec::Ltr);
    let run = &shaped.runs[0];
    let instance = run.instance.as_ref().expect("the native shaper reports it");
    assert_eq!(instance.font, run.font);
    assert_eq!(instance.font, ids[0]);
    assert!(
        instance
            .coords
            .iter()
            .any(|coord| coord.tag == *b"wdth" && coord.value > 100.0),
        "{:?}",
        instance.coords
    );
}

#[test]
fn the_language_hint_reaches_opentype_locl() {
    // Noto Sans SC substitutes its digit glyphs through `locl` under the
    // `ZHS` language system of real scripts (`latn`, `hani`, ...), so the digit
    // follows a Latin letter to inherit a script.
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("a0");
    let sc = style("Noto Sans SC");
    let hans = nana_text::font::LanguageTag::new("zh-Hans").unwrap();
    let constraints = TextConstraints::default();
    let plain = shaper.shape(&mut fonts, &ShapeRequest::new(&source, &sc, &constraints));
    let localized = shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &sc, &constraints).with_language(Some(&hans)),
    );
    assert_eq!(
        plain.runs[0].glyphs[0].glyph_id, localized.runs[0].glyphs[0].glyph_id,
        "the letter has no localized form"
    );
    assert_ne!(
        plain.runs[0].glyphs[1].glyph_id, localized.runs[0].glyphs[1].glyph_id,
        "the digit does"
    );
    assert_eq!(
        shaper.counters().shape_cache_misses,
        2,
        "language is in the key"
    );
}

#[test]
fn a_glyph_no_face_has_is_kept_as_notdef_on_the_primary() {
    let (mut fonts, ids) = system(&["nana-test-vf"]);
    let mut shaper = Shaper::default();
    let text = "AΩ";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("NanaTestVF"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    let glyphs: Vec<_> = shaped
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter())
        .collect();
    assert!(shaped.runs.iter().all(|run| run.font == ids[0]));
    assert!(!glyphs[0].flags.contains(GlyphFlags::MISSING));
    assert!(glyphs[1].flags.contains(GlyphFlags::MISSING));
    assert_eq!(glyphs[1].glyph_id, 0);
}

#[test]
fn a_covered_cluster_that_shapes_to_notdef_is_retried_with_the_next_face() {
    let (mut fonts, ids) = system(&["nana-test-notdef", "nana-test-axes"]);
    let mut shaper = Shaper::default();
    let text = "ABA";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("NanaTestNotdef, NanaTestAxes"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    let summary: Vec<(std::ops::Range<usize>, FontId)> = shaped
        .runs
        .iter()
        .map(|run| (run.source.clone(), run.font))
        .collect();
    assert_eq!(
        summary,
        vec![(0..1, ids[0]), (1..2, ids[1]), (2..3, ids[0])]
    );
    assert!(
        shaped
            .runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .all(|glyph| glyph.glyph_id != 0)
    );
    let counters = shaper.counters();
    assert_eq!(counters.fallback_retries, 1);
    assert!(counters.fallback_fonts_examined >= 1);
}

#[test]
fn a_style_span_crossing_scripts_splits_runs_at_both_boundaries() {
    let (mut fonts, _) = system(&["noto-sans-sc", "noto-sans-arabic"]);
    let mut shaper = Shaper::default();
    let text = "abc عربي";
    let mut source = TextSource::new(text);
    let mut big = style("Noto Sans SC, Noto Sans Arabic");
    big.font_size_px = 30.0;
    // From "c" into the Arabic word, ending inside it.
    source.set_spans(vec![TextSpan {
        range: 2..8,
        style: big,
        composition: Some(CompositionSegment::Preedit),
    }]);
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &source,
        &style("Noto Sans SC, Noto Sans Arabic"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    let cuts: Vec<(usize, f32)> = shaped
        .runs
        .iter()
        .map(|run| (run.source.start, run.font_size_px))
        .collect();
    assert!(cuts.contains(&(0, 20.0)));
    assert!(cuts.contains(&(2, 30.0)), "{cuts:?}");
    assert!(
        cuts.iter()
            .any(|(start, size)| *start == 4 && *size == 30.0),
        "{cuts:?}"
    );
    assert!(
        cuts.iter()
            .any(|(start, size)| *start == 8 && *size == 20.0),
        "{cuts:?}"
    );
}

#[test]
fn a_span_boundary_inside_a_grapheme_moves_to_the_cluster_start() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let text = "ae\u{301}b";
    let mut source = TextSource::new(text);
    let mut big = style("Noto Sans SC");
    big.font_size_px = 30.0;
    // Starts between e and its combining accent.
    source.set_spans(vec![TextSpan {
        range: 2..4,
        style: big,
        composition: None,
    }]);
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &source,
        &style("Noto Sans SC"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    assert!(shaped.runs.iter().all(|run| run.source.start != 2));
}

#[test]
fn a_family_list_that_resolves_to_nothing_still_yields_missing_glyphs() {
    let (mut fonts, ids) = system(&["nana-test-vf"]);
    let mut shaper = Shaper::default();
    let text = "Ω";
    let shaped = shape(
        &mut shaper,
        &mut fonts,
        &TextSource::new(text),
        &style("Inter"),
        DirSpec::Ltr,
    );
    assert_source_mapping(text, &shaped);
    assert_eq!(shaped.runs.len(), 1, "the text does not vanish");
    assert_eq!(shaped.runs[0].font, ids[0]);
    assert!(shaped.runs[0].glyphs[0].flags.contains(GlyphFlags::MISSING));
    assert_eq!(shaper.counters().text_bytes_unshaped, 0);

    let mut empty = FontSystem::with_policy(FallbackPolicy::empty());
    let nothing = shape(
        &mut shaper,
        &mut empty,
        &TextSource::new("ab\ncd"),
        &style("Inter"),
        DirSpec::Ltr,
    );
    assert!(nothing.runs.is_empty());
    assert_eq!(
        shaper.counters().text_bytes_unshaped,
        4,
        "an empty font system is the only case left unshaped, and it is counted"
    );
}

// ---- cache ----------------------------------------------------------------

#[test]
fn one_shaper_serving_two_font_systems_never_mixes_their_faces() {
    // Both systems issue FontId(0, 1) at generation 1, for different faces.
    let (mut sc_fonts, sc_ids) = system(&["noto-sans-sc"]);
    let (mut kr_fonts, kr_ids) = system(&["noto-sans-kr"]);
    assert_eq!(sc_ids[0], kr_ids[0]);
    assert_eq!(sc_fonts.generation(), kr_fonts.generation());

    let mut shaper = Shaper::default();
    let source = TextSource::new("한");
    let chain = style("Noto Sans SC, Noto Sans KR");
    let in_sc = shape(&mut shaper, &mut sc_fonts, &source, &chain, DirSpec::Ltr);
    let in_kr = shape(&mut shaper, &mut kr_fonts, &source, &chain, DirSpec::Ltr);
    assert!(in_sc.runs[0].glyphs[0].flags.contains(GlyphFlags::MISSING));
    assert!(
        !in_kr.runs[0].glyphs[0].flags.contains(GlyphFlags::MISSING),
        "the KR system shapes with its own face, not a cached SC result"
    );
    assert_eq!(shaper.counters().shape_cache_misses, 2);
}

#[test]
fn which_overlapping_span_is_a_composition_span_is_part_of_the_key() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let base = style("Noto Sans SC");
    let sized = |size: f32| TextStyle {
        font_size_px: size,
        ..style("Noto Sans SC")
    };
    let spans = |composition_on_first: bool| {
        let mut source = TextSource::new("abcd");
        let marker = |on: bool| on.then_some(CompositionSegment::Preedit);
        source.set_spans(vec![
            TextSpan {
                range: 0..4,
                style: sized(30.0),
                composition: marker(composition_on_first),
            },
            TextSpan {
                range: 0..4,
                style: sized(40.0),
                composition: marker(!composition_on_first),
            },
        ]);
        source
    };
    let first = shape(&mut shaper, &mut fonts, &spans(true), &base, DirSpec::Ltr);
    let second = shape(&mut shaper, &mut fonts, &spans(false), &base, DirSpec::Ltr);
    assert_eq!(
        first.runs[0].font_size_px, 30.0,
        "the composition span wins"
    );
    assert_eq!(second.runs[0].font_size_px, 40.0);
    assert_eq!(shaper.counters().shape_cache_misses, 2);
}

#[test]
fn ten_thousand_identical_labels_shape_once() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let label = style("Noto Sans SC");
    let sources: Vec<TextSource> = (0..10_000).map(|_| TextSource::new("Settings")).collect();
    for source in &sources {
        shape(&mut shaper, &mut fonts, source, &label, DirSpec::Ltr);
    }
    let counters = shaper.counters();
    assert_eq!(counters.shape_requests, 10_000);
    assert_eq!(counters.shape_cache_misses, 1);
    assert_eq!(counters.shape_cache_hits, 9_999);
    assert_eq!(counters.shape_runs_created, 1);
    assert_eq!(counters.text_bytes_cloned_for_shape, 0);
}

#[test]
fn repeated_lookups_of_one_source_hash_its_text_once_and_copy_nothing() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("a long paragraph ".repeat(64));
    for _ in 0..100 {
        shape(
            &mut shaper,
            &mut fonts,
            &source,
            &style("Noto Sans SC"),
            DirSpec::Ltr,
        );
    }
    let counters = shaper.counters();
    assert_eq!(counters.text_bytes_hashed, source.text().len());
    assert_eq!(counters.text_bytes_cloned_for_shape, 0);
    assert_eq!(counters.shape_cache_hits, 99);
}

#[test]
fn width_line_height_and_revision_changes_do_not_reshape() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let mut source = TextSource::new("wrap me");
    let base = style("Noto Sans SC");
    let first = shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &base, &TextConstraints::default()),
    );

    let narrow = TextConstraints {
        max_width_px: Some(12.0),
        wrap: Some(nana_ui_core::TextWrapBreak::Word),
        max_lines: Some(2),
        ..TextConstraints::default()
    };
    let relaid = shaper.shape(&mut fonts, &ShapeRequest::new(&source, &base, &narrow));
    assert!(
        Arc::ptr_eq(&first, &relaid),
        "constraint-only relayout reuses the result"
    );

    let mut taller = base.clone();
    taller.line_height = Some(nana_ui_core::LineHeightSpec::Relative(2.0));
    shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &taller, &TextConstraints::default()),
    );

    // An edit that restores the same text is a new revision but the same content.
    source.set_text("wrap me");
    shaper.shape(
        &mut fonts,
        &ShapeRequest::new(&source, &base, &TextConstraints::default()),
    );

    let counters = shaper.counters();
    assert_eq!(counters.shape_runs_created, first.runs.len());
    assert_eq!(counters.shape_cache_misses, 1);
}

#[test]
fn direction_scale_and_font_generation_do_reshape() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::default();
    let source = TextSource::new("abc");
    let base = style("Noto Sans SC");
    let constraints = TextConstraints::default();
    shaper.shape(&mut fonts, &ShapeRequest::new(&source, &base, &constraints));
    let rtl = TextConstraints {
        base_direction: DirSpec::Rtl,
        ..constraints
    };
    shaper.shape(&mut fonts, &ShapeRequest::new(&source, &base, &rtl));
    let scaled = TextConstraints {
        scale: TextScale {
            px_per_logical: 1.5,
        },
        ..constraints
    };
    let big = shaper.shape(&mut fonts, &ShapeRequest::new(&source, &base, &scaled));
    assert_eq!(big.runs[0].font_size_px, 30.0);
    assert_eq!(shaper.counters().shape_cache_misses, 3);
    assert_eq!(shaper.counters().shape_cache_entries, 3);

    fonts
        .register_bytes(
            font_blob(fixture("noto-sans-kr")),
            &FaceDescriptor::default(),
        )
        .unwrap();
    let after = shaper.shape(&mut fonts, &ShapeRequest::new(&source, &base, &constraints));
    assert_eq!(after.font_generation, fonts.generation());
    let counters = shaper.counters();
    assert_eq!(counters.shape_cache_misses, 4);
    assert_eq!(
        counters.shape_cache_evictions, 3,
        "older-generation entries are purged"
    );
    assert_eq!(counters.shape_cache_entries, 1);
}

#[test]
fn the_cache_is_bounded_by_entries_and_bytes() {
    let (mut fonts, _) = system(&["noto-sans-sc"]);
    let mut shaper = Shaper::new(ShapeCacheBudget {
        max_entries: 4,
        max_bytes: usize::MAX,
    });
    let base = style("Noto Sans SC");
    let sources: Vec<TextSource> = (0..10)
        .map(|n| TextSource::new(format!("label {n}")))
        .collect();
    for source in &sources {
        shape(&mut shaper, &mut fonts, source, &base, DirSpec::Ltr);
    }
    let counters = shaper.counters();
    assert_eq!(counters.shape_cache_entries, 4);
    assert_eq!(counters.shape_cache_evictions, 6);

    // The most recent entries survive: touching one keeps it past the next evictions.
    shape(&mut shaper, &mut fonts, &sources[9], &base, DirSpec::Ltr);
    assert_eq!(shaper.counters().shape_cache_hits, 1);
    shape(&mut shaper, &mut fonts, &sources[0], &base, DirSpec::Ltr);
    assert_eq!(shaper.counters().shape_cache_misses, 11);

    let bytes = shaper.counters().shape_cache_bytes;
    assert!(bytes > 0);
    shaper.set_budget(ShapeCacheBudget {
        max_entries: 4,
        max_bytes: bytes / 2,
    });
    let counters = shaper.counters();
    assert!(counters.shape_cache_bytes <= bytes / 2);
    assert!(counters.shape_cache_entries < 4);
}
