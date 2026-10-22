//! Renders SignalRGB HTML effects in a headless Chrome and publishes downsampled frames.
//!
//! Chrome is driven over the DevTools protocol on an inherited pipe (fds 3/4), so no port is opened.
//! One page runs per distinct (effect, options) that something is showing; Chrome itself is started on demand
//! and closed again when no canvas effect is in use.
use crate::effects::EffectInfo;
use crate::fx::{Frame, Fx, Params};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const W: usize = 80;
const H: usize = 50;
const FPS: u64 = 30;
const IDLE_CLOSE: Duration = Duration::from_secs(20);

/// Runs before the effect's own script: declares its controls as globals, stubs the host `engine`,
/// and captures the canvas after every animation frame (WebGL buffers are only readable in that same task).
const INJECT: &str = r#"(() => {
  const props = __PROPS__;
  Object.assign(window, props);
  window.engine = window.engine || { audio: { level: 0, density: 0, freq: new Array(200).fill(0) }, zone: {} };
  const W = 80, H = 50;
  const off = document.createElement('canvas'); off.width = W; off.height = H;
  const octx = off.getContext('2d', { willReadFrequently: true });
  let px = null;
  const raf = window.requestAnimationFrame.bind(window);
  // Pages draw at the display rate (60 Hz); we only sample ~30 Hz, so run their loops at that pace.
  let frameT = -1, run = true, lastRun = 0;
  window.requestAnimationFrame = (cb) => raf((t) => {
    if (t !== frameT) { frameT = t; run = t - lastRun >= 28; if (run) lastRun = t; } // decide once per frame
    if (!run) { window.requestAnimationFrame(cb); return; }
    cb(t);
    try {
      const c = document.getElementById('exCanvas');
      if (c) { octx.fillStyle = '#000'; octx.fillRect(0, 0, W, H); octx.drawImage(c, 0, 0, W, H); px = octx.getImageData(0, 0, W, H).data; }
    } catch (e) {}
  });
  window.__rf_call = (name) => { const f = window['on' + name + 'Changed']; if (typeof f === 'function') { try { f(); } catch (e) {} } };
  window.__rf_set = (name, v) => { window[name] = v; window.__rf_call(name); };
  window.__rf_grab = () => {
    if (!px) return '';
    const rgb = new Uint8Array(W * H * 3);
    for (let i = 0, j = 0; i < px.length; i += 4) { rgb[j++] = px[i]; rgb[j++] = px[i + 1]; rgb[j++] = px[i + 2]; }
    let bin = '';
    for (let i = 0; i < rgb.length; i += 8192) bin += String.fromCharCode.apply(null, rgb.subarray(i, i + 8192));
    return btoa(bin);
  };
  window.addEventListener('load', () => { for (const k in props) window.__rf_call(k); });
})();"#;

pub fn find_chrome() -> Option<PathBuf> {
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find_map(|n| {
        std::env::var_os("PATH")?.to_str()?.split(':').map(|d| Path::new(d).join(n)).find(|p| p.is_file())
    })
}

struct Browser {
    child: Child,
    tx: Mutex<File>,
    waiting: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    next: AtomicU64,
    alive: Arc<AtomicBool>,
    profile: PathBuf,
}

impl Browser {
    fn launch(chrome: &Path) -> Result<Self> {
        let profile = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or("/tmp".into())).join(format!("refract-chrome-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);

        // Parent keeps (to_w, from_r); the child sees (to_r, from_w) as fds 3 and 4.
        let (mut to, mut from) = ([0i32; 2], [0i32; 2]);
        unsafe {
            if libc::pipe2(to.as_mut_ptr(), libc::O_CLOEXEC) < 0 || libc::pipe2(from.as_mut_ptr(), libc::O_CLOEXEC) < 0 {
                bail!("pipe: {}", std::io::Error::last_os_error());
            }
        }
        // Park the child's ends on high fds so dup2 onto 3 and 4 cannot clobber one another.
        let hi = |fd: i32| unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        let (child_in, child_out) = (hi(to[0]), hi(from[1]));
        if child_in < 0 || child_out < 0 { bail!("fcntl: {}", std::io::Error::last_os_error()); }
        unsafe { libc::close(to[0]); libc::close(from[1]); }
        let mut cmd = Command::new(chrome);
        cmd.args([
            "--headless=new", "--remote-debugging-pipe", "--no-first-run", "--no-default-browser-check",
            "--disable-extensions", "--disable-sync", "--disable-component-update", "--disable-background-networking",
            "--mute-audio", "--window-size=400,300",
            "--disable-background-timer-throttling", "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
            // 2-D canvas rasterises on the CPU without a GPU process; one renderer shared by all pages.
            "--disable-gpu", "--disable-software-rasterizer", "--renderer-process-limit=1",
            "--disable-features=IsolateOrigins,site-per-process,Translate,MediaRouter,OptimizationHints",
            "--disable-breakpad", "--disable-crash-reporter",
            // Effects are third-party scripts: let them draw, but not reach the network.
            "--host-resolver-rules=MAP * ~NOTFOUND",
        ]).args(std::env::var("REFRACT_CHROME_FLAGS").unwrap_or_default().split_whitespace())
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank").stdin(Stdio::null()).stdout(Stdio::null())
            .stderr(if std::env::var_os("REFRACT_DEBUG").is_some() { Stdio::inherit() } else { Stdio::null() });
        unsafe {
            cmd.pre_exec(move || {
                // Die with the daemon rather than lingering as an orphan.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                if libc::dup2(child_in, 3) < 0 || libc::dup2(child_out, 4) < 0 { return Err(std::io::Error::last_os_error()); }
                Ok(())
            });
        }
        let child = cmd.spawn().with_context(|| format!("start {}", chrome.display()))?;
        unsafe { libc::close(child_in); libc::close(child_out); }
        let (tx, mut rx) = unsafe { (File::from_raw_fd(to[1]), File::from_raw_fd(from[0])) };

        let waiting: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>> = Default::default();
        let alive = Arc::new(AtomicBool::new(true));
        let (w2, a2) = (waiting.clone(), alive.clone());
        std::thread::spawn(move || {
            let (mut buf, mut chunk) = (Vec::new(), [0u8; 65536]);
            while let Ok(n) = rx.read(&mut chunk) {
                if n == 0 { break; }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(i) = buf.iter().position(|&b| b == 0) {
                    let msg: Vec<u8> = buf.drain(..=i).collect();
                    let Ok(v) = serde_json::from_slice::<Value>(&msg[..msg.len() - 1]) else { continue };
                    if let Some(id) = v.get("id").and_then(Value::as_u64) {
                        if let Some(s) = w2.lock().unwrap().remove(&id) { let _ = s.send(v); }
                    }
                }
            }
            a2.store(false, Ordering::Relaxed);
            w2.lock().unwrap().clear(); // wakes every waiting caller with an error
        });
        Ok(Self { child, tx: Mutex::new(tx), waiting, next: AtomicU64::new(1), alive, profile })
    }

    fn alive(&self) -> bool { self.alive.load(Ordering::Relaxed) }

    fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (s, r) = mpsc::channel();
        self.waiting.lock().unwrap().insert(id, s);
        let mut msg = json!({"id": id, "method": method, "params": params});
        if let Some(sid) = session { msg["sessionId"] = json!(sid); }
        let mut bytes = serde_json::to_vec(&msg)?;
        bytes.push(0);
        self.tx.lock().unwrap().write_all(&bytes).context("write to Chrome")?;
        let resp = r.recv_timeout(Duration::from_secs(10)).map_err(|_| anyhow!("{method}: no reply from Chrome"))?;
        if let Some(e) = resp.get("error") { bail!("{method}: {}", e.get("message").and_then(Value::as_str).unwrap_or("error")); }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

struct Tab { session: String, target: String, applied: Map<String, Value> }

fn file_url(p: &Path) -> String {
    let mut s = String::from("file://");
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) { s.push(b as char) } else { s.push_str(&format!("%{b:02X}")) }
    }
    s
}

/// Effect defaults overlaid with whatever the user set.
fn props(e: &EffectInfo, p: &Params) -> Map<String, Value> {
    let mut m = e.defaults();
    for (k, v) in &p.opts { if m.contains_key(k) { m.insert(k.clone(), v.clone()); } }
    m
}

fn open_tab(b: &Browser, e: &EffectInfo, p: &Params) -> Result<Tab> {
    let target = b.call("Target.createTarget", json!({"url": "about:blank"}), None)?["targetId"].as_str().context("targetId")?.to_string();
    let session = b.call("Target.attachToTarget", json!({"targetId": target, "flatten": true}), None)?["sessionId"].as_str().context("sessionId")?.to_string();
    let applied = props(e, p);
    b.call("Page.enable", json!({}), Some(&session))?;
    let src = INJECT.replace("__PROPS__", &Value::Object(applied.clone()).to_string());
    b.call("Page.addScriptToEvaluateOnNewDocument", json!({"source": src}), Some(&session))?;
    let nav = b.call("Page.navigate", json!({"url": file_url(&e.path)}), Some(&session))?;
    if let Some(err) = nav.get("errorText") { bail!("cannot load {}: {err}", e.path.display()); }
    Ok(Tab { session, target, applied })
}

fn grab(b: &Browser, t: &Tab) -> Result<Option<Frame>> {
    let r = b.call("Runtime.evaluate", json!({"expression": "window.__rf_grab && window.__rf_grab()", "returnByValue": true}), Some(&t.session))?;
    let Some(s) = r["result"]["value"].as_str().filter(|s| !s.is_empty()) else { return Ok(None) };
    let rgb = base64::engine::general_purpose::STANDARD.decode(s).context("frame data")?;
    Ok((rgb.len() == W * H * 3).then_some(Frame { w: W, h: H, rgb }))
}

/// Start the host. Chrome is launched lazily, the first time a canvas effect is shown.
pub fn start(fx: Arc<Fx>, effects: Arc<Vec<EffectInfo>>, chrome: PathBuf) {
    std::thread::spawn(move || loop {
        if let Err(e) = run(&fx, &effects, &chrome) { eprintln!("canvas host: {e:#}"); }
        fx.drop_frames_except(&HashMap::new());
        std::thread::sleep(Duration::from_secs(3));
    });
}

fn run(fx: &Fx, effects: &[EffectInfo], chrome: &Path) -> Result<()> {
    let mut browser: Option<Browser> = None;
    let mut tabs: HashMap<String, Tab> = HashMap::new();
    let mut idle_since = Instant::now();
    let period = Duration::from_millis(1000 / FPS);
    loop {
        let t0 = Instant::now();
        let demand = fx.canvas_demand();
        if demand.is_empty() {
            if browser.is_some() && idle_since.elapsed() > IDLE_CLOSE { browser = None; tabs.clear(); fx.drop_frames_except(&demand); }
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        idle_since = Instant::now();
        if browser.as_ref().is_some_and(|b| !b.alive()) { bail!("Chrome exited"); }
        let b = match &browser { Some(b) => b, None => browser.insert(Browser::launch(chrome)?) };

        // Close pages nothing shows any more, open pages for new demand.
        let stale: Vec<String> = tabs.keys().filter(|k| !demand.contains_key(*k)).cloned().collect();
        for k in stale { if let Some(t) = tabs.remove(&k) { let _ = b.call("Target.closeTarget", json!({"targetId": t.target}), None); } }
        for (key, p) in &demand {
            let Some(e) = effects.iter().find(|e| format!("canvas:{}", e.id) == p.effect) else { continue };
            if !tabs.contains_key(key) { tabs.insert(key.clone(), open_tab(b, e, p)?); }
        }
        fx.drop_frames_except(&demand);

        for (key, t) in tabs.iter_mut() {
            let p = &demand[key];
            let Some(e) = effects.iter().find(|e| format!("canvas:{}", e.id) == p.effect) else { continue };
            // Apply control changes made since the page loaded.
            let want = props(e, p);
            for (k, v) in &want {
                if t.applied.get(k) != Some(v) {
                    b.call("Runtime.evaluate", json!({"expression": format!("window.__rf_set({}, {})", json!(k), v)}), Some(&t.session))?;
                    t.applied.insert(k.clone(), v.clone());
                }
            }
            match grab(b, t)? {
                Some(f) => fx.set_frame(key, Arc::new(f)),
                None if std::env::var_os("REFRACT_DEBUG").is_some() => {
                    let r = b.call("Runtime.evaluate", json!({"expression": "location.href + ' | ' + document.readyState + ' | ' + typeof window.__rf_grab + ' | canvas=' + !!document.getElementById('exCanvas')", "returnByValue": true}), Some(&t.session))?;
                    eprintln!("canvas {key}: no frame yet: {}", r["result"]["value"]);
                }
                None => {}
            }
        }
        if let Some(rest) = period.checked_sub(t0.elapsed()) { std::thread::sleep(rest); }
    }
}
