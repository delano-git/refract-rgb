//! Runs effects on hardware: a supervisor starts one thread per device and restarts any that die.
use crate::fx::Fx;
use crate::{ene, hid, host};
use anyhow::{Result, anyhow};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const USB_FPS: u32 = 30;
const DRAM_FPS: u32 = 15;

#[derive(Clone, Serialize)]
pub struct Status { pub name: String, pub kind: &'static str, pub state: String, pub detail: String }

pub struct Engine {
    pub fx: Arc<Fx>,
    pub status: Arc<Mutex<BTreeMap<String, Status>>>,
    stop: Arc<AtomicBool>,
}

struct Job { handle: JoinHandle<Result<()>>, nodes: Vec<String>, name: String, kind: &'static str }

struct Plugin { path: String, src: Arc<String>, info: host::PluginInfo }

impl Engine {
    pub fn start(fx: Arc<Fx>, plugins: Option<PathBuf>) -> Arc<Self> {
        let e = Arc::new(Self { fx, status: Default::default(), stop: Default::default() });
        let (a, b) = (e.clone(), e.clone());
        std::thread::spawn(move || a.ticker());
        std::thread::spawn(move || b.supervise(plugins));
        e
    }

    pub fn shutdown(&self) { self.stop.store(true, Ordering::Relaxed); }

    fn ticker(&self) {
        let mut last = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            let now = Instant::now();
            self.fx.tick(now.duration_since(last).as_secs_f64().min(0.25));
            last = now;
        }
    }

    fn set(&self, key: &str, name: &str, kind: &'static str, state: &str, detail: &str) {
        self.status.lock().unwrap().insert(key.into(), Status { name: name.into(), kind, state: state.into(), detail: detail.into() });
    }

    fn index(dir: &Path) -> Vec<Plugin> {
        fn walk(d: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x == "js") { out.push(p) }
            }
        }
        let mut files = Vec::new();
        walk(dir, &mut files);
        files.sort();
        files.into_iter().filter_map(|f| {
            let src = std::fs::read_to_string(&f).ok()?;
            let info = host::inspect(&src).ok()?;
            Some(Plugin { path: f.display().to_string(), src: Arc::new(src), info })
        }).collect()
    }

    fn supervise(self: Arc<Self>, plugins: Option<PathBuf>) {
        let index = plugins.map(|d| Self::index(&d)).unwrap_or_default();
        let mut jobs: HashMap<String, Job> = HashMap::new();
        let mut retry: HashMap<String, Instant> = HashMap::new();
        while !self.stop.load(Ordering::Relaxed) {
            // Reap threads that ended; they come back after a pause.
            let done: Vec<String> = jobs.iter().filter(|(_, j)| j.handle.is_finished()).map(|(k, _)| k.clone()).collect();
            for k in done {
                let j = jobs.remove(&k).unwrap();
                let res = j.handle.join().unwrap_or_else(|_| Err(anyhow!("thread panicked")));
                if self.stop.load(Ordering::Relaxed) { break; }
                let msg = res.err().map(|e| format!("{e:#}").lines().next().unwrap_or("").to_string()).unwrap_or_default();
                let wait = if msg.contains("no endpoint") { 120 } else { 10 };
                self.set(&k, &j.name, j.kind, if msg.contains("no endpoint") { "unsupported" } else { "error" }, &msg);
                retry.insert(k, Instant::now() + Duration::from_secs(wait));
            }

            let devices = hid::enumerate();
            let claimed: HashSet<&String> = jobs.values().flat_map(|j| j.nodes.iter()).collect();
            let mut spawn: Vec<(String, Job)> = Vec::new();
            let mut taken: HashSet<String> = HashSet::new();
            for p in &index {
                let key = format!("hid:{}", p.path);
                if jobs.contains_key(&key) || retry.get(&key).is_some_and(|t| *t > Instant::now()) { continue; }
                let cands: Vec<_> = devices.iter()
                    .filter(|d| p.info.vendor_id == Some(d.vendor_id) && p.info.product_ids.contains(&d.product_id))
                    .filter(|d| !claimed.contains(&d.node) && !taken.contains(&d.node))
                    .cloned().collect();
                if cands.is_empty() { continue; }
                taken.extend(cands.iter().map(|d| d.node.clone()));
                let (fx, stop, src) = (self.fx.clone(), self.stop.clone(), p.src.clone());
                let nodes = cands.iter().map(|d| d.node.clone()).collect();
                let c2 = cands.clone();
                let handle = std::thread::spawn(move || {
                    let o = host::RunOpts { live: true, fx, until: host::Until::Stop { stop, fps: USB_FPS }, hold: true };
                    host::run(&src, c2, o).map(|_| ())
                });
                self.set(&key, &p.info.name, "usb", "running", &cands[0].node);
                spawn.push((key, Job { handle, nodes, name: p.info.name.clone(), kind: "usb" }));
            }

            for (bus, addr) in ene::find_dram() {
                let key = format!("dram:{bus}:{addr:x}");
                if jobs.contains_key(&key) || retry.get(&key).is_some_and(|t| *t > Instant::now()) { continue; }
                let (fx, stop) = (self.fx.clone(), self.stop.clone());
                let handle = std::thread::spawn(move || dram_loop(bus, addr, fx, stop));
                let name = format!("DDR5 RGB ({addr:#04x})");
                self.set(&key, &name, "dram", "running", &format!("i2c-{bus}"));
                spawn.push((key, Job { handle, nodes: vec![], name, kind: "dram" }));
            }
            jobs.extend(spawn);

            for _ in 0..30 { if self.stop.load(Ordering::Relaxed) { break; } std::thread::sleep(Duration::from_millis(100)); }
        }
        for (_, j) in jobs { let _ = j.handle.join(); }
    }
}

fn dram_loop(bus: u32, addr: u16, fx: Arc<Fx>, stop: Arc<AtomicBool>) -> Result<()> {
    let mut d = ene::Dram::open(bus, addr)?;
    let period = Duration::from_secs_f64(1.0 / DRAM_FPS as f64);
    let (mut fails, mut last) = (0, Instant::now());
    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        if t0.duration_since(last) > Duration::from_secs(5) { d = ene::Dram::open(bus, addr)?; }
        last = t0;
        let n = d.leds;
        let colors: Vec<[u8; 3]> = (0..n).map(|i| fx.color(if n > 1 { i as f64 / (n - 1) as f64 } else { 0.5 }, 0.5)).collect();
        match d.frame(&colors) { Ok(()) => fails = 0, Err(e) => { fails += 1; if fails > 10 { return Err(e); } } }
        if let Some(rest) = period.checked_sub(t0.elapsed()) { std::thread::sleep(rest); }
    }
    Ok(())
}
