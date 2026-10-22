// Shims for SignalRGB's built-in `@SignalRGB/*` modules, one per `//@module` block.
//@module @SignalRGB/Errors.js
export class ContextError extends Error {}
export const globalContext = { set() {}, Push() {}, Pop() {}, Shift() {}, Replace() {}, toString: () => "" };
export const Assert = {
  isOk(c, m) { if (!c) throw new Error(m ?? "Assertion failed"); },
  isEqual(a, b, m) { if (a !== b) throw new Error(m ?? `Expected ${a} === ${b}`); },
  softIsDefined() {}, // logs only in the original
  fail(m) { throw new Error(m ?? "Assert.fail"); },
  unreachable(m) { throw new Error(m ?? "unreachable"); },
};
//@module @SignalRGB/lcd
export default class LCD { constructor() {} static initialize() {} initialize() {} static shutdown() {} shutdown() {} static setImage() {} static getFrame() { return undefined; } getFrame() { return undefined; } setImage() {} }
export const lcd = new LCD();
//@module @SignalRGB/DeviceDiscovery
export default { Scan() {}, Add() {}, Remove() {}, devices: [], foundDevices: () => [], foundVirtualDevice() {}, foundNetworkDevice() {}, foundBluetoothDevice() {}, foundWiFiDevice() {} };
//@module @SignalRGB/permissions
export default { RequestPermission() {}, hasPermission: () => true, setCallback() {}, permissions: () => ({}), requestPermission() {} };
//@module @SignalRGB/systeminfo
export default globalThis.systeminfo;
//@module @SignalRGB/serial
export default { connect() {}, disconnect() {}, write() {}, read: () => [], isConnected: () => false, availablePorts: () => [], getDeviceInfo: () => ({}) };
