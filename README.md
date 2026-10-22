# refract-rgb

Lighting control for Linux that runs SignalRGB device plugins and effects.

Goal: a Linux-native RGB controller that can run SignalRGB-format
Lightscript effects (HTML/JS canvas) and device plugins.

Vendor binaries are not committed; see `.gitignore`.

## Status

Rust host (`src/`): QuickJS plugin runtime, hidraw access via sysfs (no libudev),
SignalRGB `device` API and `@SignalRGB/*` module shims, Linux DMI in place of WMI.

    cargo build
    ./target/debug/refract list
    ./target/debug/refract info <plugin.js>
    ./target/debug/refract run <plugin.js> [--frames N]   # dry run, no hardware writes
    ./target/debug/refract run <plugin.js> --live         # sends bytes to the device

Plugins are not included; point the tool at a SignalRGB `Plugins/` directory.

## Lighting everything

    refract apply '#00ff40' [--plugins DIR] [--dry-run]

Runs every plugin whose VID/PID matches a connected device, plus ENE DDR5 RAM
over SMBus. Plugin dir: `--plugins`, `$REFRACT_PLUGINS`, or the extracted tree.
Setup: `sudo cp contrib/70-refract-rgb.rules /etc/udev/rules.d/`, reload udev,
and be in `plugdev`. DRAM writes are volatile and never touch SPD space.
