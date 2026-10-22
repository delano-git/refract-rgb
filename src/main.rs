mod hid;
mod host;

use anyhow::{Context, Result, bail};
use std::fs;

const USAGE: &str = "refract <command>
  list                          list hidraw devices
  info <plugin.js>              print a plugin's name and USB IDs
  run <plugin.js> [--live] [--frames N]
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
        Some("run") => {
            let path = a.get(1).context("plugin path")?;
            let live = a.iter().any(|x| x == "--live");
            let frames = a.iter().position(|x| x == "--frames").and_then(|i| a.get(i + 1)).and_then(|x| x.parse().ok()).unwrap_or(1);
            let src = fs::read_to_string(path)?;
            let info = host::inspect(&src)?;
            // Only devices matching the plugin's USB IDs are candidates.
            let cands: Vec<_> = hid::enumerate().into_iter()
                .filter(|d| info.vendor_id == Some(d.vendor_id) && info.product_ids.contains(&d.product_id))
                .collect();
            println!("{}: {} matching hidraw endpoint(s){}", info.name, cands.len(), if live { " [LIVE]" } else { " [dry run]" });
            let r = host::run(&src, cands, live, frames)?;
            println!("selected={:?} writes={} frames={}", r.selected.map(|d| d.node), r.writes, r.frames);
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
