// End-to-end UI test: drives the real web UI in headless Chrome (real mouse events over CDP) against a
// running daemon and checks the results through its API. Needs Node 22+ and google-chrome.
// It changes live lighting settings while running and restores the global settings afterwards.
//   node tools/ui-test.mjs
import { spawn } from "node:child_process";
import { readFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const base = "http://127.0.0.1:7878";
const api = async (p, body) => (await fetch(base + p, body ? { method: "POST", headers: { "X-Refract": "1" }, body: JSON.stringify(body) } : {})).json();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let fails = 0;
const check = (name, ok, extra = "") => { console.log((ok ? "PASS " : "FAIL ") + name + (ok ? "" : "  " + extra)); if (!ok) fails++; };

const dir = mkdtempSync(join(tmpdir(), "uitest-"));
const chrome = spawn("google-chrome", ["--headless=new", "--disable-gpu", "--remote-debugging-port=0", `--user-data-dir=${dir}`, "--window-size=900,1400", "about:blank"], { stdio: "ignore" });
let port;
for (let i = 0; i < 50 && !port; i++) { await sleep(200); try { port = readFileSync(join(dir, "DevToolsActivePort"), "utf8").split("\n")[0]; } catch {} }
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const ws = new WebSocket(targets.find((t) => t.type === "page").webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const waiting = new Map(); const errors = [];
ws.onmessage = (m) => {
  const d = JSON.parse(m.data);
  if (d.id && waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); }
  if (d.method === "Runtime.exceptionThrown") errors.push(d.params.exceptionDetails.exception?.description || d.params.exceptionDetails.text);
};
const cdp = (method, params = {}) => new Promise((r) => { const i = ++id; waiting.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const ev = async (expr) => { const r = await cdp("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true }); if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description); return r.result.result.value; };
const click = async (x, y) => { for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) await cdp("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1, buttons: type === "mousePressed" ? 1 : 0 }); };
const center = (sel) => ev(`(() => { const e = ${sel}; e.scrollIntoView({block:"center"}); const r = e.getBoundingClientRect(); return [r.left + r.width/2, r.top + r.height/2]; })()`);

const saved = await api("/api/state");
try {
  // clean slate
  await api("/api/params", { target: "dram:0x71", reset: true }); await api("/api/params", { target: "dram:0x73", reset: true });
  await api("/api/params", { effect: "gradient", color: "#ff0080", color2: "#00ffd0", brightness: 1, on: true, speed: 1 });

  await cdp("Runtime.enable"); await cdp("Page.enable");
  await cdp("Emulation.setDeviceMetricsOverride", { width: 900, height: 1400, deviceScaleFactor: 1, mobile: false });
  await cdp("Page.navigate", { url: base + "/" }); await sleep(2500);
  check("page loads with a global panel", await ev(`document.querySelectorAll("#global .panel").length`) === 1);
  check("device cards render", await ev(`document.querySelectorAll("#devices .card").length`) >= 3);
  check("live strip has a gradient", (await ev(`document.querySelector("#global .strip").style.background`)).includes("linear-gradient"));

  // colour picker: open from the first swatch, drag in the SV square with the mouse
  let [x, y] = await center(`document.querySelector("#global .sw")`);
  await click(x, y); await sleep(200);
  check("picker opens", await ev(`getComputedStyle(document.getElementById("picker")).display`) === "block");
  const [sx, sy] = await center(`document.getElementById("sv")`);
  const r = await ev(`(() => { const r = document.getElementById("sv").getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()`);
  await cdp("Input.dispatchMouseEvent", { type: "mouseMoved", x: r[0] + r[2] * 0.9, y: r[1] + r[3] * 0.1 });
  await cdp("Input.dispatchMouseEvent", { type: "mousePressed", x: r[0] + r[2] * 0.9, y: r[1] + r[3] * 0.1, button: "left", clickCount: 1, buttons: 1 });
  await cdp("Input.dispatchMouseEvent", { type: "mouseReleased", x: r[0] + r[2] * 0.9, y: r[1] + r[3] * 0.1, button: "left", clickCount: 1 });
  await sleep(500);
  let st = await api("/api/state");
  const hex = st.global.color;
  check("dragging the picker changes the colour on the server", hex !== "#ff0080", hex);
  check("picker hex field matches", (await ev(`document.getElementById("hex").value`)) === hex, hex);
  // type a hex value
  await ev(`(() => { const h = document.getElementById("hex"); h.focus(); h.value = "#12ab34"; h.dispatchEvent(new Event("input")); })()`); await sleep(400);
  check("typing a hex value applies it", (await api("/api/state")).global.color === "#12ab34");
  await cdp("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape" }); await sleep(100);
  check("Escape closes the picker", await ev(`getComputedStyle(document.getElementById("picker")).display`) === "none");

  // per-device custom settings via the button
  [x, y] = await center(`[...document.querySelectorAll("#devices .card")].find(c => c.textContent.includes("0x71")).querySelector(".btn")`);
  await click(x, y); await sleep(900);
  st = await api("/api/state");
  check("Custom settings creates a device override", "dram:0x71" in st.overrides);
  check("override starts as a copy of global", st.overrides["dram:0x71"]?.color === "#12ab34", JSON.stringify(st.overrides["dram:0x71"]));
  check("the override panel appears", await ev(`document.querySelectorAll("#devices .panel").length`) === 1);

  // choose a SignalRGB canvas effect from the device's select
  await ev(`(() => { const s = document.querySelector("#devices .panel select"); s.value = "canvas:Static/Solid Color"; s.dispatchEvent(new Event("change")); })()`);
  await sleep(1500);
  st = await api("/api/state");
  check("canvas effect chosen for the device", st.overrides["dram:0x71"]?.effect === "canvas:Static/Solid Color", st.overrides["dram:0x71"]?.effect);
  check("global untouched", st.global.effect === "gradient");
  const labels = await ev(`[...document.querySelectorAll("#devices .panel .row .lbl")].map(e => e.textContent)`);
  check("canvas effect controls are generated", ["Color", "Breathing", "Breathing Speed"].every((l) => labels.includes(l)), JSON.stringify(labels));

  // toggle the canvas effect's own boolean and its colour swatch
  await ev(`(() => { const row = [...document.querySelectorAll("#devices .panel .row")].find(r => r.textContent.includes("Breathing") && r.querySelector("input[type=checkbox]") && !r.textContent.includes("Speed")); const c = row.querySelector("input"); c.checked = false; c.dispatchEvent(new Event("change")); })()`);
  await sleep(500);
  check("canvas boolean option applied", (await api("/api/state")).overrides["dram:0x71"].opts.breathe === false);
  const prev = await (await fetch(base + "/api/preview?t=" + encodeURIComponent("dram:0x71"))).json();
  await sleep(300);
  const prev2 = await (await fetch(base + "/api/preview?t=" + encodeURIComponent("dram:0x71"))).json();
  check("with breathing off the canvas effect holds steady", prev["dram:0x71"][0] === prev2["dram:0x71"][0], prev["dram:0x71"][0] + " vs " + prev2["dram:0x71"][0]);

  // reset
  [x, y] = await center(`[...document.querySelectorAll("#devices .card")].find(c => c.textContent.includes("0x71")).querySelector(".btn")`);
  await click(x, y); await sleep(900);
  check("toggling Custom off removes the override", !("dram:0x71" in (await api("/api/state")).overrides));

  // zones
  [x, y] = await center(`[...document.querySelectorAll("#devices .btn")].find(b => b.textContent.startsWith("Zones"))`);
  await click(x, y); await sleep(700);
  check("zone list opens", (await ev(`document.querySelectorAll("#devices .zone").length`)) >= 2);
  [x, y] = await center(`document.querySelector("#devices .zone .btn")`);
  await click(x, y); await sleep(900);
  check("zone override created", Object.keys((await api("/api/state")).overrides).some((k) => k.includes("/")));
  await api("/api/params", { target: "usb:MSI Mystic Light Controller", reset: true });

  check("no JavaScript errors in the page", errors.length === 0, errors.join(" | "));
} catch (e) { console.log("ERROR", e.message); fails++; }
finally {
  for (const t of ["dram:0x71", "dram:0x73", "usb:MSI Mystic Light Controller"]) await api("/api/params", { target: t, reset: true }).catch(() => {});
  const g = saved.global;
  await api("/api/params", { effect: g.effect, color: g.color, color2: g.color2, speed: g.speed, brightness: g.brightness, on: g.on, opts: g.opts }).catch(() => {});
  ws.close(); chrome.kill();
  console.log(fails ? `${fails} failed` : "all passed");
  process.exit(fails ? 1 : 0);
}
