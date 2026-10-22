//! Loopback HTTP control surface: the web UI and a small JSON API.
use crate::daemon::App;
use crate::fx::{Effect, EFFECTS};
use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tiny_http::{Header, Method, Request, Response, Server};

const UI: &str = include_str!("ui.html");

#[derive(Deserialize)]
struct Patch {
    effect: Option<Effect>,
    color: Option<String>,
    color2: Option<String>,
    speed: Option<f32>,
    brightness: Option<f32>,
    on: Option<bool>,
}

pub fn hex(c: [u8; 3]) -> String { format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]) }

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 { return None; }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn serve(app: Arc<App>, port: u16) -> Result<()> {
    let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow!("bind 127.0.0.1:{port}: {e}"))?;
    for req in server.incoming_requests() {
        let app = app.clone();
        std::thread::spawn(move || { let _ = handle(&app, req, port); });
    }
    Ok(())
}

fn header(req: &Request, name: &'static str) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.to_string())
}

fn json_resp(v: serde_json::Value, code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(v.to_string()).with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
}

fn handle(app: &App, mut req: Request, port: u16) -> Result<()> {
    // Any web page can reach localhost, so refuse foreign Host headers (DNS rebinding)
    // and require a custom header on writes, which cross-origin pages cannot send without a preflight we never grant.
    let host_ok = header(&req, "Host").is_some_and(|h| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if !host_ok { return Ok(req.respond(json_resp(json!({"error": "bad host"}), 403))?); }

    let (method, url) = (req.method().clone(), req.url().to_string());
    match (method, url.as_str()) {
        (Method::Get, "/") => {
            let r = Response::from_string(UI).with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
            Ok(req.respond(r)?)
        }
        (Method::Get, "/api/state") => {
            let p = app.fx.params();
            let devices: Vec<_> = app.engine.status.lock().unwrap().values().cloned().collect();
            let effects: Vec<_> = EFFECTS.iter().map(|(e, n)| json!({"id": e, "name": n})).collect();
            Ok(req.respond(json_resp(json!({
                "effect": p.effect, "color": hex(p.color), "color2": hex(p.color2),
                "speed": p.speed, "brightness": p.brightness, "on": p.on,
                "effects": effects, "devices": devices,
            }), 200))?)
        }
        (Method::Post, "/api/params") => {
            if header(&req, "X-Refract").is_none() { return Ok(req.respond(json_resp(json!({"error": "missing X-Refract"}), 403))?); }
            let mut body = String::new();
            std::io::Read::read_to_string(&mut req.as_reader().take(4096), &mut body)?;
            let Ok(p) = serde_json::from_str::<Patch>(&body) else { return Ok(req.respond(json_resp(json!({"error": "bad json"}), 400))?) };
            app.update(|s| {
                if let Some(e) = p.effect { s.effect = e; }
                if let Some(c) = p.color.as_deref().and_then(parse_hex) { s.color = c; }
                if let Some(c) = p.color2.as_deref().and_then(parse_hex) { s.color2 = c; }
                if let Some(v) = p.speed { s.speed = v.clamp(0.1, 5.0); }
                if let Some(v) = p.brightness { s.brightness = v.clamp(0.0, 1.0); }
                if let Some(v) = p.on { s.on = v; }
            });
            Ok(req.respond(json_resp(json!({"ok": true}), 200))?)
        }
        _ => Ok(req.respond(json_resp(json!({"error": "not found"}), 404))?),
    }
}

use std::io::Read as _;
