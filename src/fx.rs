//! Effects: a colour as a function of position (u, v in 0..1) and phase, resolved per device and zone.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;
use std::sync::{Arc, Mutex};

/// Built-in effects: (id, label, uses colour 1, uses colour 2, uses speed).
pub const BUILTIN: &[(&str, &str, bool, bool, bool)] = &[
    ("solid", "Solid", true, false, false),
    ("breathing", "Breathing", true, false, true),
    ("rainbow", "Rainbow wave", false, false, true),
    ("cycle", "Colour cycle", false, false, true),
    ("gradient", "Gradient wave", true, true, true),
    ("fire", "Fire", false, false, true),
    ("sparkle", "Sparkle", true, true, true),
    ("scanner", "Scanner", true, false, true),
    ("comet", "Comet", true, true, true),
    ("aurora", "Aurora", true, true, true),
    ("plasma", "Plasma", false, false, true),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// A built-in id, or `canvas:<effect id>` for a SignalRGB HTML effect.
    pub effect: String,
    pub color: [u8; 3],
    pub color2: [u8; 3],
    pub speed: f32,
    pub brightness: f32,
    pub on: bool,
    /// Values for a canvas effect's own `<meta property>` controls.
    pub opts: BTreeMap<String, Value>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            effect: "rainbow".into(), color: [0x80, 0x00, 0xff], color2: [0x00, 0xc8, 0xff],
            speed: 1.0, brightness: 1.0, on: true, opts: BTreeMap::new(),
        }
    }
}

/// Pixels a canvas effect last drew, downsampled; shared with the browser host.
pub struct Frame { pub w: usize, pub h: usize, pub rgb: Vec<u8> }

impl Frame {
    fn sample(&self, u: f64, v: f64) -> [u8; 3] {
        let x = (u.clamp(0.0, 1.0) * (self.w - 1) as f64).round() as usize;
        let y = (v.clamp(0.0, 1.0) * (self.h - 1) as f64).round() as usize;
        let i = (y * self.w + x) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

/// Key a canvas page is shared under: identical effect + options reuse one browser tab.
pub fn canvas_key(p: &Params) -> String {
    format!("{}|{}", p.effect, serde_json::to_string(&p.opts).unwrap_or_default())
}

#[derive(Default)]
struct Inner {
    global: Params,
    /// Overrides keyed `device` or `device/zone`; absent means "follow the parent".
    over: BTreeMap<String, Params>,
    phase: HashMap<String, f64>,
    frames: HashMap<String, Arc<Frame>>,
}

/// Shared between the control surfaces (writers) and device threads (readers).
#[derive(Default)]
pub struct Fx { inner: Mutex<Inner> }

fn parent(key: &str) -> &str { key.rsplit_once('/').map(|(d, _)| d).unwrap_or("") }

impl Inner {
    /// Which override (or "" for global) governs this device and zone.
    fn owner(&self, dev: &str, zone: &str) -> String {
        if !zone.is_empty() {
            let k = format!("{dev}/{zone}");
            if self.over.contains_key(&k) { return k; }
        }
        if self.over.contains_key(dev) { return dev.to_string(); }
        String::new()
    }
    fn params_of(&self, key: &str) -> &Params { if key.is_empty() { &self.global } else { self.over.get(key).unwrap_or(&self.global) } }
}

impl Fx {
    pub fn new(global: Params, over: BTreeMap<String, Params>) -> Self {
        Self { inner: Mutex::new(Inner { global, over, ..Default::default() }) }
    }

    pub fn params(&self) -> Params { self.inner.lock().unwrap().global.clone() }
    pub fn overrides(&self) -> BTreeMap<String, Params> { self.inner.lock().unwrap().over.clone() }

    /// Edit the settings for `key` ("" = everything). A new override starts as a copy of what it inherits.
    pub fn edit(&self, key: &str, f: impl FnOnce(&mut Params)) {
        let mut i = self.inner.lock().unwrap();
        if key.is_empty() { return f(&mut i.global); }
        if !i.over.contains_key(key) {
            let base = i.over.get(parent(key)).unwrap_or(&i.global).clone();
            i.over.insert(key.to_string(), base);
        }
        f(i.over.get_mut(key).unwrap());
    }

    /// Make `key` follow its parent again (and drop overrides beneath it).
    pub fn reset(&self, key: &str) {
        let mut i = self.inner.lock().unwrap();
        let prefix = format!("{key}/");
        i.over.retain(|k, _| k != key && !k.starts_with(&prefix));
        i.phase.remove(key);
    }

    /// Advance animation time; each override keeps its own phase so speeds stay independent.
    pub fn tick(&self, dt: f64) {
        let mut i = self.inner.lock().unwrap();
        let keys: Vec<String> = std::iter::once(String::new()).chain(i.over.keys().cloned()).collect();
        for k in keys {
            let s = i.params_of(&k).speed as f64;
            *i.phase.entry(k).or_insert(0.0) += dt * s;
        }
    }

    pub fn color(&self, dev: &str, zone: &str, u: f64, v: f64) -> [u8; 3] {
        let i = self.inner.lock().unwrap();
        let k = i.owner(dev, zone);
        let p = i.params_of(&k);
        let phase = i.phase.get(&k).copied().unwrap_or(0.0);
        if p.effect.starts_with("canvas:") {
            let Some(f) = i.frames.get(&canvas_key(p)) else { return [0; 3] };
            if !p.on { return [0; 3]; }
            let c = f.sample(u, v);
            return c.map(|x| (x as f64 * p.brightness.clamp(0.0, 1.0) as f64).round() as u8);
        }
        render(p, u, v, phase)
    }

    /// Canvas pages that some target is currently showing, with the options they need.
    pub fn canvas_demand(&self) -> HashMap<String, Params> {
        let i = self.inner.lock().unwrap();
        std::iter::once(&i.global).chain(i.over.values())
            .filter(|p| p.on && p.effect.starts_with("canvas:"))
            .map(|p| (canvas_key(p), p.clone())).collect()
    }

    pub fn set_frame(&self, key: &str, f: Arc<Frame>) { self.inner.lock().unwrap().frames.insert(key.to_string(), f); }
    pub fn drop_frames_except(&self, keep: &HashMap<String, Params>) { self.inner.lock().unwrap().frames.retain(|k, _| keep.contains_key(k)); }
}

fn mix(a: [f64; 3], b: [f64; 3], k: f64) -> [f64; 3] { [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * k) }
fn scale(a: [f64; 3], k: f64) -> [f64; 3] { a.map(|x| x * k) }

fn hash(n: u32) -> f64 {
    let mut x = n.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15; x = x.wrapping_mul(0x85EB_CA6B); x ^= x >> 13;
    x as f64 / u32::MAX as f64
}
/// Smooth 1-D value noise in 0..1.
fn noise(x: f64) -> f64 {
    let (i, f) = (x.floor(), x - x.floor());
    let s = f * f * (3.0 - 2.0 * f);
    let (a, b) = (hash(i as i64 as u32), hash((i as i64 + 1) as u32));
    a + (b - a) * s
}
/// Triangle wave 0..1..0 with period 1.
fn tri(x: f64) -> f64 { 1.0 - (2.0 * x.rem_euclid(1.0) - 1.0).abs() }

pub fn render(p: &Params, u: f64, v: f64, phase: f64) -> [u8; 3] {
    if !p.on { return [0; 3]; }
    let (c1, c2) = (p.color.map(f64::from), p.color2.map(f64::from));
    let rgb = match p.effect.as_str() {
        "breathing" => scale(c1, (1.0 - (TAU * phase * 0.4).cos()) / 2.0),
        "rainbow" => hsv((u * 0.8 + v * 0.2 + phase * 0.15).rem_euclid(1.0), 1.0, 1.0),
        "cycle" => hsv((phase * 0.08).rem_euclid(1.0), 1.0, 1.0),
        "gradient" => mix(c1, c2, 0.5 + 0.5 * (TAU * (u - phase * 0.25)).sin()),
        "fire" => {
            let heat = (noise(u * 6.0 + phase * 3.0) * 0.6 + noise(u * 15.0 - phase * 5.0) * 0.4) * (1.1 - v * 0.5);
            let h = heat.clamp(0.0, 1.0).powf(1.4) * 1.25;
            [255.0 * (h * 2.0).min(1.0), 255.0 * (h * 2.0 - 0.7).clamp(0.0, 1.0) * 0.8, 255.0 * (h * 4.0 - 3.2).clamp(0.0, 1.0) * 0.5]
        }
        "sparkle" => {
            let cell = ((u * 23.99) as u32).wrapping_add(100 * (v * 7.99) as u32);
            let (slot, f) = ((phase * 4.0).floor(), (phase * 4.0).fract());
            let lit = hash(cell.wrapping_mul(7919).wrapping_add(slot as i64 as u32).wrapping_mul(104_729)) > 0.82;
            let k = if lit { (1.0 - f).powi(2) } else { 0.0 };
            mix(scale(c1, 0.12), c2, k)
        }
        "scanner" => {
            let d = u - tri(phase * 0.35);
            scale(c1, 0.04 + (-d * d * 80.0).exp())
        }
        "comet" => {
            let d = (phase * 0.3 - u).rem_euclid(1.0);
            scale(mix(c1, c2, (d * 2.0).min(1.0)), (-d * 5.0).exp())
        }
        "aurora" => {
            let t = 0.5 + 0.5 * (TAU * 0.8 * u + phase * 0.5 + 1.7 * (phase * 0.23).sin()).sin();
            scale(mix(c1, c2, t), 0.3 + 0.7 * noise(u * 3.0 - phase * 0.7))
        }
        "plasma" => {
            let h = 0.5 + 0.25 * (u * 7.0 + phase * 1.3).sin() + 0.25 * ((u + v) * 5.0 - phase * 0.9).sin();
            hsv(h.rem_euclid(1.0), 1.0, 0.7 + 0.3 * (u * 3.0 + phase).sin())
        }
        _ => c1, // "solid" and anything unknown
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
    fn p(effect: &str) -> Params { Params { effect: effect.into(), ..Default::default() } }

    #[test]
    fn solid_brightness_and_off() {
        let mut s = Params { color: [10, 20, 30], ..p("solid") };
        assert_eq!(render(&s, 0.0, 0.0, 5.0), [10, 20, 30]);
        s.brightness = 0.5;
        assert_eq!(render(&s, 0.0, 0.0, 0.0), [5, 10, 15]);
        s.on = false;
        assert_eq!(render(&s, 0.0, 0.0, 0.0), [0, 0, 0]);
    }

    #[test]
    fn rainbow_moves_with_phase() {
        let r = p("rainbow");
        assert_ne!(render(&r, 0.0, 0.0, 0.0), render(&r, 0.0, 0.0, 1.0));
        assert_eq!(render(&r, 0.0, 0.0, 0.0), [255, 0, 0]);
    }

    #[test]
    fn every_builtin_renders_without_panicking_and_is_not_always_black() {
        for (id, ..) in BUILTIN {
            let params = p(id);
            let mut lit = false;
            for i in 0..200 {
                for j in 0..8 {
                    let c = render(&params, j as f64 / 7.0, 0.5, i as f64 * 0.37);
                    lit |= c != [0, 0, 0];
                }
            }
            assert!(lit, "{id} never produced light");
        }
    }

    #[test]
    fn overrides_inherit_then_diverge_and_reset() {
        let fx = Fx::new(Params { effect: "solid".into(), color: [1, 2, 3], ..Default::default() }, BTreeMap::new());
        assert_eq!(fx.color("usb:a", "z", 0.0, 0.0), [1, 2, 3]);
        fx.edit("usb:a/z", |p| p.color = [9, 9, 9]);
        assert_eq!(fx.color("usb:a", "z", 0.0, 0.0), [9, 9, 9]);
        assert_eq!(fx.color("usb:a", "other", 0.0, 0.0), [1, 2, 3]); // sibling zone unaffected
        fx.edit("usb:a", |p| p.color = [7, 7, 7]);
        assert_eq!(fx.color("usb:a", "other", 0.0, 0.0), [7, 7, 7]); // device override applies to its zones
        assert_eq!(fx.color("usb:a", "z", 0.0, 0.0), [9, 9, 9]); // zone override still wins
        fx.reset("usb:a");
        assert_eq!(fx.color("usb:a", "z", 0.0, 0.0), [1, 2, 3]); // reset drops the zone override too
    }
}
