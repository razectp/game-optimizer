#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

project=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary="$project/target/release/game_optimizer"

if [[ ${1:-} == "--binary" && $# == 2 ]]; then
    binary=$(realpath -- "$2")
elif [[ $# != 0 ]]; then
    echo "Usage: $0 [--binary PATH]" >&2
    exit 64
else
    CARGO_TARGET_DIR="$project/target" cargo build --manifest-path "$project/Cargo.toml" \
        --locked --release --bin game_optimizer
fi

version=$(awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' "$project/Cargo.toml")
[[ -n "$version" && $("$binary" --version) == "game_optimizer $version" ]]
revision=$(tr -d '[:space:]' <"$project/installer/linux/revision")
package_version="$version-$revision"
architecture=$(dpkg --print-architecture)
case "$architecture" in
    amd64) machine="Advanced Micro Devices X86-64" ;;
    arm64) machine="AArch64" ;;
    *) echo "Unsupported Debian architecture: $architecture" >&2; exit 1 ;;
esac
readelf -h "$binary" | awk -F: -v expected="$machine" '
    /Machine:/ { gsub(/^[[:space:]]+/, "", $2); if ($2 == expected) found=1 }
    END { exit !found }
'
minimum_glibc=$(objdump -T "$binary" | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' |
    sed 's/^GLIBC_//' | sort -V | tail -n1)
[[ -n "$minimum_glibc" ]]

stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
chmod 755 "$stage"
install -d -m 755 "$stage/DEBIAN" "$stage/usr/bin"
install -Dm755 "$binary" "$stage/usr/lib/game-optimizer/game_optimizer"
install -Dm755 "$project/installer/linux/tunnel-helper" "$stage/usr/lib/game-optimizer/tunnel-helper"
ln -s ../lib/game-optimizer/game_optimizer "$stage/usr/bin/game_optimizer"
install -Dm644 "$project/installer/linux/game-optimizer.desktop" \
    "$stage/usr/share/applications/game-optimizer.desktop"
install -Dm644 "$project/assets/game-optimizer-icon.png" \
    "$stage/usr/share/icons/hicolor/256x256/apps/game-optimizer.png"
install -Dm644 "$project/installer/linux/org.gameoptimizer.tunnel.policy" \
    "$stage/usr/share/polkit-1/actions/org.gameoptimizer.tunnel.policy"
install -Dm644 "$project/installer/linux/50-game-optimizer-tunnel.rules" \
    "$stage/usr/share/polkit-1/rules.d/50-game-optimizer-tunnel.rules"

installed_size=$(du -sk "$stage" | cut -f1)
cat >"$stage/DEBIAN/control" <<CONTROL
Package: game-optimizer
Version: $package_version
Architecture: $architecture
Section: games
Priority: optional
Maintainer: Game Optimizer <cezar@cezar.com>
Homepage: https://github.com/razectp/game-optimizer
Installed-Size: $installed_size
Depends: libc6 (>= $minimum_glibc), libgcc-s1, libegl1, libgl1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libxcursor1, libxi6, libxrandr2, libwayland-client0, systemd, pkexec, polkitd (>= 122), iproute2, iputils-ping, nftables (>= 1.0.9), procps
Recommends: xdg-desktop-portal
Description: Game performance optimizer and game-only WireGuard tunnel
 Detects native and Proton games, offers X11 macros, and isolates the
 WireGuard connection for detected games from the rest of the session.
CONTROL
chmod 0644 "$stage/DEBIAN/control"

cat >"$stage/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -eu
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
fi
POSTINST
chmod 0755 "$stage/DEBIAN/postinst"

cat >"$stage/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -eu
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
fi
POSTRM
chmod 0755 "$stage/DEBIAN/postrm"

mkdir -p "$project/dist"
output="$project/dist/game-optimizer_${package_version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$stage" "$output"
chmod 0644 "$output"
