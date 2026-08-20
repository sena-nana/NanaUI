//! Extensible JS host native-component factories mounted as ordinary `nana-*` Vue nodes.
//!
//! This is the **JS host 组件工厂表**, not the Runtime component ABI.
//! Layout, hit-testing, and Scene identity still go through
//! `nana_ui::runtime::ComponentRegistry` / `register_component`.
//! Register descriptors here only for JS props/events/commands (`Nana.components.call`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, RwLock};

use nana_js_engine::{HostValue, JsException};

use crate::{BridgeEvent, WidgetId, WidgetProps};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativePropType {
    Any,
    String,
    Number,
    Bool,
    Object,
    Array,
    Bytes,
}

impl NativePropType {
    fn accepts(self, value: &HostValue) -> bool {
        match self {
            Self::Any => true,
            Self::String => matches!(value, HostValue::String(_)),
            Self::Number => matches!(value, HostValue::Number(_) | HostValue::BigInt(_)),
            Self::Bool => matches!(value, HostValue::Bool(_)),
            Self::Object => matches!(value, HostValue::Object(_)),
            Self::Array => matches!(value, HostValue::Array(_)),
            Self::Bytes => matches!(value, HostValue::Bytes(_)),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NativePropSchema {
    properties: BTreeMap<String, NativePropRule>,
    allow_additional: bool,
}

#[derive(Debug, Clone, Copy)]
struct NativePropRule {
    kind: NativePropType,
    required: bool,
}

impl NativePropSchema {
    pub fn permissive() -> Self {
        Self {
            properties: BTreeMap::new(),
            allow_additional: true,
        }
    }

    pub fn property(mut self, name: impl Into<String>, kind: NativePropType) -> Self {
        self.properties.insert(
            normalize_name(&name.into()),
            NativePropRule {
                kind,
                required: false,
            },
        );
        self
    }

    pub fn required(mut self, name: impl Into<String>, kind: NativePropType) -> Self {
        self.properties.insert(
            normalize_name(&name.into()),
            NativePropRule {
                kind,
                required: true,
            },
        );
        self
    }

    pub fn allow_additional(mut self, allow: bool) -> Self {
        self.allow_additional = allow;
        self
    }

    fn validate(
        &self,
        component: &str,
        props: &BTreeMap<String, HostValue>,
    ) -> Result<(), JsException> {
        for (name, rule) in &self.properties {
            if rule.required && !props.contains_key(name) {
                return Err(component_error(
                    "NativeComponentPropError",
                    format!("component `{component}` requires prop `{name}`"),
                ));
            }
        }
        for (name, value) in props {
            let Some(rule) = self.properties.get(name) else {
                if self.allow_additional {
                    continue;
                }
                return Err(component_error(
                    "NativeComponentPropError",
                    format!("component `{component}` does not declare prop `{name}`"),
                ));
            };
            if !rule.kind.accepts(value) {
                return Err(component_error(
                    "NativeComponentPropError",
                    format!("component `{component}` received an invalid `{name}` prop"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct NativeComponentContext {
    pub id: WidgetId,
    pub props: BTreeMap<String, HostValue>,
    pub semantic: WidgetProps,
    events: Arc<BTreeSet<String>>,
}

impl NativeComponentContext {
    pub fn event(&self, name: &str, payload: HostValue) -> Result<BridgeEvent, JsException> {
        let name = normalize_name(name);
        if !self.events.contains(&name) {
            return Err(component_error(
                "NativeComponentEventError",
                format!("native component did not declare event `{name}`"),
            ));
        }
        Ok(BridgeEvent::Native {
            id: self.id,
            name,
            payload,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NativeComponentCommand {
    pub id: WidgetId,
    pub name: String,
    pub args: HostValue,
}

pub trait NativeComponentFactory: Send + Sync + 'static {
    fn command(&self, command: NativeComponentCommand) -> Result<HostValue, JsException> {
        Err(component_error(
            "NativeComponentCommandError",
            format!("native component command `{}` has no handler", command.name),
        ))
    }

    fn unmount(&self, _id: WidgetId) {}
}

pub struct NativeComponentDescriptor {
    name: String,
    factory: Arc<dyn NativeComponentFactory>,
    props: NativePropSchema,
    events: BTreeSet<String>,
    commands: BTreeSet<String>,
}

impl NativeComponentDescriptor {
    pub fn new(name: impl Into<String>, factory: impl NativeComponentFactory) -> Self {
        Self {
            name: normalize_component_name(&name.into()),
            factory: Arc::new(factory),
            props: NativePropSchema::permissive(),
            events: BTreeSet::new(),
            commands: BTreeSet::new(),
        }
    }

    pub fn props(mut self, schema: NativePropSchema) -> Self {
        self.props = schema;
        self
    }

    pub fn events<I, S>(mut self, events: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.events = events
            .into_iter()
            .map(Into::into)
            .map(|name| normalize_name(&name))
            .collect();
        self
    }

    pub fn commands<I, S>(mut self, commands: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.commands = commands
            .into_iter()
            .map(Into::into)
            .map(|name| normalize_name(&name))
            .collect();
        self
    }
}

impl std::fmt::Debug for NativeComponentDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeComponentDescriptor")
            .field("name", &self.name)
            .field("events", &self.events)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

/// JS host 组件工厂表：描述符、props 白名单与 `Nana.components.call` 命令。
///
/// 不扩展 `WidgetKind`，也不是 Runtime [`nana_ui_runtime::ComponentRegistry`] ABI。
/// 新控件若要进入布局 / 命中 / Scene，必须同时 `register_component`。
#[derive(Clone, Default)]
#[doc(alias = "JsHostComponentRegistry")]
pub struct NativeComponentRegistry {
    inner: Arc<RwLock<BTreeMap<String, Arc<NativeComponentDescriptor>>>>,
    errors: Arc<Mutex<VecDeque<NativeComponentFailure>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeComponentFailure {
    pub component: String,
    pub id: WidgetId,
    pub error: JsException,
}

impl std::fmt::Debug for NativeComponentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.names();
        f.debug_struct("NativeComponentRegistry")
            .field("components", &names)
            .finish()
    }
}

impl NativeComponentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, descriptor: NativeComponentDescriptor) -> Result<(), JsException> {
        if descriptor.name.is_empty() {
            return Err(component_error(
                "NativeComponentRegistrationError",
                "native component name must not be empty",
            ));
        }
        let mut components = self.inner.write().map_err(|_| {
            component_error(
                "NativeComponentRegistrationError",
                "native component registry poisoned",
            )
        })?;
        if components.contains_key(&descriptor.name) {
            return Err(component_error(
                "NativeComponentRegistrationError",
                format!(
                    "native component `{}` is already registered",
                    descriptor.name
                ),
            ));
        }
        components.insert(descriptor.name.clone(), Arc::new(descriptor));
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.resolve(name).is_some()
    }

    pub fn names(&self) -> Vec<String> {
        self.inner
            .read()
            .map(|components| components.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn drain_errors(&self) -> Vec<JsException> {
        self.errors
            .lock()
            .map(|mut errors| errors.drain(..).map(|failure| failure.error).collect())
            .unwrap_or_default()
    }

    pub fn drain_failures(&self) -> Vec<NativeComponentFailure> {
        self.errors
            .lock()
            .map(|mut errors| errors.drain(..).collect())
            .unwrap_or_default()
    }

    pub(crate) fn has_failures(&self) -> bool {
        self.errors
            .lock()
            .map(|errors| !errors.is_empty())
            .unwrap_or(false)
    }

    pub(crate) fn restore_failures(&self, failures: Vec<NativeComponentFailure>) {
        if failures.is_empty() {
            return;
        }
        if let Ok(mut errors) = self.errors.lock() {
            for failure in failures.into_iter().rev() {
                errors.push_front(failure);
            }
        }
    }

    pub(crate) fn report_error(&self, component: &str, id: WidgetId, error: JsException) {
        if let Ok(mut errors) = self.errors.lock() {
            const MAX_PENDING_FAILURES: usize = 256;
            if errors.len() == MAX_PENDING_FAILURES {
                errors.pop_front();
            }
            errors.push_back(NativeComponentFailure {
                component: normalize_component_name(component),
                id,
                error,
            });
        }
    }

    pub(crate) fn validate_props(
        &self,
        name: &str,
        props: &BTreeMap<String, HostValue>,
    ) -> Result<(), JsException> {
        let descriptor = self.require(name)?;
        descriptor.props.validate(&descriptor.name, props)
    }

    pub(crate) fn command(
        &self,
        component: &str,
        id: WidgetId,
        name: &str,
        args: HostValue,
    ) -> Result<HostValue, JsException> {
        let descriptor = self.require(component)?;
        let name = normalize_name(name);
        if !descriptor.commands.contains(&name) {
            return Err(component_error(
                "NativeComponentCommandError",
                format!("component `{component}` does not declare command `{name}`"),
            ));
        }
        catch_unwind(AssertUnwindSafe(|| {
            descriptor
                .factory
                .command(NativeComponentCommand { id, name, args })
        }))
        .map_err(|_| {
            component_error(
                "NativeComponentCommandError",
                format!("component `{component}` panicked while handling a command"),
            )
        })?
    }

    pub(crate) fn unmount(&self, component: &str, id: WidgetId) {
        if let Some(descriptor) = self.resolve(component) {
            let _ = catch_unwind(AssertUnwindSafe(|| descriptor.factory.unmount(id)));
        }
    }

    fn require(&self, name: &str) -> Result<Arc<NativeComponentDescriptor>, JsException> {
        self.resolve(name).ok_or_else(|| {
            component_error(
                "NativeComponentNotFoundError",
                format!(
                    "unknown native component `{}`",
                    normalize_component_name(name)
                ),
            )
        })
    }

    fn resolve(&self, name: &str) -> Option<Arc<NativeComponentDescriptor>> {
        let name = normalize_component_name(name);
        self.inner.read().ok()?.get(&name).cloned()
    }
}

pub(crate) fn normalize_component_name(name: &str) -> String {
    normalize_name(name.trim().strip_prefix("nana-").unwrap_or(name.trim()))
}

fn normalize_name(name: &str) -> String {
    name.trim().replace('_', "-").to_ascii_lowercase()
}

fn component_error(name: &str, message: impl Into<String>) -> JsException {
    JsException::new(message).with_name(name)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct ProbeFactory {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl NativeComponentFactory for ProbeFactory {
        fn command(&self, command: NativeComponentCommand) -> Result<HostValue, JsException> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("command:{}:{}", command.id, command.name));
            Ok(command.args)
        }

        fn unmount(&self, id: WidgetId) {
            self.calls.lock().unwrap().push(format!("unmount:{id}"));
        }
    }

    fn descriptor(calls: Arc<Mutex<Vec<String>>>) -> NativeComponentDescriptor {
        NativeComponentDescriptor::new("probe-view", ProbeFactory { calls })
            .props(
                NativePropSchema::default()
                    .required("model-id", NativePropType::String)
                    .property("paused", NativePropType::Bool),
            )
            .events(["ready"])
            .commands(["refresh"])
    }

    #[test]
    fn registry_validates_props_events_commands_and_lifecycle() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let registry = NativeComponentRegistry::new();
        registry.register(descriptor(Arc::clone(&calls))).unwrap();
        assert!(registry.register(descriptor(Arc::clone(&calls))).is_err());

        let props = BTreeMap::from([
            ("model-id".into(), HostValue::string("m1")),
            ("paused".into(), HostValue::Bool(false)),
        ]);
        registry.validate_props("probe-view", &props).unwrap();
        assert!(
            registry
                .validate_props(
                    "probe-view",
                    &BTreeMap::from([("model-id".into(), HostValue::Number(1.0))]),
                )
                .is_err()
        );

        assert_eq!(
            registry
                .command("probe-view", 7, "refresh", HostValue::string("ok"))
                .unwrap(),
            HostValue::string("ok")
        );
        assert!(
            registry
                .command("probe-view", 7, "missing", HostValue::Null)
                .is_err()
        );
        registry.unmount("probe-view", 7);
        assert_eq!(*calls.lock().unwrap(), ["command:7:refresh", "unmount:7"]);
    }
}
