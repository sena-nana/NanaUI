//! Responsive policy (Issue #265): a node takes a style variant by the size
//! of a container box, from bounded size buckets.
//!
//! A rule is style intent. The variant its bucket picks is written over the
//! node's authored layout as if the author had written it, so paint,
//! typography and layout all follow it, and design intent resolves over the
//! result as it does over any authored style. A bucket change is therefore a
//! style change to the node, classified and seeded like one, and laid out by
//! the one incremental layout. No query engine, renderer or callback writes
//! a box.
//!
//! Rules are indexed by their container. A container whose content box moved
//! on the axis its rules read evaluates those rules and no other; a rule
//! that lands in the bucket it was in changes nothing.

use std::fmt;
use std::sync::Arc;

use nana_ui_core::{LayoutFieldSet, LayoutStyle};

use crate::StableNodeId;

/// The most breakpoints a rule takes: buckets are bounded.
pub const MAX_RESPONSIVE_BREAKPOINTS: usize = 16;

/// The box whose size a rule reads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResponsiveContainer {
    /// The node's parent: the space the node is given.
    Parent,
    /// A named box: a dock panel, a page.
    Node(StableNodeId),
    /// The nearest box above the node that is a query container for the
    /// rule's axis (`container-type`), and answers to `name` when one is
    /// given (`container-name`): CSS `@container`. While there is none, the
    /// node keeps its authored style.
    Nearest { name: Option<String> },
}

/// The axis of the container's content box a rule reads. Inline and block
/// follow the container's writing mode; width and height do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponsiveAxis {
    Inline,
    Block,
    Width,
    Height,
}

/// A style a rule applies in one bucket, written over the node's authored
/// layout.
///
/// Built from data ([`Self::between`]) it is the layout fields that differ
/// with their values, and two variants are equal when they write the same
/// values; built from a closure ([`Self::patch`]) two are equal when they are
/// one closure.
#[derive(Clone)]
pub struct StyleVariant(Variant);

#[derive(Clone)]
enum Variant {
    Fields {
        fields: LayoutFieldSet,
        /// The written values; every other field is the default.
        values: Arc<LayoutStyle>,
    },
    Patch(Arc<dyn Fn(&mut LayoutStyle) + Send + Sync>),
}

impl StyleVariant {
    /// The fields `styled` writes over `base`, with `styled`'s values;
    /// `None` when the two are equal. Written over another base, it changes
    /// those fields and keeps the rest of that base.
    pub fn between(base: &LayoutStyle, styled: &LayoutStyle) -> Option<Self> {
        let fields = base.differing_fields(styled);
        if fields.is_empty() {
            return None;
        }
        let mut values = LayoutStyle::default();
        values.copy_fields(styled, &fields);
        Some(Self(Variant::Fields {
            fields,
            values: Arc::new(values),
        }))
    }

    /// A variant that runs `patch` over the node's authored layout.
    pub fn patch(patch: impl Fn(&mut LayoutStyle) + Send + Sync + 'static) -> Self {
        Self(Variant::Patch(Arc::new(patch)))
    }

    /// Write the variant over `layout`.
    pub fn apply(&self, layout: &mut LayoutStyle) {
        match &self.0 {
            Variant::Fields { fields, values } => layout.copy_fields(values, fields),
            Variant::Patch(patch) => patch(layout),
        }
    }
}

impl fmt::Debug for StyleVariant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Variant::Fields { fields, .. } => {
                write!(formatter, "StyleVariant({} fields)", fields.len())
            }
            Variant::Patch(_) => formatter.write_str("StyleVariant(..)"),
        }
    }
}

impl PartialEq for StyleVariant {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (
                Variant::Fields { fields, values },
                Variant::Fields {
                    fields: other_fields,
                    values: other_values,
                },
            ) => {
                fields == other_fields
                    && (Arc::ptr_eq(values, other_values) || values == other_values)
            }
            (Variant::Patch(patch), Variant::Patch(other)) => Arc::ptr_eq(patch, other),
            _ => false,
        }
    }
}

/// A node's responsive policy: the container size it follows, where that
/// size's buckets break, and the variant the node takes in each.
///
/// Bucket 0 lies below the first breakpoint; bucket `i` runs from
/// breakpoint `i - 1` up to the next. A bucket with no variant keeps the
/// node's authored style.
///
/// ```
/// # use nana_ui_runtime::{ResponsiveAxis, ResponsiveContainer, ResponsiveRule};
/// # use nana_ui_core::FlexDirection;
/// // A toolbar stacks its items below 480 px of the space it is given.
/// let compact = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
///     .below(480.0, |layout| layout.direction = Some(FlexDirection::Column));
/// assert_eq!(compact.bucket_for(479.0), 0);
/// assert_eq!(compact.bucket_for(480.0), 1);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ResponsiveRule {
    container: ResponsiveContainer,
    axis: ResponsiveAxis,
    breakpoints: Vec<f32>,
    variants: Vec<Option<StyleVariant>>,
}

impl ResponsiveRule {
    /// A rule on `axis` of `container` with one bucket: the node's authored
    /// style at every size.
    pub fn new(container: ResponsiveContainer, axis: ResponsiveAxis) -> Self {
        Self {
            container,
            axis,
            breakpoints: Vec::new(),
            variants: vec![None],
        }
    }

    /// A rule from its buckets: `breakpoints` finite and strictly ascending,
    /// at most [`MAX_RESPONSIVE_BREAKPOINTS`], and one variant or `None` per
    /// bucket, `breakpoints.len() + 1` of them. `None` when they are not.
    pub fn from_buckets(
        container: ResponsiveContainer,
        axis: ResponsiveAxis,
        breakpoints: Vec<f32>,
        variants: Vec<Option<StyleVariant>>,
    ) -> Option<Self> {
        let rule = Self {
            container,
            axis,
            breakpoints,
            variants,
        };
        rule.is_valid().then_some(rule)
    }

    /// Below `extent`, apply `patch`; from `extent` up, the buckets above.
    /// Sets the first breakpoint, so call it once, before
    /// [`Self::at_least`].
    pub fn below(
        mut self,
        extent: f32,
        patch: impl Fn(&mut LayoutStyle) + Send + Sync + 'static,
    ) -> Self {
        self.variants[0] = Some(StyleVariant::patch(patch));
        self.breakpoints.insert(0, extent);
        self.variants.insert(1, None);
        self
    }

    /// From `extent` up to the next breakpoint, apply `patch`. Breakpoints
    /// ascend: `extent` lies above every one already set.
    pub fn at_least(
        mut self,
        extent: f32,
        patch: impl Fn(&mut LayoutStyle) + Send + Sync + 'static,
    ) -> Self {
        self.breakpoints.push(extent);
        self.variants.push(Some(StyleVariant::patch(patch)));
        self
    }

    /// From `extent` up to the next breakpoint, keep the node's authored
    /// style.
    pub fn own_from(mut self, extent: f32) -> Self {
        self.breakpoints.push(extent);
        self.variants.push(None);
        self
    }

    pub fn container(&self) -> &ResponsiveContainer {
        &self.container
    }

    pub fn axis(&self) -> ResponsiveAxis {
        self.axis
    }

    /// The bucket a container extent falls in.
    pub fn bucket_for(&self, extent: f32) -> usize {
        self.breakpoints
            .partition_point(|breakpoint| *breakpoint <= extent)
    }

    /// The variant of `bucket`, if it has one.
    pub(crate) fn variant(&self, bucket: usize) -> Option<&StyleVariant> {
        self.variants.get(bucket).and_then(Option::as_ref)
    }

    /// Finite, strictly ascending breakpoints, at most
    /// [`MAX_RESPONSIVE_BREAKPOINTS`] of them, and a variant slot per bucket.
    pub(crate) fn is_valid(&self) -> bool {
        self.breakpoints.len() <= MAX_RESPONSIVE_BREAKPOINTS
            && self.variants.len() == self.breakpoints.len() + 1
            && self
                .breakpoints
                .iter()
                .all(|breakpoint| breakpoint.is_finite())
            && self.breakpoints.windows(2).all(|pair| pair[0] < pair[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::LengthSpec;

    #[test]
    fn buckets_break_at_each_breakpoint_from_below() {
        let rule = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
            .below(480.0, |_| {})
            .at_least(960.0, |_| {});
        assert!(rule.is_valid());
        assert_eq!(rule.bucket_for(0.0), 0);
        assert_eq!(rule.bucket_for(479.9), 0);
        assert_eq!(rule.bucket_for(480.0), 1);
        assert_eq!(rule.bucket_for(959.0), 1);
        assert_eq!(rule.bucket_for(960.0), 2);
        assert!(rule.variant(0).is_some());
        assert!(rule.variant(1).is_none());
        assert!(rule.variant(2).is_some());
    }

    #[test]
    fn breakpoints_must_ascend_and_stay_bounded() {
        let descending = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
            .own_from(960.0)
            .own_from(480.0);
        assert!(!descending.is_valid());
        let nan = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Block)
            .own_from(f32::NAN);
        assert!(!nan.is_valid());
        let mut many = ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline);
        for step in 0..=MAX_RESPONSIVE_BREAKPOINTS {
            many = many.own_from(step as f32 * 10.0);
        }
        assert!(!many.is_valid());
        let short = ResponsiveRule::from_buckets(
            ResponsiveContainer::Parent,
            ResponsiveAxis::Inline,
            vec![480.0],
            vec![None],
        );
        assert!(short.is_none(), "a variant slot per bucket");
    }

    /// A data variant writes the fields it was taken from and keeps the rest
    /// of whatever it is written over; equal data is one variant, so a rule
    /// built again from the same styles equals the first.
    #[test]
    fn a_data_variant_writes_its_fields_and_compares_by_value() {
        let base = LayoutStyle::default();
        let narrow = LayoutStyle {
            width: Some(LengthSpec::Px(120.0)),
            background: Some([0.0, 0.0, 1.0, 1.0]),
            ..LayoutStyle::default()
        };
        assert!(StyleVariant::between(&base, &base).is_none());
        let variant = StyleVariant::between(&base, &narrow).unwrap();
        let mut authored = LayoutStyle {
            height: Some(LengthSpec::Px(40.0)),
            ..LayoutStyle::default()
        };
        variant.apply(&mut authored);
        assert_eq!(authored.width, Some(LengthSpec::Px(120.0)));
        assert_eq!(authored.background, Some([0.0, 0.0, 1.0, 1.0]));
        assert_eq!(authored.height, Some(LengthSpec::Px(40.0)));
        assert_eq!(variant, StyleVariant::between(&base, &narrow).unwrap());
        let build = || {
            ResponsiveRule::from_buckets(
                ResponsiveContainer::Nearest {
                    name: Some("card".into()),
                },
                ResponsiveAxis::Width,
                vec![480.0f32.next_up()],
                vec![StyleVariant::between(&base, &narrow), None],
            )
            .unwrap()
        };
        assert_eq!(build(), build());
        assert_ne!(StyleVariant::patch(|_| {}), StyleVariant::patch(|_| {}));
    }
}
