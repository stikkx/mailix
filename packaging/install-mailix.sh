#!/usr/bin/env bash
# install-mailix.sh — install or update Mailix from GitHub releases.
#
# Usage:
#   install-mailix.sh              install latest (or update, if already installed)
#   install-mailix.sh --check      only report installed vs. latest version
#   install-mailix.sh --force      install even if up to date
#   install-mailix.sh --dir DIR    appimage/tarball target dir (default ~/.local/share/mailix)
#
# Package selection:
#   Debian/Ubuntu (dpkg present)   -> .deb  (sudo dpkg -i, updates via re-run)
#   Arch (pacman present)          -> .deb? no — AppImage + desktop entry
#   anything else                  -> AppImage + desktop entry
#
# Version detection:
#   .deb install  : dpkg-query -W -f='${Version}' mailix
#   appimage/etc. : stamp file $STAMP written by this script
set -euo pipefail

REPO="stikkx/mailix"
API="https://api.github.com/repos/$REPO/releases/latest"
APPIMAGE_BIN_DEFAULT="$HOME/.local/share/mailix/mailix.AppImage"
BIN_LINK="$HOME/.local/bin/mailix"
STAMP="$HOME/.local/share/mailix/.installed-version"
DESKTOP_SRC="$HOME/.local/share/mailix/mailix.desktop"

check=0; force=0; dir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --check) check=1 ;;
    --force) force=1 ;;
    --dir) dir="${2:?}"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done

log()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!>\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- latest release from GitHub -------------------------------------------
# Requires the repo to be public (or a public fork/mirror) with a published
# release; anonymous API calls 404 otherwise.
manifest="$(curl -sS "$API" || true)"
if [ -z "$(printf '%s' "$manifest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)" ]; then
  if printf '%s' "$manifest" | grep -q '"Not Found"'; then
    die "GitHub reports $REPO as not found. The repository must be public and a release published before installs/updates work (see README 'Install')."
  fi
  die "Could not fetch latest release from $API — check your network."
fi
tag="$(printf '%s' "$manifest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
[ -n "$tag" ] || die "Could not fetch latest release from $API"
version="${tag#v}"   # asset names use the version without leading v

# --- installed version -----------------------------------------------------
installed=""
if command -v dpkg-query >/dev/null 2>&1 && dpkg-query -W -f='${Status}' mailix 2>/dev/null | grep -q "install ok installed"; then
  installed="$(dpkg-query -W -f='${Version}' mailix)"
  method="deb"
elif [ -f "$STAMP" ]; then
  installed="$(cat "$STAMP")"
  method="appimage"
elif command -v pacman >/dev/null 2>&1 && pacman -Q mailix >/dev/null 2>&1; then
  installed="$(pacman -Q mailix | awk '{print $2}')"
  method="pacman"
fi

log "Latest release: $tag"
log "Installed: ${installed:-none}"

if [ "$check" = 1 ]; then
  if [ "$installed" = "$version" ]; then echo "Up to date."; else echo "Update available: ${installed:-none} -> $version"; fi
  exit 0
fi

if [ "$installed" = "$version" ] && [ "$force" = 0 ]; then
  log "Already up to date. Use --force to reinstall."
  exit 0
fi

# --- download --------------------------------------------------------------
base="https://github.com/$REPO/releases/download/$tag"
log "Downloading $tag ..."
curl -sSfL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" || die "SHA256SUMS not found in release $tag"

fetch() { curl -sSfL -o "$tmp/$1" "$base/$1"; }
verify() {
  (cd "$tmp" && grep -w "$1" SHA256SUMS | sha256sum -c - >/dev/null 2>&1) \
    || die "Checksum mismatch for $1 — aborting."
}

# --- choose package --------------------------------------------------------
if command -v dpkg >/dev/null 2>&1; then
  deb="mailix_${version}_amd64.deb"
  fetch "$deb"; verify "$deb"
  log "Installing $deb (requires sudo) ..."
  sudo dpkg -i "$tmp/$deb" || sudo apt-get install -f -y
  log "Installed $version via apt/dpkg. Future updates: re-run this script."
  exit 0
fi

# AppImage for everything else
appimage="mailix-$version-x86_64-linux.AppImage"
target="${dir%/}"
[ -n "$target" ] || target="${APPIMAGE_BIN_DEFAULT%/*}"
fetch "$appimage"; verify "$appimage"

log "Installing AppImage to $target ..."
mkdir -p "$target" "$HOME/.local/bin"
mv "$tmp/$appimage" "$target/mailix.AppImage"
chmod +x "$target/mailix.AppImage"

# extract the .desktop file + icon the AppImage ships with
"$target/mailix.AppImage" --appimage-extract mailix.desktop >/dev/null 2>&1 \
  && { cp squashfs-root/mailix.desktop "$target/mailix.desktop"; rm -rf squashfs-root; } \
  || warn "Could not extract desktop file from AppImage."
if [ -f "$target/mailix.desktop" ]; then
  sed "s|^Exec=.*|Exec=$target/mailix.AppImage|; s|^TryExec=.*|TryExec=$target/mailix.AppImage|" \
    "$target/mailix.desktop" > "$HOME/.local/share/applications/mailix.desktop"
fi

ln -sf "$target/mailix.AppImage" "$BIN_LINK"
echo "$version" > "$STAMP"

log "Installed $version. Start with: mailix"
[ "${PATH//$HOME\/.local\/bin/}" != "$PATH" ] || warn "$HOME/.local/bin is not in PATH — add it to use 'mailix' from the terminal."
case "$method" in
  pacman) warn "'pacman -Q mailix' still reports the old version; consider removing that package to avoid two copies." ;;
esac
