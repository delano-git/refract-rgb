mod hid;
mod canvas;
mod daemon;
mod ene;
mod effects;
mod engine;
mod fx;
mod i2c;
mod tray;
mod web;
mod host;

use anyhow::{Context, Result, bail};
use std::fs;

const USAGE: &str = "refract <command>
  list                          list hidraw devices
  info <plugin.js>              print a plugin's name and USB IDs
  i2c                           list SMBus adapters
  i2c-scan <bus>                passive scan of one adapter (receive-byte only, skips 0x50-0x57)
  ene-id <bus> <addr>           read an ENE RGB controller's firmware string (0x70-0x77 only)
  ene-color <bus> <addr> <#rrggbb>
                                solid colour on an ENE V2 DRAM controller (volatile, never saved)
  apply <#rrggbb> [--plugins DIR] [--dry-run]
                                set one colour on every supported device found (USB plugins + DRAM)
  daemon [--plugins DIR]        run the app: effects engine, tray icon, web UI on 127.0.0.1:7878
  install-service               autostart the daemon at login (systemd user unit)
  canvas-test <effects-dir> <id>  render a SignalRGB HTML effect in Chrome and print sampled colours
  sweep <dir>                   dry-run every plugin under <dir>, summarise failures
  run <plugin.js> [--live] [--frames N] [--color #rrggbb] [--hold]
                                drive a plugin's lifecycle. Without --live no
                                bytes are sent to hardware (dry run).";

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.first().map(String::as_str) {
        Some("list") => {
            for d in hid::enumerate() {
                println!("{:<10} {:04x}:{:04x} if={} page={:#06x} usage={:#06x}  {}",
                    d.node, d.vendor_id, d.product_id, d.interface, d.usage_page, d.usage, d.name);
            }
        }
        Some("info") => {
            let src = fs::read_to_string(a.get(1).context("plugin path")?)?;
            let i = host::inspect(&src)?;
            println!("{} vid={:?} pids={:04x?}", i.name, i.vendor_id.map(|v| format!("{v:04x}")), i.product_ids);
        }
        Some("i2c") => for ad in i2c::adapters() { println!("i2c-{:<3} {}", ad.num, ad.name); },
        Some("i2c-scan") => {
            let n: u32 = a.get(1).context("bus number")?.parse()?;
            for (addr, what) in i2c::scan(n)? { println!("{addr:#04x}  {what}"); }
        }
        Some("ene-id") => {
            let n: u32 = a.get(1).context("bus number")?.parse()?;
            let addr = u16::from_str_radix(a.get(2).context("address")?.trim_start_matches("0x"), 16)?;
            if !(0x70..=0x77).contains(&addr) { bail!("ene-id only probes 0x70-0x77"); }
            let mut bus = i2c::Bus::open(n, true)?;
            bus.set_addr(addr)?;
            let id: Vec<u8> = (0..16u16).map(|i| bus.ene_read(0x1000 + i)).collect::<Result<_>>()?;
            println!("{addr:#04x}: {:?}  raw={id:02x?}", String::from_utf8_lossy(&id).trim_end_matches('\0'));
        }
        Some("ene-color") => {
            let n: u32 = a.get(1).context("bus number")?.parse()?;
            let addr = u16::from_str_radix(a.get(2).context("address")?.trim_start_matches("0x"), 16)?;
            let leds = ene::set_color(n, addr, parse_color(a.get(3).context("colour")?)?)?;
            println!("{addr:#04x}: {leds} LEDs");
        }
        Some("daemon") => daemon::run(a.iter().position(|x| x == "--plugins").and_then(|i| a.get(i + 1)).map(Into::into))?,
        Some("canvas-test") => {
            let dir = std::path::PathBuf::from(a.get(1).context("effects dir")?);
            let id = a.get(2).context("effect id, e.g. Static/Rainbow")?;
            let list = std::sync::Arc::new(effects::scan(&dir, "test"));
            let fx = std::sync::Arc::new(fx::Fx::new(fx::Params { effect: format!("canvas:{id}"), ..Default::default() }, Default::default()));
            canvas::start(fx.clone(), list, canvas::find_chrome().context("no Chrome found")?);
            let rounds = std::env::var("REFRACT_TEST_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
            for _ in 0..rounds {
                std::thread::sleep(std::time::Duration::from_millis(700));
                let row: Vec<String> = (0..5).map(|i| { let c = fx.color("", "", i as f64 / 4.0, 0.5); format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }).collect();
                println!("{}", row.join(" "));
            }
        }
        Some("install-service") => install_service()?,
        Some("apply") => apply(&a[1..])?,
        Some("sweep") => sweep(a.get(1).context("plugins dir")?)?,
        Some("run") => {
            let path = a.get(1).context("plugin path")?;
            let live = a.iter().any(|x| x == "--live");
            let frames = a.iter().position(|x| x == "--frames").and_then(|i| a.get(i + 1)).and_then(|x| x.parse().ok()).unwrap_or(1);
            let hold = a.iter().any(|x| x == "--hold");
            let color = match a.iter().position(|x| x == "--color").and_then(|i| a.get(i + 1)) {
                Some(c) => parse_color(c)?,
                None => [0, 0, 0],
            };
            let src = fs::read_to_string(path)?;
            let info = host::inspect(&src)?;
            // Only devices matching the plugin's USB IDs are candidates.
            let cands: Vec<_> = hid::enumerate().into_iter()
                .filter(|d| info.vendor_id == Some(d.vendor_id) && info.product_ids.contains(&d.product_id))
                .collect();
            println!("{}: {} matching hidraw endpoint(s){}", info.name, cands.len(), if live { " [LIVE]" } else { " [dry run]" });
            let r = host::run(&src, cands, host::RunOpts::once(live, solid(color), frames, hold))?;
            println!("selected={:?} writes={} frames={}", r.selected.map(|d| d.node), r.writes, r.frames);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

fn solid(color: [u8; 3]) -> std::sync::Arc<fx::Fx> {
    std::sync::Arc::new(fx::Fx::new(fx::Params { effect: "solid".into(), color, ..Default::default() }, Default::default()))
}

fn parse_color(s: &str) -> Result<[u8; 3]> {
    let h = s.trim_start_matches('#');
    if h.len() != 6 { bail!("colour must be #rrggbb"); }
    let v = u32::from_str_radix(h, 16).context("colour must be #rrggbb")?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { walk(&p, out); } else if p.extension().is_some_and(|x| x == "js") { out.push(p); }
    }
}

/// Dry-run every plugin with a synthetic endpoint matching its first USB ID.
fn sweep(dir: &str) -> Result<()> {
    let mut files = Vec::new();
    walk(std::path::Path::new(dir), &mut files);
    files.sort();
    let (mut ok, mut failed) = (0, Vec::new());
    for f in &files {
        let res = (|| -> Result<()> {
            let src = fs::read_to_string(f)?;
            let info = host::inspect(&src)?;
            let cand = hid::HidInfo {
                node: String::new(), vendor_id: info.vendor_id.unwrap_or(0),
                product_id: info.product_ids.first().copied().unwrap_or(0),
                name: info.name, interface: 0, usage_page: 0, usage: 0,
            };
            host::run(&src, vec![cand], host::RunOpts::once(false, solid([0; 3]), 1, false)).map(|_| ())
        })();
        match res { Ok(()) => ok += 1, Err(e) => failed.push((f.clone(), format!("{e:#}"))) }
    }
    println!("{} plugins: {} ok, {} failed", files.len(), ok, failed.len());
    let mut groups: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, e) in &failed {
        let k: String = e.lines().next().unwrap_or("").chars().take(90).collect();
        *groups.entry(k).or_default() += 1;
    }
    let mut g: Vec<_> = groups.into_iter().collect();
    g.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in g.iter().take(25) { println!("{n:4}  {k}"); }
    Ok(())
}

/// Plugin directory: --plugins, $REFRACT_PLUGINS, or the extracted vendor tree if present.
fn plugins_dir(a: &[String]) -> Option<std::path::PathBuf> {
    a.iter().position(|x| x == "--plugins").and_then(|i| a.get(i + 1)).map(Into::into)
        .or_else(|| std::env::var_os("REFRACT_PLUGINS").map(Into::into))
        .or_else(|| Some("extracted/stage3/lib/net45/Signal-x64/Plugins".into()).filter(|p: &std::path::PathBuf| p.is_dir()))
}

fn apply(a: &[String]) -> Result<()> {
    let color = parse_color(a.first().context("colour #rrggbb")?)?;
    let live = !a.iter().any(|x| x == "--dry-run");
    let dir = plugins_dir(a).context("no plugin directory: pass --plugins DIR or set REFRACT_PLUGINS")?;
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();

    // USB/HID devices: run every plugin whose VID/PID matches a connected device, once per node.
    let devices = hid::enumerate();
    let mut claimed: std::collections::HashSet<String> = Default::default();
    for f in &files {
        let Ok(src) = fs::read_to_string(f) else { continue };
        let Ok(info) = host::inspect(&src) else { continue };
        let cands: Vec<_> = devices.iter()
            .filter(|d| info.vendor_id == Some(d.vendor_id) && info.product_ids.contains(&d.product_id) && !claimed.contains(&d.node))
            .cloned().collect();
        if cands.is_empty() { continue; }
        let opts = host::RunOpts::once(live, solid(color), 3, true);
        match host::run(&src, cands.clone(), opts) {
            Ok(r) if r.writes > 0 && r.selected.is_some() => {
                let node = r.selected.map(|d| d.node).unwrap_or_default();
                println!("ok    {:<32} {node} ({} writes)", info.name, r.writes);
                claimed.extend(cands.into_iter().map(|d| d.node));
            }
            Ok(_) => println!("skip  {:<32} no endpoint accepted", info.name),
            Err(e) => println!("FAIL  {:<32} {}", info.name, format!("{e:#}").lines().next().unwrap_or("")),
        }
    }

    // DRAM: ENE controllers sit at each SPD hub address + 0x20.
    for (bus, addr) in ene::find_dram() {
        if !live { println!("plan  ENE DRAM i2c-{bus} {addr:#04x}"); continue; }
        match ene::set_color(bus, addr, color) {
            Ok(n) => println!("ok    ENE DRAM i2c-{bus} {addr:#04x} ({n} LEDs)"),
            Err(e) => println!("FAIL  ENE DRAM i2c-{bus} {addr:#04x} {e:#}"),
        }
    }
    Ok(())
}

/// Install and start a systemd user unit that runs `refract daemon` at login.
fn install_service() -> Result<()> {
    let exe = std::env::current_exe()?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user");
    fs::create_dir_all(&dir)?;
    // The config stores the plugin dir after first run, but seed it from the current tree so the unit works unattended.
    let plugins = plugins_dir(&[]).and_then(|p| fs::canonicalize(p).ok())
        .map(|p| format!(" --plugins {}", p.display().to_string().replace(' ', "\\x20"))).unwrap_or_default();
    let unit = format!(
        "[Unit]\nDescription=Refract RGB lighting\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n\
         [Service]\nExecStart={} daemon{plugins}\nRestart=on-failure\nRestartSec=3\n\n[Install]\nWantedBy=graphical-session.target\n",
        exe.display().to_string().replace(' ', "\\x20"));
    fs::write(dir.join("refract-rgb.service"), unit)?;
    for args in [&["daemon-reload"][..], &["enable", "--now", "refract-rgb.service"]] {
        let st = std::process::Command::new("systemctl").arg("--user").args(args).status()?;
        if !st.success() { bail!("systemctl --user {} failed", args.join(" ")); }
    }
    println!("installed and started; manage with: systemctl --user status refract-rgb");
    Ok(())
}
