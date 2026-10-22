# SignalRGB plugin host API

Source: usage tallied across the 462 plugin scripts bundled with SignalRGB
2.5.80 (`Plugins/`). Counts are call sites across all plugins (>=3 shown);
semantics are inferred from usage, and are confirmed only where Refract RGB
implements and exercises them.

## Plugin module exports (the contract a plugin fulfils)

Required by nearly every plugin: `Name`, `Publisher`, `Initialize`, `Render`,
`Shutdown`, `Size`, `ImageUrl`, `DeviceType`, `ControllableParameters`.
Hardware match: `VendorId`, `ProductId`, `Validate(endpoint)`.
Layout: `LedNames`, `LedPositions`, `DefaultPosition`, `DefaultScale`.
Optional: `Documentation`, `Type`, `ConflictingProcesses`, `SubdeviceController`,
`DefaultComponentBrand`, `SupportsFanControl`, `Scan`, `DeviceMessages`,
`BrandGPUList`, `UsesCorsairMutex`, and `on<Param>Changed` setting callbacks
(e.g. `ondpi`, `onPollingRateChanged`).

## Global `device` object

- USB/HID I/O: `write`, `read`, `send_report`, `get_report`, `input_report`,
  `bulk_transfer`, `control_transfer`, `set_endpoint`, `getHidEndpoints`,
  `getHidInfo`, `clearReadBuffer`, `getLastReadSize`, `flush`, `setProtocol`
- Flow/diagnostics: `log`, `pause`, `notify`, `setFrameRateTarget`
- Identity/layout: `setName`, `setSize`, `setControllableLeds`, `SetLedLimit`,
  `getLedCount`, `productId`, `getDeviceInfo`, `channel`, `addChannel`,
  `removeChannel`, `getChannelNames`, `getChannelPulseColor`, `setImageFromUrl`
- Colour: `color`, `createColorArray`, `subdeviceColor`, `getBrightness`
- Subdevices: `createSubdevice`, `removeSubdevice`, `setSubdeviceName`,
  `setSubdeviceSize`, `setSubdeviceLeds`, `setSubdeviceImageUrl`,
  `setSubdeviceImage`, `SetIsSubdeviceController`
- Fans/sensors: `createFanControl`, `removeFanControl`, `fanControlDisabled`,
  `setRPM`, `getFanlevel`, `getNormalizedFanlevel`, `createTemperatureSensor`,
  `removeTemperatureSensor`, `SetTemperature`
- Properties: `addProperty`, `removeProperty`, `getProperty`, `setUserProperty`,
  `addFeature`
- Rarely used (likely deprecated): `readTimeout`, `repollName/Leds/Size`,
  `getMotherboardName`, `SupportsSubdevices`

## Other globals

- `serial`: `connect`, `disconnect`, `write`, `isConnected`, `availablePorts`,
  `getDeviceInfo`
- `hid`: `write`
- `systeminfo`: `GetMotherboardInfo`, `GetRamInfo`, `GetBiosInfo`
- Network (UDP/HTTP/WebSocket) and SMBus/I2C access is not visible in these
  tallies; check `.ts` plugins.

## Porting notes

- The `device` I/O group maps onto hidapi/libusb. `serial` maps onto termios.
- `systeminfo` maps onto `/sys/class/dmi/id` and `dmidecode`.
- Unresolved: SMBus/RAM-RGB plugins, `Corsair` mutex semantics, event/
  settings plumbing, and how `Render` output is scheduled.
