#!/bin/sh
#
# get-svg — one-line installer (POSIX sh: macOS, Linux, Windows Git Bash).
#
#   macOS / Linux / Windows-Git-Bash:
#     curl -fsSL https://raw.githubusercontent.com/avdeshjadon/get-svg/main/install.sh | sh
#
#   Install somewhere else:
#     curl -fsSL .../install.sh | sh -s -- --dir "$HOME/bin"
#
# Always installs the latest release automatically (or pin with
# GET_SVG_VERSION, e.g. GET_SVG_VERSION=v0.1.0). Both `get-svg` and the
# `getsvg` alias are installed to ~/.local/bin (or --dir), SHA-256 verified.
set -eu

REPO="avdeshjadon/get-svg"
BIN="get-svg"
VERSION="${GET_SVG_VERSION:-latest}"

DIR="${INSTALL_DIR:-}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --dir=*) DIR="${1#*=}"; shift ;;
    -h|--help) echo "usage: install.sh [--dir DIR]  (or set INSTALL_DIR / GET_SVG_VERSION)"; exit 0 ;;
    *) echo "install.sh: unknown argument: $1" >&2; exit 1 ;;
  esac
done
[ -n "$DIR" ] || DIR="${HOME}/.local/bin"

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }

# --- platform detection ---------------------------------------------------
ARCH="$(uname -m 2>/dev/null || echo unknown)"
OS="$(uname -s 2>/dev/null || echo unknown)"

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64|amd64)  TARGET="x86_64-apple-darwin" ;;
      *) echo "error: unsupported Apple CPU: $ARCH" >&2; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  Linux)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) echo "error: unsupported Linux CPU: $ARCH (only x86_64 builds)" >&2; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-pc-windows-msvc" ;;
      *) echo "error: unsupported Windows CPU: $ARCH" >&2; exit 1 ;;
    esac
    EXT="zip"
    ;;
  *)
    echo "error: unsupported OS: $OS" >&2
    exit 1
    ;;
esac

# --- resolve latest release tag -------------------------------------------
if [ "$VERSION" = "latest" ]; then
  echo "> resolving latest release ..." >&2
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | sed -n '1p')"
  if [ -z "$VERSION" ]; then
    echo "error: could not determine the latest release tag" >&2
    exit 1
  fi
fi

BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
ARTIFACT="$BIN-$TARGET.$EXT"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/get-svg.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT HUP INT TERM

echo "> downloading $ARTIFACT ($VERSION) ..." >&2
curl -fsSL -o "$TMP/$ARTIFACT" "$BASE_URL/$ARTIFACT"
curl -fsSL -o "$TMP/$ARTIFACT.sha256" "$BASE_URL/$ARTIFACT.sha256"

# --- verify SHA-256 --------------------------------------------------------
sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$TMP" && sha256sum "$1" | awk '{print $1}' )
  elif command -v shasum >/dev/null 2>&1; then
    ( cd "$TMP" && shasum -a 256 "$1" | awk '{print $1}' )
  else
    echo "error: no SHA-256 tool (sha256sum / shasum) available" >&2
    exit 1
  fi
}
EXPECTED="$(awk '{print $1}' "$TMP/$ARTIFACT.sha256")"
ACTUAL="$(sha256_file "$ARTIFACT")"
if [ "$EXPECTED" != "$ACTUAL" ]; then
  echo "error: checksum mismatch for $ARTIFACT" >&2
  exit 1
fi
echo "> checksum verified ($ACTUAL)" >&2

# --- extract --------------------------------------------------------------
EXTRACTED="$TMP/extracted"
mkdir -p "$EXTRACTED"
if [ "$EXT" = "zip" ]; then
  command -v unzip >/dev/null 2>&1 || { echo "error: 'unzip' is required on Windows Git Bash" >&2; exit 1; }
  ( cd "$EXTRACTED" && unzip -q "$TMP/$ARTIFACT" )
else
  tar -xzf "$TMP/$ARTIFACT" -C "$EXTRACTED"
fi

# --- install ---------------------------------------------------------------
mkdir -p "$DIR"
for NAME in "$BIN" "getsvg"; do
  BINPATH="$(find "$EXTRACTED" -type f -name "$NAME" -o -type f -name "$NAME.exe" 2>/dev/null | sed -n '1p')"
  [ -n "$BINPATH" ] || { echo "error: archive did not contain a $NAME binary" >&2; exit 1; }
  install -m 755 "$BINPATH" "$DIR/$NAME"
  echo "> installed $DIR/$NAME ($VERSION)" >&2
done

if [ -x "$DIR/$BIN" ]; then
  "$DIR/$BIN" --version >/dev/null 2>&1 && echo "> $($DIR/$BIN --version)" >&2 || true
fi

case ":$PATH:" in
  *":$DIR:"*) : ;;
  *) echo "> add $DIR to your PATH, e.g.: export PATH=\"$DIR:\$PATH\"" >&2 ;;
esac

echo "> done. Try:  getsvg --help   or just:  getsvg"
echo "$DIR"