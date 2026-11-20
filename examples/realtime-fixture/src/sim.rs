//! Application-owned realtime state.
//!
//! Nothing in this module names a NanaUI type. The clock, the input axes and
//! the bodies are what a game-like application keeps for itself; NanaUI only
//! wakes the program at the deadline [`FixedClock::deadline`] asks for and
//! hands it raw key events.

use std::time::{Duration, Instant};

/// Rate of the application's fixed update.
pub const FIXED_HZ: u32 = 60;

/// Fixed steps one wake may run before the clock gives the backlog up. A
/// stall (a debugger, a suspended laptop) costs at most this many steps of
/// catch-up instead of a spiral of them.
pub const MAX_CATCH_UP: u32 = 4;

/// What one [`FixedClock::advance`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Advance {
    /// Fixed steps to run now.
    pub steps: u32,
    /// Due steps skipped because they exceeded [`MAX_CATCH_UP`].
    pub dropped: u64,
}

/// A fixed-step deadline the application owns.
///
/// Stopped, it has no deadline, so a paused or hidden application asks the
/// host for no wakeups at all.
#[derive(Debug, Clone)]
pub struct FixedClock {
    step: Duration,
    next: Option<Instant>,
}

impl FixedClock {
    pub fn new(hz: u32) -> Self {
        Self {
            step: Duration::from_secs(1) / hz.max(1),
            next: None,
        }
    }

    pub fn step(&self) -> Duration {
        self.step
    }

    /// Start (or restart) the cadence one step after `now`. A resumed clock
    /// does not owe the steps it slept through.
    pub fn start(&mut self, now: Instant) {
        self.next = Some(now + self.step);
    }

    pub fn stop(&mut self) {
        self.next = None;
    }

    pub fn running(&self) -> bool {
        self.next.is_some()
    }

    /// When the next fixed step is due.
    pub fn deadline(&self) -> Option<Instant> {
        self.next
    }

    /// Count the steps due at `now` and move the deadline past them, keeping
    /// phase. More than [`MAX_CATCH_UP`] due steps are dropped and the clock
    /// re-phases on `now`.
    pub fn advance(&mut self, now: Instant) -> Advance {
        let Some(mut next) = self.next else {
            return Advance::default();
        };
        let mut advance = Advance::default();
        while next <= now && advance.steps < MAX_CATCH_UP {
            advance.steps += 1;
            next += self.step;
        }
        if next <= now {
            let behind = now.duration_since(next).as_nanos() / self.step.as_nanos().max(1);
            advance.dropped = u64::try_from(behind).unwrap_or(u64::MAX).saturating_add(1);
            next = now + self.step;
        }
        self.next = Some(next);
        advance
    }
}

/// The application's own continuous input: which direction keys are held.
///
/// Built straight from physical key codes, so it does not depend on focus,
/// on a control consuming the key, or on any framework action map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputAxes {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

impl InputAxes {
    /// Apply one physical key edge. Returns whether the key is one of ours.
    pub fn apply_key(&mut self, physical: &str, pressed: bool) -> bool {
        let held = match physical {
            "KeyA" | "ArrowLeft" => &mut self.left,
            "KeyD" | "ArrowRight" => &mut self.right,
            "KeyW" | "ArrowUp" => &mut self.up,
            "KeyS" | "ArrowDown" => &mut self.down,
            _ => return false,
        };
        *held = pressed;
        true
    }

    /// Release everything, as when the window loses focus and the key-up
    /// events will go elsewhere.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// `MoveX`: -1 left, +1 right.
    pub fn move_x(&self) -> f32 {
        f32::from(i8::from(self.right) - i8::from(self.left))
    }

    /// `MoveY`: -1 up, +1 down (screen space).
    pub fn move_y(&self) -> f32 {
        f32::from(i8::from(self.down) - i8::from(self.up))
    }
}

/// One moving object, in playfield units: `0..1` across each axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub position: [f32; 2],
    pub velocity: [f32; 2],
}

/// Speed of the input-driven player, in playfield units per second.
pub const PLAYER_SPEED: f32 = 0.45;

/// The application's object model. NanaUI never sees it; the renderer reads
/// a flattened copy ([`Self::write_instances`]).
#[derive(Debug, Clone)]
pub struct World {
    bodies: Vec<Body>,
    player: Body,
    ticks: u64,
}

impl World {
    /// `count` drifting bodies from a fixed seed, plus the player.
    pub fn new(count: usize) -> Self {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let bodies = (0..count)
            .map(|_| {
                let angle = next() * std::f32::consts::TAU;
                let speed = 0.08 + next() * 0.22;
                Body {
                    position: [0.05 + next() * 0.9, 0.05 + next() * 0.9],
                    velocity: [angle.cos() * speed, angle.sin() * speed],
                }
            })
            .collect();
        Self {
            bodies,
            player: Body {
                position: [0.5, 0.5],
                velocity: [0.0, 0.0],
            },
            ticks: 0,
        }
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    pub fn player(&self) -> Body {
        self.player
    }

    /// Fixed steps run so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// One fixed step: integrate, bounce off the playfield edges, steer the
    /// player from the application's own axes.
    pub fn fixed_update(&mut self, dt: f32, input: &InputAxes) {
        for body in &mut self.bodies {
            integrate(body, dt);
        }
        self.player.velocity = [input.move_x() * PLAYER_SPEED, input.move_y() * PLAYER_SPEED];
        integrate(&mut self.player, dt);
        self.ticks += 1;
    }

    /// Flatten into the renderer's instance layout. The player is last, so it
    /// draws on top of the drifting bodies.
    pub fn write_instances(&self, out: &mut Vec<Instance>) {
        out.clear();
        out.extend(self.bodies.iter().enumerate().map(|(index, body)| {
            let hue = index as f32 * 0.618_034;
            Instance {
                center: body.position,
                radius: 0.018,
                color: drift_color(hue.fract()),
            }
        }));
        out.push(Instance {
            center: self.player.position,
            radius: 0.032,
            color: [1.0, 0.82, 0.25, 1.0],
        });
    }
}

fn integrate(body: &mut Body, dt: f32) {
    for axis in 0..2 {
        let mut position = body.position[axis] + body.velocity[axis] * dt;
        if position < 0.0 {
            position = -position;
            body.velocity[axis] = body.velocity[axis].abs();
        } else if position > 1.0 {
            position = 2.0 - position;
            body.velocity[axis] = -body.velocity[axis].abs();
        }
        body.position[axis] = position.clamp(0.0, 1.0);
    }
}

/// A cool, linear-light tint for a drifting body.
fn drift_color(t: f32) -> [f32; 4] {
    [0.10 + 0.25 * t, 0.35 + 0.35 * (1.0 - t), 0.85, 0.9]
}

/// The renderer's per-object record: centre in playfield units, radius as a
/// fraction of the playfield's shorter side, straight linear RGBA.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub center: [f32; 2],
    pub radius: f32,
    pub color: [f32; 4],
}

impl Instance {
    /// Bytes of one instance in the GPU vertex buffer.
    pub const STRIDE: usize = 32;

    pub fn write_bytes(&self, out: &mut Vec<u8>) {
        let words = [
            self.center[0],
            self.center[1],
            self.radius,
            0.0,
            self.color[0],
            self.color[1],
            self.color[2],
            self.color[3],
        ];
        for word in words {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_keeps_phase_and_bounds_catch_up() {
        let start = Instant::now();
        let mut clock = FixedClock::new(FIXED_HZ);
        assert_eq!(clock.advance(start), Advance::default());
        clock.start(start);
        let step = clock.step();
        assert_eq!(clock.advance(start).steps, 0);
        assert_eq!(clock.advance(start + step).steps, 1);
        // A late wake runs the steps it owes and keeps phase.
        assert_eq!(clock.advance(start + step * 3 + step / 2).steps, 2);
        assert_eq!(clock.deadline(), Some(start + step * 4));
        // A long stall drops the backlog beyond the catch-up bound.
        let stalled = clock.advance(start + step * 40);
        assert_eq!(stalled.steps, MAX_CATCH_UP);
        assert!(stalled.dropped > 30, "{stalled:?}");
        assert_eq!(clock.deadline(), Some(start + step * 41));
        clock.stop();
        assert_eq!(clock.deadline(), None);
    }

    #[test]
    fn axes_come_from_physical_keys_and_clear_on_focus_loss() {
        let mut axes = InputAxes::default();
        assert!(axes.apply_key("KeyD", true));
        assert!(axes.apply_key("ArrowUp", true));
        assert!(!axes.apply_key("KeyQ", true));
        assert_eq!((axes.move_x(), axes.move_y()), (1.0, -1.0));
        axes.apply_key("KeyA", true);
        assert_eq!(axes.move_x(), 0.0);
        axes.clear();
        assert_eq!((axes.move_x(), axes.move_y()), (0.0, 0.0));
    }

    #[test]
    fn bodies_stay_in_the_playfield() {
        let mut world = World::new(32);
        let axes = InputAxes::default();
        for _ in 0..600 {
            world.fixed_update(1.0 / 60.0, &axes);
        }
        assert_eq!(world.ticks(), 600);
        for body in world.bodies() {
            assert!(body.position.iter().all(|p| (0.0..=1.0).contains(p)));
        }
        let mut instances = Vec::new();
        world.write_instances(&mut instances);
        assert_eq!(instances.len(), 33);
        let mut bytes = Vec::new();
        instances[0].write_bytes(&mut bytes);
        assert_eq!(bytes.len(), Instance::STRIDE);
    }
}
