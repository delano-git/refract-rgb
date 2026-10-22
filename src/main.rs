mod hid;
mod host;

use anyhow::{Context, Result, bail};
use std::fs;

const USAGE: &str = "refract <command>
  list                          list hidraw devices
  info <plugin.js>              print a plugin's name and USB IDs
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
