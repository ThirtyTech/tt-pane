#!/bin/sh
# Renders the macOS packaging artwork from its SVG masters:
#
#   icons/<size>.png               from assets/tt-pane.svg, for the .icns
#   dmg-background.png / @2x.png   from dmg-background.svg, for the .dmg
#
# The PNGs are committed, so package.sh and the release job need no SVG
# renderer; run this again after changing either SVG. Needs rsvg-convert
# (librsvg: `brew install librsvg`).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
command -v rsvg-convert >/dev/null 2>&1 || { echo "render-artwork: rsvg-convert is required" >&2; exit 1; }
mkdir -p "$here/icons"
for size in 16 32 64 128 256 512 1024; do
    rsvg-convert -w "$size" -h "$size" "$root/assets/tt-pane.svg" -o "$here/icons/$size.png"
done
rsvg-convert -w 660 -h 420 "$here/dmg-background.svg" -o "$here/dmg-background.png"
rsvg-convert -w 1320 -h 840 "$here/dmg-background.svg" -o "$here/dmg-background@2x.png"
echo "Rendered icons and DMG background into $here"
