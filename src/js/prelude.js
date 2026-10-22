// Defines the `device` host object over the native __hid* bindings.
// Everything not backed by hardware is stored or ignored for now.
(function () {
  const n = globalThis;
  // Effects are evaluated natively; plugins sample them by normalised position, as they would a canvas.
  const nrm = (x, size) => (size > 1 ? Math.min(1, Math.max(0, x / (size - 1))) : 0.5);
  const sample = (u, v) => __fx(u, v);
  const hex = (h) => { const v = parseInt(String(h).replace("#", ""), 16) || 0; return [(v >> 16) & 255, (v >> 8) & 255, v & 255]; };
  const fill = (n) => { const out = []; for (let i = 0; i < n; i++) out.push(...sample(n > 1 ? i / (n - 1) : 0.5, 0.5)); return out; };
  const state = { leds: [], channels: [], props: {}, name: "", size: [1, 1], sub: {} };
  n.device = {
    write: (d, len) => __hidWrite(d, len ?? d.length),
    send_report: (d, len) => __hidSendReport(d, len ?? d.length),
    get_report: (d, len) => __hidGetReport(d, len ?? d.length),
    input_report: (d, len) => __hidGetReport(d, len ?? d.length),
    read: (d, len, timeout) => __hidRead(typeof d === "number" ? d : (len ?? d.length), timeout ?? 10),
    set_endpoint: (i, u, p, c) => __hidSetEndpoint(i ?? -1, u ?? 0, p ?? 0, c ?? 0),
    pause: (ms) => __sleep(+ms || 0),
    log: (m) => __log(String(m)),
    notify() {}, denotify() {}, flush() {}, clearReadBuffer() {}, getLastReadSize: () => 0,
    getHidEndpoints: () => __hidEndpoints(), getHidInfo: () => ({}), getDeviceInfo: () => ({}),
    productId: () => __hidProductId(), vendorId: () => __hidVendorId(),
    setName(v) { state.name = v; }, setSize(v) { state.size = v; },
    setControllableLeds(names, pos) { state.leds = [names, pos]; },
    setImageFromUrl() {}, setFrameRateTarget() {}, SetLedLimit() {},
    addChannel(...a) { state.channels.push(a); },
    getChannelNames: () => state.channels.map(c => c[0]),
    channel(name) {
      const c = state.channels.find(x => x[0] === name);
      const n = c ? (c[1] | 0) : 0;
      return { shouldPulseColors: () => false, LedCount: () => n, ledCount: n, getColors: () => fill(n), getComponentNames: () => [] };
    },
    getLedCount: () => 0, getChannelPulseColor: () => [0, 0, 0],
    createSubdevice() {}, removeSubdevice() {}, setSubdeviceName() {}, setSubdeviceSize(name, w, h) { state.sub[name] = [w, h]; },
    setSubdeviceLeds() {}, setSubdeviceImageUrl() {}, setSubdeviceImage() {}, getCurrentSubdevices: () => [],
    color: (x, y) => sample(nrm(x, state.size[0]), nrm(y, state.size[1])),
    subdeviceColor: (name, x, y) => { const sz = state.sub[name] ?? [1, 1]; return sample(nrm(x, sz[0]), nrm(y, sz[1])); }, getBrightness: () => 100,
    createColorArray(c, len) {
      if (typeof c === "number") return new Array(Math.max(0, c | 0) * 3).fill(0);
      const rgb = typeof c === "string" ? hex(c) : c;
      const out = [];
      for (let i = 0; i < (len | 0); i++) out.push(rgb[0], rgb[1], rgb[2]);
      return out;
    },
    addProperty(p) { state.props[p?.property ?? ""] = p; }, removeProperty() {}, getProperty: () => undefined,
    addFeature() {}, setDeviceType() {},
    fanControlDisabled: () => true, createFanControl() {}, removeFanControl() {}, setRPM() {},
    getNormalizedFanlevel: () => 0, getFanlevel: () => 0,
    createTemperatureSensor() {}, removeTemperatureSensor() {}, SetTemperature() {},
    bulk_transfer: () => 0, control_transfer: () => 0, setProtocol() {},
  };
  const noop = () => {};
  n.console = { log: (...a) => __log(a.join(' ')), warn: (...a) => __log(a.join(' ')), error: (...a) => __log(a.join(' ')), info: noop, debug: noop };
  n.hid = { write: (d, l) => __hidWrite(d, l ?? d.length) };
  n.serial = { isConnected: () => false, write: noop, connect: noop, disconnect: noop, availablePorts: () => [] };
  n.systeminfo = {
    GetMotherboardInfo: () => { const m = __dmi("board_name"); return m ? { model: m, product: m, manufacturer: __dmi("board_vendor"), version: __dmi("board_version") } : null; },
    GetBiosInfo: () => { const v = __dmi("bios_version"); return v ? { vendor: __dmi("bios_vendor"), version: v, date: __dmi("bios_date") } : null; },
    GetRamInfo: () => [],
  };
  n.bus = new Proxy({ log: noop }, { get: (t, k) => (k in t ? t[k] : () => 0) });
  n.battery = { setBatteryLevel: noop, setBatteryState: noop, lowBatteryWarningLevel: 0 };
})();
