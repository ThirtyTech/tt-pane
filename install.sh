#!/bin/sh
# Installs tt-pane from its GitHub releases on Linux (x64) and macOS
# (Apple Silicon):
#
#   curl -fsSL https://github.com/ThirtyTech/tt-pane/releases/latest/download/install.sh | sh
#
# It puts tt-pane in <prefix>/bin. On Linux it also adds an app launcher,
# dev.ttpane.TtPane.desktop in <prefix>/share/applications/, and its icon in
# <prefix>/share/icons/hicolor/. To remove them again:
#
#   curl -fsSL https://github.com/ThirtyTech/tt-pane/releases/latest/download/install.sh | sh -s -- --uninstall
#
# Environment:
#   TTPANE_VERSION  tag to install, such as v0.2.0 (default: latest release)
#   TTPANE_PREFIX   where to install (default: ~/.local). For every user, use
#                   /usr/local and run the script with sudo.

set -eu

repo="ThirtyTech/tt-pane"
prefix="${TTPANE_PREFIX:-$HOME/.local}"
uninstall=0

err() {
    echo "tt-pane install: $*" >&2
    exit 1
}

for arg in "$@"; do
    case "$arg" in
        --uninstall) uninstall=1 ;;
        *) err "unknown option: $arg (the only one is --uninstall)" ;;
    esac
done

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) err "unsupported OS: $(uname -s). On Windows, run the -setup.exe from https://github.com/$repo/releases" ;;
esac

bin="$prefix/bin/tt-pane"
desktop="$prefix/share/applications/dev.ttpane.TtPane.desktop"
icon="$prefix/share/icons/hicolor/256x256/apps/tt-pane.png"

# Tells the desktop about a new or removed launcher and icon. Desktops that
# watch these folders don't need it; the tools may not be installed.
refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$prefix/share/applications" 2>/dev/null || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$prefix/share/icons/hicolor/index.theme" ]; then
        gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2>/dev/null || true
    fi
}

if [ "$uninstall" = 1 ]; then
    removed=0
    for f in "$bin" "$desktop" "$icon"; do
        if [ -e "$f" ]; then
            rm -f "$f"
            removed=1
        fi
    done
    [ "$removed" = 1 ] || err "tt-pane isn't installed in $prefix"
    [ "$os" = linux ] && refresh
    echo "Removed tt-pane from $prefix. Your configuration is kept."
    echo "If tt-pane is running, quit it from its tray menu."
    exit 0
fi

command -v curl >/dev/null 2>&1 || err "curl is required"
command -v tar >/dev/null 2>&1 || err "tar is required"

case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) err "unsupported architecture: $(uname -m)" ;;
esac

if [ "$os" = macos ] && [ "$arch" = x64 ]; then
    err "Intel Macs aren't supported; only Apple Silicon builds are published"
fi
if [ "$os" = linux ] && [ "$arch" = arm64 ]; then
    err "Linux arm64 isn't supported yet; only x64 builds are published"
fi

if [ -n "${TTPANE_VERSION:-}" ]; then
    version="$TTPANE_VERSION"
else
    # /releases/latest redirects to /releases/tag/<version>.
    version=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest")
    version="${version##*/}"
    [ -n "$version" ] && [ "$version" != latest ] || err "couldn't find the latest release"
fi

name="tt-pane-$version-$os-$arch"
base="https://github.com/$repo/releases/download/$version"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "Downloading tt-pane $version ($os-$arch)"
curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz" || err "download failed: $base/$name.tar.gz"
curl -fsSL -o "$tmp/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || err "couldn't download SHA256SUMS.txt"

expected=$(grep " $name.tar.gz\$" "$tmp/SHA256SUMS.txt" | cut -d' ' -f1)
[ -n "$expected" ] || err "no checksum for $name.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)
fi
[ "$expected" = "$actual" ] || err "checksum mismatch for $name.tar.gz"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
src="$tmp/$name"
[ -f "$src/tt-pane" ] || err "$name.tar.gz has no tt-pane"

mkdir -p "$prefix/bin" || err "can't create $prefix/bin"
[ -w "$prefix/bin" ] || err "can't write to $prefix/bin; run the script with sudo, or set TTPANE_PREFIX"
# Install through a temp name and rename, so a running tt-pane isn't
# overwritten in place.
cp "$src/tt-pane" "$bin.new"
chmod 755 "$bin.new"
mv -f "$bin.new" "$bin"
echo "Installed tt-pane $version to $bin"

if [ "$os" = linux ]; then
    mkdir -p "$(dirname "$desktop")"
    icon_line=""
    # Releases before the launcher was added don't carry the icon.
    if [ -f "$src/tt-pane.png" ]; then
        mkdir -p "$(dirname "$icon")"
        cp "$src/tt-pane.png" "$icon.new"
        mv -f "$icon.new" "$icon"
        icon_line="Icon=tt-pane"
    fi
    # The launcher starts tt-pane by its full path, so it works whether or
    # not <prefix>/bin is on the session's PATH. The file name matches the
    # app id tt-pane gives its windows, so Wayland desktops show its icon.
    {
        echo '[Desktop Entry]'
        echo 'Type=Application'
        echo 'Name=tt-pane'
        echo 'Comment=Place windows with keyboard shortcuts'
        echo "Exec=\"$bin\""
        [ -n "$icon_line" ] && echo "$icon_line"
        echo 'Terminal=false'
        echo 'Categories=Utility;'
        echo 'StartupWMClass=dev.ttpane.TtPane'
    } > "$desktop.new"
    mv -f "$desktop.new" "$desktop"
    refresh
    echo "Start it from your app launcher (tt-pane). It starts in the system tray."

    # The libraries tt-pane links. The ones it loads when it starts (Vulkan,
    # Wayland) don't show here.
    if command -v ldd >/dev/null 2>&1; then
        missing=$(ldd "$bin" 2>/dev/null | awk '/not found/ { printf "%s%s", sep, $1; sep = " " }')
        if [ -n "$missing" ]; then
            echo "Warning: tt-pane needs libraries that aren't installed: $missing"
            echo "  See https://thirtytech.github.io/tt-pane/platforms/ for the packages."
        fi
    fi
else
    # Downloads through a browser are quarantined; this one usually isn't,
    # but clear it in case so Gatekeeper doesn't block the unsigned binary.
    xattr -d com.apple.quarantine "$bin" 2>/dev/null || true
    echo "Run $bin to start it in the menu bar. Give it Accessibility access in"
    echo "System Settings → Privacy & Security → Accessibility. An upgrade replaces the"
    echo "unsigned binary, so macOS can ask for that access again."
fi

echo "If an older tt-pane is running, quit it from its tray menu and start it again."
