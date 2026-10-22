mod hid;
mod i2c;
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
            if !(0x70..=0x77).contains(&addr) { bail!("ene-color only drives 0x70-0x77"); }
            let [r, g, b] = parse_color(a.get(3).context("colour")?)?;
            let mut bus = i2c::Bus::open(n, true)?;
            bus.set_addr(addr)?;
            let id: Vec<u8> = (0..16u16).map(|i| bus.ene_read(0x1000 + i)).collect::<Result<_>>()?;
            if !id.starts_with(b"AUDA0-E6K5-0101") { bail!("unrecognised firmware {:?}; refusing to write", String::from_utf8_lossy(&id)); }
            let leds = bus.ene_read(0x1c02)?.max(bus.ene_read(0x1c03)?) as usize;
            if leds == 0 || leds > 15 { bail!("implausible LED count {leds}"); }
            println!("{addr:#04x}: {leds} LEDs");
            bus.ene_write(0x8020, 1)?;                       // direct mode
            let data: Vec<u8> = (0..leds).flat_map(|_| [r, b, g]).collect(); // ENE wants R,B,G
            for (i, chunk) in data.chunks(15).enumerate() { bus.ene_write_block(0x8100 + (i * 15) as u16, chunk)?; }
            bus.ene_write(0x802f, 1)?;                       // apply (no flash save)
        }
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
            let r = host::run(&src, cands, host::RunOpts { live, frames, color, hold })?;
            println!("selected={:?} writes={} frames={}", r.selected.map(|d| d.node), r.writes, r.frames);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
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
            host::run(&src, vec![cand], host::RunOpts { live: false, frames: 1, color: [0; 3], hold: false }).map(|_| ())
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
