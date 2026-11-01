//! Momentum scrolling for the desktop app on Linux. A touchpad there only
//! reports the fingers' movement, with no glide after they lift (browsers
//! and GTK add their own; egui doesn't), so a long panel takes many
//! swipes. This scales touchpad scrolling up and, once the fingers stop,
//! keeps it going and slows it down smoothly. A mouse wheel's notches
//! don't glide.

use eframe::egui::{self, Event, MouseWheelUnit, Vec2};

/// How much further a touchpad swipe moves things.
const BOOST: f32 = 2.0;
/// How long without touchpad input before the glide starts, in seconds.
const LIFT: f64 = 0.05;
/// How fast a glide slows: its speed halves about every 0.15 s.
const FRICTION: f32 = 4.6;
/// Glides slower than this (points per second) stop.
const STOP: f32 = 40.0;
/// egui's points per scroll line on the desktop.
const LINE: f32 = 40.0;

#[derive(Default)]
pub struct Glide {
    /// The scroll's speed, in points per second.
    velocity: Vec2,
    /// When the last touchpad scroll came in.
    last_input: f64,
    /// The time of the last frame.
    last_frame: Option<f64>,
}

impl Glide {
    /// Rewrites one frame's input: boosts touchpad scrolling and adds the
    /// glide's own scroll when the fingers have lifted. Returns whether
    /// it's still gliding (so the app keeps redrawing).
    pub fn apply(&mut self, input: &mut egui::RawInput) -> bool {
        let Some(now) = input.time else {
            return false;
        };
        let dt = self
            .last_frame
            .map_or(0.0, |t| (now - t).clamp(0.0, 0.05) as f32);
        self.last_frame = Some(now);
        let mut moved = Vec2::ZERO;
        let mut touchpad = false;
        let mut stop = false;
        for event in &mut input.events {
            match event {
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                } if !(modifiers.ctrl || modifiers.command) => {
                    // A wheel's notches come as whole lines; a touchpad's
                    // movement as points or fractions of a line.
                    let smooth = match unit {
                        MouseWheelUnit::Point => true,
                        MouseWheelUnit::Line => delta.x.fract() != 0.0 || delta.y.fract() != 0.0,
                        MouseWheelUnit::Page => false,
                    };
                    if smooth {
                        *delta *= BOOST;
                        touchpad = true;
                        moved += match unit {
                            MouseWheelUnit::Line => *delta * LINE,
                            _ => *delta,
                        };
                    } else {
                        stop = true;
                    }
                }
                Event::PointerButton { pressed: true, .. } | Event::Key { pressed: true, .. } => {
                    stop = true;
                }
                _ => {}
            }
        }
        if stop {
            self.velocity = Vec2::ZERO;
            return false;
        }
        if touchpad {
            // Follow the fingers' speed, smoothed over a few frames.
            if dt > 0.0 {
                self.velocity = self.velocity * 0.5 + (moved / dt) * 0.5;
            }
            self.last_input = now;
            return false;
        }
        if now - self.last_input < LIFT || self.velocity.length() < STOP {
            if now - self.last_input >= LIFT {
                self.velocity = Vec2::ZERO;
            }
            return false;
        }
        // Gliding: scroll on by the speed, and slow it down.
        input.events.push(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: self.velocity * dt,
            modifiers: Default::default(),
        });
        self.velocity *= (-FRICTION * dt).exp();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(glide: &mut Glide, time: f64, scroll: Option<Event>) -> (bool, Vec<Event>) {
        let mut input = egui::RawInput {
            time: Some(time),
            events: scroll.into_iter().collect(),
            ..Default::default()
        };
        let gliding = glide.apply(&mut input);
        (gliding, input.events)
    }

    fn swipe(y: f32) -> Option<Event> {
        Some(Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: Vec2::new(0.0, y),
            modifiers: Default::default(),
        })
    }

    #[test]
    fn a_swipe_keeps_gliding_after_the_fingers_lift_then_stops() {
        let mut g = Glide::default();
        let mut t = 0.0;
        for _ in 0..5 {
            t += 0.016;
            frame(&mut g, t, swipe(-20.0));
        }
        // Fingers lift: after a moment it glides on, the same way.
        t += 0.1;
        let (gliding, events) = frame(&mut g, t, None);
        assert!(gliding);
        assert!(matches!(
            events.last(),
            Some(Event::MouseWheel { delta, .. }) if delta.y < 0.0
        ));
        // And comes to rest.
        for _ in 0..200 {
            t += 0.016;
            frame(&mut g, t, None);
        }
        assert!(!frame(&mut g, t + 0.016, None).0);
    }

    #[test]
    fn a_click_stops_the_glide() {
        let mut g = Glide::default();
        let mut t = 0.0;
        for _ in 0..5 {
            t += 0.016;
            frame(&mut g, t, swipe(-20.0));
        }
        t += 0.1;
        assert!(frame(&mut g, t, None).0);
        let click = Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        };
        assert!(!frame(&mut g, t + 0.016, Some(click)).0);
        assert!(!frame(&mut g, t + 0.032, None).0);
    }

    #[test]
    fn a_mouse_wheel_notch_does_not_glide() {
        let mut g = Glide::default();
        let notch = Some(Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: Vec2::new(0.0, -1.0),
            modifiers: Default::default(),
        });
        frame(&mut g, 0.016, notch);
        assert!(!frame(&mut g, 0.2, None).0);
    }
}
