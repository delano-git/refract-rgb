// Defines the `device` host object over the native __hid* bindings.
// Everything not backed by hardware is stored or ignored for now.
(function () {
  const n = globalThis;
  const state = { leds: [], channels: [], props: {}, name: "", size: [1, 1] };
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
    channel: () => ({ shouldPulseColors: () => false, LedCount: () => 0, ledCount: 0, getColors: () => [], getComponentNames: () => [] }),
    getLedCount: () => 0, getChannelPulseColor: () => [0, 0, 0],
    createSubdevice() {}, removeSubdevice() {}, setSubdeviceName() {}, setSubdeviceSize() {},
    setSubdeviceLeds() {}, setSubdeviceImageUrl() {}, setSubdeviceImage() {}, getCurrentSubdevices: () => [],
    color: () => [0, 0, 0], subdeviceColor: () => [0, 0, 0], getBrightness: () => 100,
    createColorArray: (len) => new Array(Math.max(0, len | 0) * 3).fill(0),
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
