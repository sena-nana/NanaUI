//! Props: a constant, a signal, or a closure, bound to one component field.
//!
//! A constant is written into the component before it is created and costs
//! nothing afterwards. A signal or computed is a direct binding: a field
//! writer and the signal's id, no closure. A closure is boxed once.

use std::any::Any;
use std::cell::RefCell;
use std::panic::Location;
use std::sync::Arc;

use std::marker::PhantomData;

use super::node::{DirectBinding, DynBinding, DynProp, NodeBindings};
use super::reactive::{Computed, Const, Signal};

/// Writes one field of `C`. The view layer generates these per bindable
/// field; applications write their own for [`super::El::prop`].
pub trait FieldWrite<C, T> {
    /// `Component.field`, for traces.
    const FIELD: &'static str;
    fn write(target: &mut C, value: T);

    /// Whether writing `value` would change `target`. A binding whose
    /// fields all compare equal costs no copy and no projection; the default
    /// assumes every write changes something.
    fn differs(target: &C, value: &T) -> bool {
        let _ = (target, value);
        true
    }
}

/// Interactive controls that can report whether the user currently owns their
/// transient state (an active pointer drag, an open color picker popover, or
/// an uncommitted numeric text draft). Used by [`WhileIdle`].
pub trait InteractiveIdle {
    /// Returns `true` when external reactive updates may safely write into
    /// this control without clobbering in-flight user interaction.
    fn is_idle(&self) -> bool;
}

/// Wraps any [`FieldWrite`] so updates are skipped while the target control is
/// actively being interacted with ([`InteractiveIdle::is_idle`] returns
/// `false`). Useful for live-polled telemetry/store bindings.
pub struct WhileIdle<W>(PhantomData<W>);

impl<C, T, W> FieldWrite<C, T> for WhileIdle<W>
where
    C: InteractiveIdle,
    W: FieldWrite<C, T>,
{
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: T) {
        if target.is_idle() {
            W::write(target, value);
        }
    }

    fn differs(target: &C, value: &T) -> bool {
        target.is_idle() && W::differs(target, value)
    }
}

/// Adapts an `Option<Arc<str>>` writer `W` to accept `Arc<str>`, mapping an
/// empty string to `None` and non-empty strings to `Some(value)`.
pub struct NonEmpty<W>(PhantomData<W>);

impl<C, W: FieldWrite<C, Option<Arc<str>>>> FieldWrite<C, Arc<str>> for NonEmpty<W> {
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: Arc<str>) {
        let mapped = (!value.is_empty()).then_some(value);
        W::write(target, mapped);
    }

    fn differs(target: &C, value: &Arc<str>) -> bool {
        let mapped = (!value.is_empty()).then(|| Arc::clone(value));
        W::differs(target, &mapped)
    }
}

/// Adapts an `Option<Arc<str>>` writer `W` to accept `String`, mapping an
/// empty string to `None` and non-empty strings to `Some(Arc::from(value))`.
pub struct NonEmptyString<W>(PhantomData<W>);

impl<C, W: FieldWrite<C, Option<Arc<str>>>> FieldWrite<C, String> for NonEmptyString<W> {
    const FIELD: &'static str = W::FIELD;

    fn write(target: &mut C, value: String) {
        let mapped = (!value.is_empty()).then(|| Arc::from(value));
        W::write(target, mapped);
    }

    fn differs(target: &C, value: &String) -> bool {
        let mapped = (!value.is_empty()).then(|| Arc::from(value.as_str()));
        W::differs(target, &mapped)
    }
}

/// A value, signal or closure that can drive a `T` field.
pub trait IntoProp<T: 'static>: Sized {
    #[doc(hidden)]
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    );

    #[doc(hidden)]
    fn into_source(self) -> PropSource<T>;
}

/// A prop the control may also write back to: what `.model` (`v-model`)
/// takes. A [`Signal`] or a store path ([`super::Store`],
/// [`super::Subfield`], [`super::Item`]), which carries both the read and
/// the write, so binding a store field needs no handler of its own.
pub trait Writable<T: 'static>: IntoProp<T> + Copy + Send + 'static {
    /// Write the control's new value and notify readers. Does nothing once
    /// a list item on a store path is gone.
    #[track_caller]
    fn write_back(&self, value: T);
}

impl<T: Clone + 'static> Writable<T> for Signal<T> {
    #[track_caller]
    fn write_back(&self, value: T) {
        self.set(value);
    }
}

/// A prop kept for later evaluation (conditions, list sources).
pub enum PropSource<T: 'static> {
    Const(T),
    Signal(Signal<T>),
    Computed(Computed<T>),
    Dyn(Box<dyn Fn() -> T + Send>),
}

impl<T: Clone + 'static> PropSource<T> {
    /// The current value, tracked by the innermost effect.
    #[track_caller]
    pub fn get(&self) -> T {
        match self {
            Self::Const(value) => value.clone(),
            Self::Signal(signal) => signal.get(),
            Self::Computed(computed) => computed.get(),
            Self::Dyn(f) => f(),
        }
    }
}

fn apply_cell<C, T: Clone + 'static, W: FieldWrite<C, T>>(target: &mut C, cell: &dyn Any) {
    let cell = cell
        .downcast_ref::<RefCell<T>>()
        .expect("signal cell holds its own type");
    W::write(target, cell.borrow().clone());
}

fn differs_cell<C, T: 'static, W: FieldWrite<C, T>>(target: &C, cell: &dyn Any) -> bool {
    let cell = cell
        .downcast_ref::<RefCell<T>>()
        .expect("signal cell holds its own type");
    W::differs(target, &cell.borrow())
}

fn bind_direct<C, T: Clone + 'static, W: FieldWrite<C, T>>(
    source: super::reactive::SignalKey,
    bindings: &mut NodeBindings<C>,
    site: &'static Location<'static>,
) {
    bindings.direct.push(DirectBinding {
        source,
        apply: apply_cell::<C, T, W>,
        differs: differs_cell::<C, T, W>,
        field: W::FIELD,
        site,
    });
}

/// A prop kept for later binds as what it was given as: a constant is
/// written, a signal or computed binds directly, a closure is boxed.
impl<T: Clone + Send + 'static> IntoProp<T> for PropSource<T> {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        match self {
            Self::Const(value) => W::write(target, value),
            Self::Signal(signal) => signal.bind_field::<C, W>(target, bindings, site),
            Self::Computed(computed) => computed.bind_field::<C, W>(target, bindings, site),
            Self::Dyn(f) => f.bind_field::<C, W>(target, bindings, site),
        }
    }

    fn into_source(self) -> PropSource<T> {
        self
    }
}

impl<T: Clone + 'static> IntoProp<T> for Signal<T> {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        _: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        bind_direct::<C, T, W>(self.key, bindings, site);
    }

    fn into_source(self) -> PropSource<T> {
        PropSource::Signal(self)
    }
}

impl<T: Clone + 'static> IntoProp<T> for Computed<T> {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        _: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        bind_direct::<C, T, W>(self.key, bindings, site);
    }

    fn into_source(self) -> PropSource<T> {
        PropSource::Computed(self)
    }
}

impl<T: Clone + 'static> IntoProp<T> for Const<T> {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        Fixed(self.get()).bind_field::<C, W>(target, bindings, site);
    }

    fn into_source(self) -> PropSource<T> {
        Fixed(self.get()).into_source()
    }
}

/// A value of any type used as a constant prop. The `.vue` compiler wraps
/// expressions it proved read no signal in it, so they are written once
/// instead of becoming a binding.
#[doc(hidden)]
pub struct Fixed<T>(pub T);

impl<T: 'static> IntoProp<T> for Fixed<T> {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        target: &mut C,
        _: &mut NodeBindings<C>,
        _: &'static Location<'static>,
    ) {
        W::write(target, self.0);
    }

    fn into_source(self) -> PropSource<T> {
        PropSource::Const(self.0)
    }
}

/// A closure prop: evaluated once per run, the value kept until written.
struct ClosureProp<F, T, W> {
    f: F,
    pending: Option<T>,
    _writer: PhantomData<fn() -> W>,
}

impl<C, T, F, W> DynProp<C> for ClosureProp<F, T, W>
where
    T: Send,
    F: Fn() -> T + Send,
    W: FieldWrite<C, T>,
{
    fn evaluate(&mut self, current: &C) -> bool {
        let value = (self.f)();
        let differs = W::differs(current, &value);
        self.pending = Some(value);
        differs
    }

    fn write(&mut self, target: &mut C) {
        if let Some(value) = self.pending.take() {
            W::write(target, value);
        }
    }

    fn discard(&mut self) {
        self.pending = None;
    }
}

impl<T: Send + 'static, F: Fn() -> T + Send + 'static> IntoProp<T> for F {
    fn bind_field<C: 'static, W: FieldWrite<C, T> + 'static>(
        self,
        _: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        bindings.dynamic.push(DynBinding {
            prop: Box::new(ClosureProp::<F, T, W> {
                f: self,
                pending: None,
                _writer: PhantomData,
            }),
            field: W::FIELD,
            site,
        });
    }

    fn into_source(self) -> PropSource<T> {
        PropSource::Dyn(Box::new(self))
    }
}

macro_rules! const_props {
    ($($from:ty => $to:ty, |$value:ident| $convert:expr;)*) => {$(
        impl IntoProp<$to> for $from {
            fn bind_field<C: 'static, W: FieldWrite<C, $to> + 'static>(
                self,
                target: &mut C,
                _: &mut NodeBindings<C>,
                _: &'static Location<'static>,
            ) {
                let $value = self;
                W::write(target, $convert);
            }

            fn into_source(self) -> PropSource<$to> {
                let $value = self;
                PropSource::Const($convert)
            }
        }
    )*};
}

const_props! {
    String => String, |v| v;
    &'static str => String, |v| v.to_owned();
    Arc<str> => Arc<str>, |v| v;
    &'static str => Arc<str>, |v| Arc::from(v);
    String => Arc<str>, |v| Arc::from(v);
    bool => bool, |v| v;
    f32 => f32, |v| v;
    f64 => f64, |v| v;
    i32 => i32, |v| v;
    i64 => i64, |v| v;
    u32 => u32, |v| v;
    u64 => u64, |v| v;
    usize => usize, |v| v;
    Option<Arc<str>> => Option<Arc<str>>, |v| v;
    Option<&'static str> => Option<Arc<str>>, |v| v.map(Arc::from);
    &'static str => Option<Arc<str>>, |v| Some(Arc::from(v));
    String => Option<Arc<str>>, |v| Some(Arc::from(v));
    crate::TextureSlotName => crate::TextureSlotName, |v| v;
    &'static str => crate::TextureSlotName, |v| crate::TextureSlotName::from(v);
    String => crate::TextureSlotName, |v| crate::TextureSlotName::from(v);
    Vec<crate::SelectOption> => Vec<crate::SelectOption>, |v| v;
    crate::StableNodeId => Option<crate::StableNodeId>, |v| Some(v);
    Option<crate::StableNodeId> => Option<crate::StableNodeId>, |v| v;
    nana_ui_core::SemanticColorRole => Option<nana_ui_core::SemanticColorRole>, |v| Some(v);
    Option<nana_ui_core::SemanticColorRole> => Option<nana_ui_core::SemanticColorRole>, |v| v;
    nana_ui_core::RadiusTier => Option<nana_ui_core::RadiusTier>, |v| Some(v);
    nana_ui_core::ElevationRole => Option<nana_ui_core::ElevationRole>, |v| Some(v);
    Option<nana_ui_core::ElevationRole> => Option<nana_ui_core::ElevationRole>, |v| v;
    nana_ui_core::PaintTransform => Option<nana_ui_core::PaintTransform>, |v| Some(v);
    Option<nana_ui_core::PaintTransform> => Option<nana_ui_core::PaintTransform>, |v| v;
    nana_ui_core::style_model::PaintColor => Option<nana_ui_core::style_model::PaintColor>, |v| Some(v);
    Option<nana_ui_core::style_model::PaintColor> => Option<nana_ui_core::style_model::PaintColor>, |v| v;
    nana_ui_core::FlexDirection => Option<nana_ui_core::FlexDirection>, |v| Some(v);
    Option<nana_ui_core::FlexDirection> => Option<nana_ui_core::FlexDirection>, |v| v;
    nana_ui_core::LengthSpec => Option<nana_ui_core::LengthSpec>, |v| Some(v);
    Option<nana_ui_core::LengthSpec> => Option<nana_ui_core::LengthSpec>, |v| v;
    crate::MenuItemMark => crate::MenuItemMark, |v| v;
    nana_ui_core::ColorOverrides => nana_ui_core::ColorOverrides, |v| v;
    nana_ui_core::ThemePatch => nana_ui_core::ThemePatch, |v| v;
    Option<nana_ui_core::RadiusTier> => Option<nana_ui_core::RadiusTier>, |v| v;
    nana_ui_core::Icon => nana_ui_core::Icon, |v| v;
    nana_ui_core::Icon => Option<nana_ui_core::Icon>, |v| Some(v);
    Option<nana_ui_core::Icon> => Option<nana_ui_core::Icon>, |v| v;
    nana_ui_core::StatusTone => nana_ui_core::StatusTone, |v| v;
    nana_ui_core::ValidationIntent => nana_ui_core::ValidationIntent, |v| v;
    crate::HintPlacement => Option<crate::HintPlacement>, |v| Some(v);
    Option<crate::HintPlacement> => Option<crate::HintPlacement>, |v| v;
    nana_ui_core::ButtonKind => nana_ui_core::ButtonKind, |v| v;
    crate::ListItemRole => crate::ListItemRole, |v| v;
    crate::RangeSpanOrientation => crate::RangeSpanOrientation, |v| v;
    Option<f64> => Option<f64>, |v| v;
    f64 => Option<f64>, |v| Some(v);
    Option<f32> => Option<f32>, |v| v;
    f32 => Option<f32>, |v| Some(v);
    // An unsuffixed literal settles on `f64`: `.thickness(3.0)`.
    f64 => Option<f32>, |v| Some(v as f32);
    (f64, f64) => (f64, f64), |v| v;
    Option<(f64, f64)> => Option<(f64, f64)>, |v| v;
    (f64, f64) => Option<(f64, f64)>, |v| Some(v);
    Vec<f64> => Vec<f64>, |v| v;
    Vec<Arc<str>> => Vec<Arc<str>>, |v| v;
    Option<(u32, u32)> => Option<(u32, u32)>, |v| v;
    (u32, u32) => Option<(u32, u32)>, |v| Some(v);
    [f32; 4] => [f32; 4], |v| v;
    Vec<nana_ui_core::TreeNode<Arc<str>>> => Vec<nana_ui_core::TreeNode<Arc<str>>>, |v| v;
    crate::LocalizedText => crate::LocalizedText, |v| v;
    crate::Locale => Option<crate::Locale>, |v| Some(v);
    Option<crate::Locale> => Option<crate::Locale>, |v| v;
    // A language tag: `.locale("ar")`. An empty one names no locale.
    &'static str => Option<crate::Locale>, |v| crate::Locale::parse(v);
}

/// Any `String` control prop (`Button::label`, `Checkbox::label`,
/// `Switch::label`, `ListItem::label`, …) accepts [`crate::LocalizedText`]
/// directly and updates automatically when the world's locale or catalog
/// changes.
impl IntoProp<String> for crate::LocalizedText {
    fn bind_field<C: 'static, W: FieldWrite<C, String> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        let resolver = super::controls::locale_resolver();
        (move || resolver.get().resolve(&self)).bind_field::<C, W>(target, bindings, site);
    }

    fn into_source(self) -> PropSource<String> {
        let resolver = super::controls::locale_resolver();
        PropSource::Dyn(Box::new(move || resolver.get().resolve(&self)))
    }
}

/// Any `Arc<str>` control prop (`TextInput::placeholder`, `IconButton::label`,
/// `StatusBadge::label`, `EmptyState::title`, `ActionMenu::label`, …) accepts
/// [`crate::LocalizedText`] directly and updates on locale or catalog switches.
impl IntoProp<Arc<str>> for crate::LocalizedText {
    fn bind_field<C: 'static, W: FieldWrite<C, Arc<str>> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        let resolver = super::controls::locale_resolver();
        (move || Arc::<str>::from(resolver.get().resolve(&self)))
            .bind_field::<C, W>(target, bindings, site);
    }

    fn into_source(self) -> PropSource<Arc<str>> {
        let resolver = super::controls::locale_resolver();
        PropSource::Dyn(Box::new(move || {
            Arc::<str>::from(resolver.get().resolve(&self))
        }))
    }
}

/// Any `Option<Arc<str>>` control prop (`Select::placeholder`,
/// `Slider::label`, `SettingsRow::hint`, `TextInput::label`, …) accepts
/// [`crate::LocalizedText`] directly and updates on locale or catalog switches.
impl IntoProp<Option<Arc<str>>> for crate::LocalizedText {
    fn bind_field<C: 'static, W: FieldWrite<C, Option<Arc<str>>> + 'static>(
        self,
        target: &mut C,
        bindings: &mut NodeBindings<C>,
        site: &'static Location<'static>,
    ) {
        let resolver = super::controls::locale_resolver();
        (move || {
            let text = resolver.get().resolve(&self);
            (!text.is_empty()).then(|| Arc::from(text))
        })
        .bind_field::<C, W>(target, bindings, site);
    }

    fn into_source(self) -> PropSource<Option<Arc<str>>> {
        let resolver = super::controls::locale_resolver();
        PropSource::Dyn(Box::new(move || {
            let text = resolver.get().resolve(&self);
            (!text.is_empty()).then(|| Arc::from(text))
        }))
    }
}
