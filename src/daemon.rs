//! The long-running app: engine + saved settings + local web UI + tray icon.
use crate::engine::Engine;
use crate::fx::{Fx, Params};
use crate::{tray, web};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const PORT: u16 = 7878;

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config { pub params: Params, pub plugins_dir: Option<String> }

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("refract-rgb/config.json")
}

fn load() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub struct App { pub fx: Arc<Fx>, pub engine: Arc<Engine>, dirty: AtomicBool, plugins_dir: Option<String> }

impl App {
    /// Change settings from any control surface; they are saved shortly after.
    pub fn update(&self, f: impl FnOnce(&mut Params)) {
        self.fx.set(f);
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn save(&self) -> Result<()> {
        let path = config_path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let cfg = Config { params: self.fx.params(), plugins_dir: self.plugins_dir.clone() };
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&cfg)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn quit(&self) -> ! {
        let _ = self.save();
        self.engine.shutdown();
        std::thread::sleep(Duration::from_millis(300)); // let device threads leave the last frame in place
        std::process::exit(0)
    }
}

pub fn run(plugins_override: Option<PathBuf>) -> Result<()> {
    let cfg = load();
    let plugins_dir = plugins_override.map(|p| p.display().to_string()).or(cfg.plugins_dir.clone())
        .or_else(|| Some("extracted/stage3/lib/net45/Signal-x64/Plugins".to_string()).filter(|p| std::path::Path::new(p).is_dir()));
    // Services start in an arbitrary directory, so store an absolute path.
    let plugins_dir = plugins_dir.map(|p| std::fs::canonicalize(&p).map(|c| c.display().to_string()).unwrap_or(p));
    let fx = Arc::new(Fx::new(cfg.params));
    let engine = Engine::start(fx.clone(), plugins_dir.clone().map(PathBuf::from));
    let app = Arc::new(App { fx, engine, dirty: AtomicBool::new(plugins_dir != cfg.plugins_dir), plugins_dir });
    if app.plugins_dir.is_none() { eprintln!("warning: no plugin directory; USB devices disabled (use --plugins DIR)"); }

    let a = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        if a.dirty.swap(false, Ordering::Relaxed) { if let Err(e) = a.save() { eprintln!("save failed: {e:#}"); } }
    });

    // The tray is optional: without a StatusNotifier host the web UI still works.
    if let Err(e) = tray::spawn(app.clone()) { eprintln!("tray unavailable: {e:#}"); }
    eprintln!("refract: controls at http://127.0.0.1:{PORT}/");
    web::serve(app, PORT).context("web server")
}
