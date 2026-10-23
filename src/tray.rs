//! StatusNotifier tray icon: shows the live colour, quick effect picks, and opens the web UI.
use crate::daemon::{App, PORT};
use crate::fx::BUILTIN;
use anyhow::{Result, anyhow};
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};
use std::sync::Arc;
use std::time::Duration;

struct RefractTray { app: Arc<App> }

impl ksni::Tray for RefractTray {
    fn id(&self) -> String { "refract-rgb".into() }
    fn title(&self) -> String { "Refract RGB".into() }
    fn category(&self) -> ksni::Category { ksni::Category::Hardware }
    fn icon_name(&self) -> String { String::new() }

    // A light ring (visible on dark panels) with a dot inside in whatever colour the lights are
    // showing right now. When off, or near-black, only the ring is drawn.
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        const N: i32 = 32;
        let c = self.app.fx.color("", "", 0.5, 0.5);
        let dark = !self.app.fx.params().on || c.iter().map(|&v| v as u32).sum::<u32>() < 60;
        let mut data = Vec::with_capacity((N * N * 4) as usize);
        for y in 0..N {
            for x in 0..N {
                let (dx, dy) = (x as f32 + 0.5 - N as f32 / 2.0, y as f32 + 0.5 - N as f32 / 2.0);
                let d = (dx * dx + dy * dy).sqrt();
                let outer = (N as f32 / 2.0 - 2.0 - d + 0.5).clamp(0.0, 1.0); // antialiased edge
                let ring = outer * (d - (N as f32 / 2.0 - 6.0) + 0.5).clamp(0.0, 1.0);
                let dot = (N as f32 / 2.0 - 8.0 - d + 0.5).clamp(0.0, 1.0);
                if dark || ring >= 1.0 && dot == 0.0 {
                    data.extend_from_slice(&[(ring * 255.0) as u8, 200, 200, 200]);
                } else {
                    // ring over dot, blended where they meet
                    let a = ring + dot * (1.0 - ring);
                    let mix = |dc: u8| ((200.0 * ring + dc as f32 * dot * (1.0 - ring)) / a) as u8;
                    data.extend_from_slice(&[(a * 255.0) as u8, mix(c[0]), mix(c[1]), mix(c[2])]);
                }
            }
        }
        vec![ksni::Icon { width: N, height: N, data }]
    }

    fn activate(&mut self, _x: i32, _y: i32) { open_ui(); }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let p = self.app.fx.params();
        let selected = BUILTIN.iter().position(|e| e.0 == p.effect).unwrap_or(0);
        vec![
            StandardItem { label: "Open controls".into(), activate: Box::new(|_| open_ui()), ..Default::default() }.into(),
            MenuItem::Separator,
            StandardItem {
                label: if p.on { "💡  Lights on  (click to turn off)".into() } else { "🌙  Lights off  (click to turn on)".into() },
                activate: Box::new(|t: &mut Self| t.app.update("", |s| s.on = !s.on)),
                ..Default::default()
            }.into(),
            RadioGroup {
                selected,
                select: Box::new(|t: &mut Self, i| { let e = BUILTIN[i].0; t.app.update("", |s| { s.effect = e.into(); s.on = true; }) }),
                options: BUILTIN.iter().map(|e| RadioItem { label: e.1.into(), ..Default::default() }).collect(),
            }.into(),
            MenuItem::Separator,
            StandardItem { label: "Quit".into(), activate: Box::new(|t: &mut Self| t.app.quit()), ..Default::default() }.into(),
        ]
    }
}

fn open_ui() {
    let _ = std::process::Command::new("xdg-open").arg(format!("http://127.0.0.1:{PORT}/")).spawn();
}

pub fn spawn(app: Arc<App>) -> Result<()> {
    let handle = RefractTray { app }.spawn().map_err(|e| anyhow!("{e}"))?;
    // Keep the icon colour and menu in step with changes made elsewhere (web UI, animation).
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(800));
        if handle.update(|_| {}).is_none() { break; }
    });
    Ok(())
}
