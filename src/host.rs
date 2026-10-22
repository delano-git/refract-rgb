//! QuickJS plugin host: exposes SignalRGB's `device` API and drives a plugin's lifecycle.
use crate::hid::{HidDevice, HidInfo};
use anyhow::{Result, anyhow};
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Context, Ctx, Function, Module, Runtime, Value, module::{Declared, Evaluated}};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const PRELUDE: &str = include_str!("js/prelude.js");
const MODULES: &str = include_str!("js/modules.js");

/// What the plugin is currently talking to.
#[derive(Default)]
pub struct Session {
    pub live: bool,
    pub candidates: Vec<HidInfo>,
    pub selected: Option<HidInfo>,
    pub dev: Option<HidDevice>,
    pub vid: u16,
    pub pid: u16,
    pub writes: usize,
    pub log: Vec<String>,
}
type Shared = Rc<RefCell<Session>>;

struct Shims(HashMap<String, String>);
impl Shims {
    fn load() -> Self {
        let mut m = HashMap::new();
        for chunk in MODULES.split("//@module ").skip(1) {
            let (name, body) = chunk.split_once('\n').unwrap();
            m.insert(name.trim().to_string(), body.to_string());
        }
        // `@SignalRGB/Errors` and `@SignalRGB/Errors.js` are used interchangeably by plugins.
        if let Some(e) = m.get("@SignalRGB/Errors.js").cloned() { m.insert("@SignalRGB/Errors".into(), e); }
        Shims(m)
    }
}
impl Resolver for Shims {
    fn resolve<'js>(&mut self, _: &Ctx<'js>, _base: &str, name: &str, _attrs: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        if self.0.contains_key(name) { Ok(name.to_string()) } else { Err(rquickjs::Error::new_resolving(_base, name)) }
    }
}
impl Loader for Shims {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _attrs: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, Declared>> {
        Module::declare(ctx.clone(), name, self.0[name].clone())
    }
}

fn bytes(v: Value<'_>, len: usize) -> Vec<u8> {
    let mut out: Vec<u8> = v.as_array()
        .map(|a| a.iter::<i32>().map(|x| x.unwrap_or(0) as u8).collect())
        .unwrap_or_default();
    out.truncate(len.max(0));
    out
}

fn install(ctx: &Ctx<'_>, s: &Shared) -> rquickjs::Result<()> {
    let g = ctx.globals();
    let c = s.clone();
    g.set("__hidWrite", Function::new(ctx.clone(), move |d: Value, len: usize| -> i32 {
        let buf = bytes(d, len);
        let mut s = c.borrow_mut();
        s.writes += 1;
        if s.live { if let Some(dev) = s.dev.as_mut() { return dev.write(&buf).map(|n| n as i32).unwrap_or(-1); } }
        buf.len() as i32
    }))?;
    let c = s.clone();
    g.set("__hidSendReport", Function::new(ctx.clone(), move |d: Value, len: usize| -> i32 {
        let buf = bytes(d, len);
        let mut s = c.borrow_mut();
        s.writes += 1;
        if s.live { if let Some(dev) = s.dev.as_mut() { return dev.send_feature(&buf).map(|n| n as i32).unwrap_or(-1); } }
        buf.len() as i32
    }))?;
    let c = s.clone();
    g.set("__hidGetReport", Function::new(ctx.clone(), move |d: Value, len: usize| -> Vec<i32> {
        let mut buf = bytes(d, len);
        buf.resize(len, 0);
        let mut s = c.borrow_mut();
        if s.live { if let Some(dev) = s.dev.as_mut() { let _ = dev.get_feature(&mut buf); } }
        buf.into_iter().map(i32::from).collect()
    }))?;
    let c = s.clone();
    g.set("__hidRead", Function::new(ctx.clone(), move |len: usize, timeout: i32| -> Vec<i32> {
        let mut s = c.borrow_mut();
        if s.live { if let Some(dev) = s.dev.as_mut() {
            return dev.read(len, timeout).unwrap_or_default().into_iter().map(i32::from).collect();
        } }
        vec![0; len]
    }))?;
    let c = s.clone();
    g.set("__hidSetEndpoint", Function::new(ctx.clone(), move |i: i32, u: u32, p: u32, _coll: i32| {
        let mut s = c.borrow_mut();
        let pick = s.candidates.iter()
            .find(|d| (d.interface == i || i < 0) && (u == 0 || d.usage == u) && (p == 0 || d.usage_page == p))
            .cloned();
        if let Some(info) = pick {
            if s.live { s.dev = HidDevice::open(&info.node).ok(); }
            s.selected = Some(info);
        }
    }))?;
    let c = s.clone();
    g.set("__hidEndpoints", Function::new(ctx.clone(), move || -> Vec<Vec<i32>> {
        c.borrow().candidates.iter().map(|d| vec![d.interface, d.usage as i32, d.usage_page as i32, 0]).collect()
    }))?;
    // Linux stand-in for Windows WMI: read SMBIOS strings from sysfs.
    g.set("__dmi", Function::new(ctx.clone(), |k: String| -> Option<String> {
        if !k.chars().all(|c| c.is_ascii_lowercase() || c == '_') { return None; }
        std::fs::read_to_string(format!("/sys/class/dmi/id/{k}")).ok().map(|v| v.trim().to_string())
    }))?;
    let c = s.clone();
    g.set("__hidProductId", Function::new(ctx.clone(), move || c.borrow().pid as i32))?;
    let c = s.clone();
    g.set("__hidVendorId", Function::new(ctx.clone(), move || c.borrow().vid as i32))?;
    g.set("__sleep", Function::new(ctx.clone(), |ms: u64| std::thread::sleep(std::time::Duration::from_millis(ms.min(2000)))))?;
    let c = s.clone();
    g.set("__log", Function::new(ctx.clone(), move |m: String| { eprintln!("[plugin] {m}"); c.borrow_mut().log.push(m); }))?;
    Ok(())
}

pub struct PluginInfo { pub name: String, pub vendor_id: Option<u16>, pub product_ids: Vec<u16> }

/// Load a plugin and read its static metadata (Name/VendorId/ProductId).
pub fn inspect(src: &str) -> Result<PluginInfo> {
    let s: Shared = Rc::new(RefCell::new(Session::default()));
    with_plugin(src, &s, |ctx, m| {
        let name: String = call(ctx, m, "Name")?;
        let vid: Option<u16> = call_opt(ctx, m, "VendorId");
        let pid_v: Value = m.get::<_, Function>("ProductId").and_then(|f| f.call(())).unwrap_or(Value::new_undefined(ctx.clone()));
        // ProductId() returns a number, or an array of numbers/strings ("0x7D75" or decimal).
        let parse = |v: Value| -> Option<u16> {
            if let Some(n) = v.as_int() { return Some(n as u16); }
            if let Some(f) = v.as_float() { return Some(f as u16); }
            let st = v.as_string()?.to_string().ok()?;
            match st.strip_prefix("0x").or_else(|| st.strip_prefix("0X")) {
                Some(h) => u16::from_str_radix(h, 16).ok(),
                None => st.parse::<u16>().ok().or_else(|| u16::from_str_radix(&st, 16).ok()),
            }
        };
        let pids = if let Some(a) = pid_v.as_array() { a.iter::<Value>().filter_map(|x| x.ok()).filter_map(parse).collect() }
                   else { parse(pid_v).into_iter().collect() };
        Ok(PluginInfo { name, vendor_id: vid, product_ids: pids })
    })
}

pub struct RunReport { pub writes: usize, pub selected: Option<HidInfo>, pub frames: usize }

/// Drive Validate -> Initialize -> Render x frames -> Shutdown against `candidates`.
pub fn run(src: &str, candidates: Vec<HidInfo>, live: bool, frames: usize) -> Result<RunReport> {
    let s: Shared = Rc::new(RefCell::new(Session { live, candidates: candidates.clone(), ..Default::default() }));
    with_plugin(src, &s, |ctx, m| {
        let vid: Option<u16> = call_opt(ctx, m, "VendorId");
        if let Some(v) = vid { s.borrow_mut().vid = v; }
        // Controllable parameters become globals holding their defaults.
        if let Ok(f) = m.get::<_, Function>("ControllableParameters") {
            let params: Value = f.call(())?;
            if let Some(arr) = params.as_array() {
                for p in arr.iter::<rquickjs::Object>().flatten() {
                    if let (Ok(name), Ok(def)) = (p.get::<_, String>("property"), p.get::<_, Value>("default")) {
                        ctx.globals().set(name, def)?;
                    }
                }
            }
        }
        // Pick the first candidate endpoint the plugin's Validate() accepts.
        let validate: Option<Function> = m.get("Validate").ok();
        let mut chosen = None;
        for c in &candidates {
            let ok = match &validate {
                Some(f) => {
                    let o = rquickjs::Object::new(ctx.clone())?;
                    o.set("interface", c.interface)?; o.set("usage", c.usage)?;
                    o.set("usage_page", c.usage_page)?; o.set("collection", 0)?;
                    f.call::<_, bool>((o,)).unwrap_or(false)
                }
                None => true,
            };
            if ok { chosen = Some(c.clone()); break; }
        }
        {
            let mut ss = s.borrow_mut();
            if let Some(c) = chosen {
                ss.pid = c.product_id;
                if live { ss.dev = HidDevice::open(&c.node).ok(); }
                ss.selected = Some(c);
            }
        }
        let _ : () = m.get::<_, Function>("Initialize").map(|f| f.call(())).unwrap_or(Ok(()))?;
        for _ in 0..frames { let _ : () = m.get::<_, Function>("Render").map(|f| f.call(())).unwrap_or(Ok(()))?; }
        let _ : () = m.get::<_, Function>("Shutdown").map(|f| f.call((false,))).unwrap_or(Ok(()))?;
        let ss = s.borrow();
        Ok(RunReport { writes: ss.writes, selected: ss.selected.clone(), frames })
    })
}

fn call<'js, T: rquickjs::FromJs<'js>>(_ctx: &Ctx<'js>, m: &Module<'js, Evaluated>, f: &str) -> Result<T> {
    let func: Function = m.get(f).map_err(|e| anyhow!("{f}: {e}"))?;
    func.call(()).map_err(|e| anyhow!("{f}(): {e}"))
}
fn call_opt<'js, T: rquickjs::FromJs<'js>>(_ctx: &Ctx<'js>, m: &Module<'js, Evaluated>, f: &str) -> Option<T> {
    m.get::<_, Function>(f).ok().and_then(|func| func.call(()).ok())
}

fn with_plugin<R>(src: &str, s: &Shared, f: impl for<'js> FnOnce(&Ctx<'js>, &Module<'js, Evaluated>) -> Result<R>) -> Result<R> {
    let rt = Runtime::new()?;
    rt.set_loader(Shims::load(), Shims::load());
    rt.set_memory_limit(256 * 1024 * 1024);
    let ctx = Context::full(&rt)?;
    ctx.with(|ctx| {
        let r = (|| -> Result<R> {
                    install(&ctx, s).map_err(|e| anyhow!("install: {e}"))?;
        ctx.eval::<(), _>(PRELUDE).map_err(|e| anyhow!("prelude: {e}"))?;
        let (m, p) = Module::declare(ctx.clone(), "plugin", src).map_err(|e| anyhow!("load: {e}"))?
            .eval().map_err(|e| anyhow!("load: {e}"))?;
        let _ : () = p.finish().map_err(|e| anyhow!("evaluate: {e}"))?;
        f(&ctx, &m)
        })();
        // Replace QuickJS's opaque "Exception generated" with the thrown value.
        r.map_err(|e| {
            let c = ctx.catch();
            let detail = c.as_exception().map(|x| format!("{} {}", x.message().unwrap_or_default(), x.stack().unwrap_or_default()))
                .or_else(|| c.as_string().and_then(|s| s.to_string().ok()));
            match detail { Some(d) if !d.trim().is_empty() => anyhow!("{e}: {}", d.trim()), _ => e }
        })
    })
}
