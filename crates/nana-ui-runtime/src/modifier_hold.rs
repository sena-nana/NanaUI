//! Debounced modifier-key hold tracker for shortcut/key-hint overlays.
//!
//! Recognizes a deliberate hold of a single modifier key (such as `Control`,
//! `Alt`, `Shift`, or `Meta`) after a configurable delay while cancelling or
//! spoiling the gesture whenever another key is pressed, a pointer press or
//! wheel scroll occurs, or the window loses focus.

use std::time::Duration;

use nana_ui_input::{InputModifiers, InputPayload, KeyInput, KeyState, PointerPhase};

/// Which modifier key [`ModifierHoldTracker`] watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    Control,
    Alt,
    Shift,
    Meta,
}

impl ModifierKey {
    /// True if `modifiers` currently has this modifier pressed.
    #[inline]
    pub const fn is_held(self, modifiers: InputModifiers) -> bool {
        match self {
            Self::Control => modifiers.control,
            Self::Alt => modifiers.alt,
            Self::Shift => modifiers.shift,
            Self::Meta => modifiers.meta,
        }
    }

    /// True if any modifier other than `self` is currently pressed.
    #[inline]
    pub const fn any_other_held(self, modifiers: InputModifiers) -> bool {
        match self {
            Self::Control => modifiers.alt || modifiers.shift || modifiers.meta,
            Self::Alt => modifiers.control || modifiers.shift || modifiers.meta,
            Self::Shift => modifiers.control || modifiers.alt || modifiers.meta,
            Self::Meta => modifiers.control || modifiers.alt || modifiers.shift,
        }
    }

    /// True when `key` represents this modifier key (physical or logical name).
    pub fn matches_key(self, key: &KeyInput) -> bool {
        let physical = key.physical.0.as_ref();
        let logical = key.logical.0.as_ref();
        match self {
            Self::Control => {
                matches!(physical, "ControlLeft" | "ControlRight")
                    || logical.eq_ignore_ascii_case("Control")
                    || logical.eq_ignore_ascii_case("Ctrl")
            }
            Self::Alt => {
                matches!(physical, "AltLeft" | "AltRight")
                    || logical.eq_ignore_ascii_case("Alt")
                    || logical.eq_ignore_ascii_case("Option")
            }
            Self::Shift => {
                matches!(physical, "ShiftLeft" | "ShiftRight")
                    || logical.eq_ignore_ascii_case("Shift")
            }
            Self::Meta => {
                matches!(
                    physical,
                    "MetaLeft" | "MetaRight" | "SuperLeft" | "SuperRight" | "OSLeft" | "OSRight"
                ) || logical.eq_ignore_ascii_case("Meta")
                    || logical.eq_ignore_ascii_case("Super")
                    || logical.eq_ignore_ascii_case("Command")
            }
        }
    }
}

/// Configuration for [`ModifierHoldTracker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierHoldConfig {
    /// Modifier key that arms the hold gesture.
    pub key: ModifierKey,
    /// Minimum duration the modifier must remain held alone before activating.
    pub hold_delay: Duration,
    /// Whether pointer down / wheel events spoil an armed or active hold.
    pub spoil_on_pointer: bool,
}

impl ModifierHoldConfig {
    /// Create a hold configuration for `key` with an explicit `hold_delay`.
    pub const fn new(key: ModifierKey, hold_delay: Duration) -> Self {
        Self {
            key,
            hold_delay,
            spoil_on_pointer: true,
        }
    }

    /// Control whether pointer presses and wheel events spoil the hold.
    pub const fn spoil_on_pointer(mut self, spoil: bool) -> Self {
        self.spoil_on_pointer = spoil;
        self
    }
}

/// Lifecycle phase of a tracked modifier-key hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierHoldState {
    /// Target modifier is not pressed.
    Idle,
    /// Target modifier is held alone; waiting until `activate_at` to activate.
    Armed { activate_at: Duration },
    /// Hold threshold elapsed and overlay/hint mode is active.
    Active,
    /// Another key/pointer gesture occurred while held; stays spoiled until
    /// the modifier is released or the tracker is reset.
    Spoiled,
}

/// Edge transition emitted by [`ModifierHoldTracker::on_input`] or [`ModifierHoldTracker::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierHoldTransition {
    /// State did not cross the active boundary.
    None,
    /// Hold threshold was reached; show the hint/overlay.
    Activated,
    /// An active hold ended (key released, spoiled, focus lost, or reset).
    Deactivated,
}

/// Pure, clock-driven state machine for debounced modifier-key hold detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierHoldTracker {
    config: ModifierHoldConfig,
    state: ModifierHoldState,
}

impl ModifierHoldTracker {
    pub const fn new(config: ModifierHoldConfig) -> Self {
        Self {
            config,
            state: ModifierHoldState::Idle,
        }
    }

    pub const fn config(&self) -> ModifierHoldConfig {
        self.config
    }

    pub const fn state(&self) -> ModifierHoldState {
        self.state
    }

    pub const fn is_active(&self) -> bool {
        matches!(self.state, ModifierHoldState::Active)
    }

    /// Next monotonic timestamp when [`Self::tick`] should be invoked, if armed.
    pub const fn deadline(&self) -> Option<Duration> {
        match self.state {
            ModifierHoldState::Armed { activate_at } => Some(activate_at),
            ModifierHoldState::Idle | ModifierHoldState::Active | ModifierHoldState::Spoiled => {
                None
            }
        }
    }

    /// Reset to [`ModifierHoldState::Idle`], returning
    /// [`ModifierHoldTransition::Deactivated`] if currently active.
    pub fn reset(&mut self) -> ModifierHoldTransition {
        let was_active = self.is_active();
        self.state = ModifierHoldState::Idle;
        if was_active {
            ModifierHoldTransition::Deactivated
        } else {
            ModifierHoldTransition::None
        }
    }

    /// Advance the tracker clock to `now`. Activates if the armed deadline has
    /// been reached.
    pub fn tick(&mut self, now: Duration) -> ModifierHoldTransition {
        match self.state {
            ModifierHoldState::Armed { activate_at } if now >= activate_at => {
                self.state = ModifierHoldState::Active;
                ModifierHoldTransition::Activated
            }
            _ => ModifierHoldTransition::None,
        }
    }

    /// Feed a canonical [`InputPayload`] sampled at monotonic timestamp `now`.
    pub fn on_input(&mut self, payload: &InputPayload, now: Duration) -> ModifierHoldTransition {
        match payload {
            InputPayload::Focus { focused: false } => self.reset(),
            InputPayload::Key(key) => self.on_key(key, now),
            InputPayload::Pointer(pointer) if self.config.spoil_on_pointer => {
                if !self.config.key.is_held(pointer.modifiers)
                    && self.state != ModifierHoldState::Idle
                {
                    return self.reset();
                }
                if matches!(pointer.phase, PointerPhase::Down) {
                    self.spoil()
                } else {
                    self.tick(now)
                }
            }
            InputPayload::Wheel(wheel) if self.config.spoil_on_pointer => {
                if !self.config.key.is_held(wheel.modifiers)
                    && self.state != ModifierHoldState::Idle
                {
                    return self.reset();
                }
                self.spoil()
            }
            _ => self.tick(now),
        }
    }

    fn on_key(&mut self, key: &KeyInput, now: Duration) -> ModifierHoldTransition {
        let target = self.config.key;
        let is_target_key = target.matches_key(key);
        let other_modifier = target.any_other_held(key.modifiers);

        match key.state {
            KeyState::Pressed => {
                if is_target_key {
                    if other_modifier {
                        return self.spoil();
                    }
                    match self.state {
                        ModifierHoldState::Idle => {
                            if self.config.hold_delay.is_zero() {
                                self.state = ModifierHoldState::Active;
                                ModifierHoldTransition::Activated
                            } else {
                                self.state = ModifierHoldState::Armed {
                                    activate_at: now.saturating_add(self.config.hold_delay),
                                };
                                ModifierHoldTransition::None
                            }
                        }
                        ModifierHoldState::Armed { activate_at } => {
                            if now >= activate_at {
                                self.state = ModifierHoldState::Active;
                                ModifierHoldTransition::Activated
                            } else {
                                ModifierHoldTransition::None
                            }
                        }
                        ModifierHoldState::Active | ModifierHoldState::Spoiled => {
                            ModifierHoldTransition::None
                        }
                    }
                } else {
                    // Any non-target key press (e.g. Ctrl+C) spoils the hold.
                    if target.is_held(key.modifiers) || self.state != ModifierHoldState::Idle {
                        self.spoil()
                    } else {
                        ModifierHoldTransition::None
                    }
                }
            }
            KeyState::Released => {
                if is_target_key || !target.is_held(key.modifiers) {
                    self.reset()
                } else {
                    ModifierHoldTransition::None
                }
            }
        }
    }

    fn spoil(&mut self) -> ModifierHoldTransition {
        let was_active = self.is_active();
        if self.state != ModifierHoldState::Idle {
            self.state = ModifierHoldState::Spoiled;
        }
        if was_active {
            ModifierHoldTransition::Deactivated
        } else {
            ModifierHoldTransition::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_input::{PointerInput, WheelInput, WheelUnit};

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    fn ctrl_mods() -> InputModifiers {
        InputModifiers {
            control: true,
            ..Default::default()
        }
    }

    fn ctrl_down() -> InputPayload {
        InputPayload::Key(KeyInput::named(
            "ControlLeft",
            "Control",
            KeyState::Pressed,
            ctrl_mods(),
        ))
    }

    fn ctrl_up() -> InputPayload {
        InputPayload::Key(KeyInput::named(
            "ControlLeft",
            "Control",
            KeyState::Released,
            InputModifiers::default(),
        ))
    }

    #[test]
    fn arms_activates_after_delay_and_deactivates_on_release() {
        let mut tracker =
            ModifierHoldTracker::new(ModifierHoldConfig::new(ModifierKey::Control, ms(250)));
        assert_eq!(tracker.state(), ModifierHoldState::Idle);
        assert_eq!(tracker.deadline(), None);

        assert_eq!(
            tracker.on_input(&ctrl_down(), ms(100)),
            ModifierHoldTransition::None
        );
        assert_eq!(
            tracker.state(),
            ModifierHoldState::Armed {
                activate_at: ms(350),
            }
        );
        assert_eq!(tracker.deadline(), Some(ms(350)));
        assert_eq!(tracker.tick(ms(349)), ModifierHoldTransition::None);
        assert!(!tracker.is_active());

        assert_eq!(tracker.tick(ms(350)), ModifierHoldTransition::Activated);
        assert!(tracker.is_active());
        assert_eq!(tracker.deadline(), None);

        assert_eq!(
            tracker.on_input(&ctrl_up(), ms(500)),
            ModifierHoldTransition::Deactivated
        );
        assert_eq!(tracker.state(), ModifierHoldState::Idle);
    }

    #[test]
    fn concurrent_shortcut_key_spoils_armed_and_active_holds() {
        let mut tracker =
            ModifierHoldTracker::new(ModifierHoldConfig::new(ModifierKey::Control, ms(200)));
        tracker.on_input(&ctrl_down(), ms(0));
        let ctrl_c =
            InputPayload::Key(KeyInput::named("KeyC", "c", KeyState::Pressed, ctrl_mods()));
        assert_eq!(
            tracker.on_input(&ctrl_c, ms(50)),
            ModifierHoldTransition::None
        );
        assert_eq!(tracker.state(), ModifierHoldState::Spoiled);
        assert_eq!(tracker.tick(ms(500)), ModifierHoldTransition::None);
        assert!(!tracker.is_active());

        // Releasing Ctrl clears spoiled state so the next clean hold works.
        assert_eq!(
            tracker.on_input(&ctrl_up(), ms(600)),
            ModifierHoldTransition::None
        );
        assert_eq!(tracker.state(), ModifierHoldState::Idle);

        tracker.on_input(&ctrl_down(), ms(700));
        assert_eq!(tracker.tick(ms(900)), ModifierHoldTransition::Activated);
        assert_eq!(
            tracker.on_input(&ctrl_c, ms(950)),
            ModifierHoldTransition::Deactivated
        );
        assert_eq!(tracker.state(), ModifierHoldState::Spoiled);
    }

    #[test]
    fn pointer_press_wheel_and_focus_loss_end_active_hold() {
        let mut tracker =
            ModifierHoldTracker::new(ModifierHoldConfig::new(ModifierKey::Alt, Duration::ZERO));
        let alt_down = InputPayload::Key(KeyInput::named(
            "AltLeft",
            "Alt",
            KeyState::Pressed,
            InputModifiers {
                alt: true,
                ..Default::default()
            },
        ));
        assert_eq!(
            tracker.on_input(&alt_down, ms(10)),
            ModifierHoldTransition::Activated
        );
        let click = InputPayload::Pointer(PointerInput {
            modifiers: InputModifiers {
                alt: true,
                ..Default::default()
            },
            ..PointerInput::mouse(PointerPhase::Down, 20.0, 20.0)
        });
        assert_eq!(
            tracker.on_input(&click, ms(20)),
            ModifierHoldTransition::Deactivated
        );
        assert_eq!(tracker.state(), ModifierHoldState::Spoiled);

        // Focus loss resets to Idle cleanly.
        assert_eq!(
            tracker.on_input(&InputPayload::Focus { focused: false }, ms(30)),
            ModifierHoldTransition::None
        );
        assert_eq!(tracker.state(), ModifierHoldState::Idle);

        // Wheel also spoils an active hold.
        assert_eq!(
            tracker.on_input(&alt_down, ms(40)),
            ModifierHoldTransition::Activated
        );
        let wheel = InputPayload::Wheel(WheelInput {
            pointer_id: nana_ui_input::PointerId(1),
            x: 10.0,
            y: 10.0,
            delta_x: 0.0,
            delta_y: -24.0,
            unit: WheelUnit::Pixels,
            modifiers: InputModifiers {
                alt: true,
                ..Default::default()
            },
        });
        assert_eq!(
            tracker.on_input(&wheel, ms(50)),
            ModifierHoldTransition::Deactivated
        );
        assert_eq!(tracker.state(), ModifierHoldState::Spoiled);
    }
}
