//! Effects: a colour as a function of position (u, v in 0..1) and phase.
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect { Solid, Breathing, Rainbow, Cycle, Gradient }

pub const EFFECTS: &[(Effect, &str)] = &[
    (Effect::Solid, "Solid"), (Effect::Breathing, "Breathing"), (Effect::Rainbow, "Rainbow wave"),
    (Effect::Cycle, "Colour cycle"), (Effect::Gradient, "Gradient wave"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub effect: Effect,
    pub color: [u8; 3],
    pub color2: [u8; 3],
    pub speed: f32,
    pub brightness: f32,
    pub on: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self { effect: Effect::Rainbow, color: [0x80, 0x00, 0xff], color2: [0x00, 0xc8, 0xff], speed: 1.0, brightness: 1.0, on: true }
    }
}

/// Shared between the control surfaces (writers) and device threads (readers).
#[derive(Default)]
pub struct Fx { params: Mutex<Params>, phase: Mutex<f64> }

impl Fx {
    pub fn new(p: Params) -> Self { Self { params: Mutex::new(p), phase: Mutex::new(0.0) } }
    pub fn params(&self) -> Params { self.params.lock().unwrap().clone() }
    pub fn set(&self, f: impl FnOnce(&mut Params)) { f(&mut self.params.lock().unwrap()); }
    /// Advance animation time; speed changes then alter rate without jumping phase.
    pub fn tick(&self, dt: f64) {
        let speed = self.params.lock().unwrap().speed as f64;
        *self.phase.lock().unwrap() += dt * speed;
    }
    pub fn color(&self, u: f64, v: f64) -> [u8; 3] {
        let p = self.params();
        let phase = *self.phase.lock().unwrap();
        render(&p, u, v, phase)
    }
}

pub fn render(p: &Params, u: f64, v: f64, phase: f64) -> [u8; 3] {
    if !p.on { return [0; 3]; }
    let c = |a: [u8; 3]| a.map(f64::from);
    let rgb = match p.effect {
        Effect::Solid => c(p.color),
        Effect::Breathing => {
            let k = (1.0 - (std::f64::consts::TAU * phase * 0.4).cos()) / 2.0;
            c(p.color).map(|x| x * k)
        }
        Effect::Rainbow => hsv((u * 0.8 + v * 0.2 + phase * 0.15).rem_euclid(1.0), 1.0, 1.0),
        Effect::Cycle => hsv((phase * 0.08).rem_euclid(1.0), 1.0, 1.0),
        Effect::Gradient => {
            let k = 0.5 + 0.5 * (std::f64::consts::TAU * (u - phase * 0.25)).sin();
            let (a, b) = (c(p.color), c(p.color2));
            [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * k)
        }
    };
    rgb.map(|x| (x * p.brightness.clamp(0.0, 1.0) as f64).round().clamp(0.0, 255.0) as u8)
}

fn hsv(h: f64, s: f64, v: f64) -> [f64; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
    [r * 255.0, g * 255.0, b * 255.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solid_and_off() {
        let mut p = Params { effect: Effect::Solid, color: [10, 20, 30], ..Default::default() };
        assert_eq!(render(&p, 0.0, 0.0, 5.0), [10, 20, 30]);
        p.brightness = 0.5;
        assert_eq!(render(&p, 0.0, 0.0, 0.0), [5, 10, 15]);
        p.on = false;
        assert_eq!(render(&p, 0.0, 0.0, 0.0), [0, 0, 0]);
    }
    #[test]
    fn rainbow_moves_with_phase() {
        let p = Params { effect: Effect::Rainbow, ..Default::default() };
        assert_ne!(render(&p, 0.0, 0.0, 0.0), render(&p, 0.0, 0.0, 1.0));
        assert_eq!(render(&p, 0.0, 0.0, 0.0), [255, 0, 0]);
    }
}
