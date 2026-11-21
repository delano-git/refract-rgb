#!/usr/bin/env bash
# Build a .deb and a release tarball from the current tree.
#
#   contrib/build-deb.sh            # writes dist/refract-rgb_<version>_<arch>.deb and .tar.gz
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(dpkg --print-architecture)"
PKG="refract-rgb_${VERSION}_${ARCH}"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
OUT="$ROOT/dist"
mkdir -p "$OUT"

cargo build --release --locked
T="$STAGE/$PKG"

install -Dm755 target/release/refract "$T/usr/bin/refract"
strip "$T/usr/bin/refract"
install -Dm644 contrib/70-refract-rgb.rules "$T/usr/lib/udev/rules.d/70-refract-rgb.rules"
install -Dm644 contrib/refract-rgb.desktop "$T/usr/share/applications/refract-rgb.desktop"
install -Dm644 assets/logo.svg "$T/usr/share/icons/hicolor/scalable/apps/refract-rgb.svg"
install -Dm644 LICENSE "$T/usr/share/doc/refract-rgb/copyright"
install -Dm644 README.md "$T/usr/share/doc/refract-rgb/README.md"
install -Dm644 /dev/stdin "$T/usr/lib/modules-load.d/refract-rgb.conf" <<< "i2c-dev"
install -Dm644 /dev/stdin "$T/usr/lib/systemd/user/refract-rgb.service" <<'U'
[Unit]
Description=Refract RGB lighting
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/refract daemon
Restart=on-failure
RestartSec=3

[Install]
WantedBy=graphical-session.target
U

# Binary needs glibc 2.39 (Ubuntu 24.04 or newer).
GLIBC="$(objdump -T target/release/refract | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 | sed 's/GLIBC_//')"
SIZE="$(du -sk "$T/usr" | cut -f1)"
mkdir -p "$T/DEBIAN"
cat > "$T/DEBIAN/control" <<C
Package: refract-rgb
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Depends: libc6 (>= $GLIBC), libgcc-s1
Recommends: chromium | chromium-browser | google-chrome-stable
Maintainer: Delano <31388690+delano-git@users.noreply.github.com>
Homepage: https://github.com/delano-git/refract-rgb
Description: Lighting control for Linux that runs SignalRGB device plugins
 Refract RGB drives motherboard headers, DDR5 memory and USB peripherals
 from one background service with a tray icon and a web interface.
 It does not include SignalRGB's plugins or effects; see the README.
C
cat > "$T/DEBIAN/postinst" <<'P'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
  udevadm control --reload 2>/dev/null || true
  udevadm trigger --subsystem-match=hidraw --action=add 2>/dev/null || true
  udevadm trigger --subsystem-match=i2c-dev --action=add 2>/dev/null || true
  modprobe i2c-dev 2>/dev/null || true
  gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
  update-desktop-database -q 2>/dev/null || true
  cat <<'M'

Refract RGB installed. Next steps, as your own user:
  sudo usermod -aG plugdev "$USER"     # then log out and back in once
  systemctl --user enable --now refract-rgb.service
  REFRACT_PLUGINS=/path/to/SignalRGB/Plugins refract install-service   # to set the plugin folder
Then open http://127.0.0.1:7878/

M
fi
P
cat > "$T/DEBIAN/postrm" <<'P'
#!/bin/sh
set -e
udevadm control --reload 2>/dev/null || true
P
chmod 755 "$T/DEBIAN/postinst" "$T/DEBIAN/postrm"

dpkg-deb --root-owner-group --build "$T" "$OUT/$PKG.deb" >/dev/null

TAR="refract-rgb-${VERSION}-linux-${ARCH}"
mkdir -p "$STAGE/$TAR"
cp target/release/refract LICENSE README.md "$STAGE/$TAR/"
cp -r contrib "$STAGE/$TAR/contrib"
tar -C "$STAGE" -czf "$OUT/$TAR.tar.gz" "$TAR"

(cd "$OUT" && sha256sum "$PKG.deb" "$TAR.tar.gz" > SHA256SUMS)
echo "built:"; ls -l "$OUT"
