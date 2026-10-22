//! Discovery of SignalRGB HTML effects: each is a page drawing on `#exCanvas`, with
//! `<meta property=...>` tags declaring the controls the host should offer.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Control {
    pub property: String,
    pub label: String,
    pub kind: String,
    pub min: f64,
    pub max: f64,
    pub step: Option<f64>,
    pub values: Vec<String>,
    pub default: Value,
}

#[derive(Clone, Debug)]
pub struct EffectInfo {
    /// Path relative to its effects directory, without extension: `Static/Rainbow`.
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: String,
    pub path: PathBuf,
    pub controls: Vec<Control>,
}

impl EffectInfo {
    pub fn json(&self) -> Value {
        let controls: Vec<Value> = self.controls.iter().map(|c| json!({
            "property": c.property, "label": c.label, "type": c.kind, "min": c.min, "max": c.max,
            "step": c.step, "values": c.values, "default": c.default,
        })).collect();
        json!({
            "id": format!("canvas:{}", self.id), "name": self.name, "group": self.group, "description": self.description,
            "color": false, "color2": false, "speed": false, "controls": controls,
        })
    }

    /// Control defaults as the globals the page expects.
    pub fn defaults(&self) -> serde_json::Map<String, Value> {
        self.controls.iter().map(|c| (c.property.clone(), c.default.clone())).collect()
    }
}

/// Attributes of every `<meta ...>` tag in the document head, in order.
fn meta_tags(html: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<meta") {
        // Skip commented-out tags.
        let before = &html[..html.len() - rest.len() + i];
        let in_comment = before.rfind("<!--").is_some_and(|c| before[c..].find("-->").is_none());
        let tail = &rest[i + 5..];
        let end = tail.find('>').unwrap_or(tail.len());
        if !in_comment { out.push(attrs(&tail[..end])); }
        rest = &tail[end..];
    }
    out
}

fn attrs(s: &str) -> Vec<(String, String)> {
    let b: Vec<char> = s.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        while i < b.len() && (b[i].is_whitespace() || b[i] == '/') { i += 1; }
        let k0 = i;
        while i < b.len() && !b[i].is_whitespace() && b[i] != '=' && b[i] != '/' { i += 1; }
        let key: String = b[k0..i].iter().collect();
        if key.is_empty() { break; }
        while i < b.len() && b[i].is_whitespace() { i += 1; }
        let mut val = String::new();
        if i < b.len() && b[i] == '=' {
            i += 1;
            while i < b.len() && b[i].is_whitespace() { i += 1; }
            if i < b.len() && (b[i] == '"' || b[i] == '\'') {
                let q = b[i]; i += 1;
                while i < b.len() && b[i] != q { val.push(b[i]); i += 1; }
                i += 1;
            } else {
                while i < b.len() && !b[i].is_whitespace() { val.push(b[i]); i += 1; }
            }
        }
        out.push((key.to_lowercase(), val));
    }
    out
}

fn get<'a>(a: &'a [(String, String)], k: &str) -> Option<&'a str> { a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()) }

fn title(html: &str) -> Option<String> {
    let s = html.find("<title>")? + 7;
    Some(html[s..s + html[s..].find("</title>")?].trim().to_string())
}

/// Parse one effect. `None` if it needs host features we do not provide (screen or audio capture).
pub fn parse(id: &str, group: &str, path: &Path, html: &str) -> Option<EffectInfo> {
    if html.contains("engine.zone") || html.contains("engine.audio") || html.contains("getDisplayMedia") { return None; }
    let mut name = title(html).unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string());
    let mut description = String::new();
    let mut controls = Vec::new();
    for a in meta_tags(html) {
        if let Some(d) = get(&a, "description") { description = d.to_string(); }
        let Some(prop) = get(&a, "property") else { continue };
        let kind = get(&a, "type").unwrap_or("text").to_string();
        let raw = get(&a, "default").unwrap_or("");
        let num = |k: &str| get(&a, k).and_then(|v| v.trim().parse::<f64>().ok());
        let default = match kind.as_str() {
            "boolean" => Value::Bool(matches!(raw.trim(), "1" | "true")),
            "number" => json!(raw.trim().parse::<f64>().unwrap_or(0.0)),
            _ => Value::String(raw.to_string()),
        };
        controls.push(Control {
            property: prop.to_string(),
            label: get(&a, "label").unwrap_or(prop).to_string(),
            kind,
            min: num("min").unwrap_or(0.0),
            max: num("max").unwrap_or(100.0),
            step: num("step"),
            values: get(&a, "values").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default(),
            default,
        });
    }
    if name.is_empty() { name = id.to_string(); }
    Some(EffectInfo { id: id.to_string(), name, description, group: group.to_string(), path: path.to_path_buf(), controls })
}

/// All usable effects under `dir`, labelled with `group`.
pub fn scan(dir: &Path, group: &str) -> Vec<EffectInfo> {
    // Chrome needs absolute file URLs.
    let Ok(dir) = std::fs::canonicalize(dir) else { return Vec::new() };
    let dir = dir.as_path();
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x == "html") { out.push(p) }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files.iter().filter_map(|f| {
        let html = std::fs::read_to_string(f).ok()?;
        let id = f.strip_prefix(dir).ok()?.with_extension("").to_string_lossy().replace('\\', "/");
        parse(&id, group, f, &html)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_controls_and_defaults() {
        let html = r##"<head><title>Demo</title>
          <meta description="A demo." />
          <meta property="speed" label="Speed" type="number" min="0" max="100" default="50" />
          <meta property="loop" label="Loop" type="boolean" default="1">
          <meta property="dir" label="Direction" type="combobox" values="Left,Right" default="Right">
          <!-- <meta property="old" type="color" default="#000000"> -->
          <meta property="tint" type="color" default="#00BBff">
        </head>"##;
        let e = parse("X/Demo", "SignalRGB", Path::new("x"), html).unwrap();
        assert_eq!(e.name, "Demo");
        assert_eq!(e.description, "A demo.");
        let names: Vec<_> = e.controls.iter().map(|c| c.property.as_str()).collect();
        assert_eq!(names, ["speed", "loop", "dir", "tint"]); // commented-out tag skipped
        assert_eq!(e.controls[0].default, json!(50.0));
        assert_eq!(e.controls[1].default, json!(true));
        assert_eq!(e.controls[2].values, ["Left", "Right"]);
        assert_eq!(e.controls[3].default, json!("#00BBff"));
    }

    #[test]
    fn rejects_effects_needing_host_capture() {
        assert!(parse("a", "g", Path::new("a"), "<title>t</title><script>engine.zone.hue</script>").is_none());
    }

    #[test]
    fn bundled_effects_parse() {
        let dir = Path::new("extracted/stage3/lib/net45/Signal-x64/Effects");
        if !dir.is_dir() { return; }
        let list = scan(dir, "SignalRGB");
        let ids: Vec<_> = list.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"Static/Rainbow") && ids.contains(&"Static/Side To Side"), "{ids:?}");
        assert!(!ids.contains(&"Dynamic/Screen Ambience"), "needs engine.zone");
        let rainbow = list.iter().find(|e| e.id == "Static/Rainbow").unwrap();
        assert_eq!(rainbow.controls.len(), 2);
        let side = list.iter().find(|e| e.id == "Static/Side To Side").unwrap();
        assert!(side.controls.iter().any(|c| c.property == "direction" && c.values.len() == 3));
    }
}
