#!/usr/bin/env bash
#
# get-svg — one-line installer.
#
#   macOS / Linux / Windows-Git-Bash:
#     curl -fsSL https://raw.githubusercontent.com/avdeshjadon/get-svg/main/install.sh | sh
#
#   Pin a specific version:
#     curl -fsSL .../install.sh | GET_SVG_VERSION=v0.1.0 sh
#     curl -fsSL .../install.sh | sh -s -- --dir "$HOME/bin"
#
# Installs the official release binary (SHA-256 verified) for your OS + CPU.
set -euo pipefail

REPO="avdeshjadon/get-svg"
BIN="get-svg"
VERSION="${GET_SVG_VERSION:-latest}"

DIR="${INSTALL_DIR:-}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --dir=*) DIR="${1#*=}"; shift ;;
    -h|--help) echo "usage: install.sh [--dir DIR]  (or set INSTALL_DIR / GET_SVG_VERSION)"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done
[ -n "$DIR" ] || DIR="${HOME}/.local/bin"

# --- platform detection ---------------------------------------------------
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
      x86_64|amd64)  TARGET="x86_64-apple-darwin" ;;
      *) echo "unsupported Apple CPU: $ARCH" >&2; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  Linux)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) echo "unsupported Linux CPU: $ARCH (only x86_64 CI builds)" >&2; exit 1 ;;
    esac
    EXT="tar.gz"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    case "$ARCH" in
      x86_64|amd64) TARGET="x86_64-pc-windows-msvc" ;;
      *) echo "unsupported Windows CPU: $ARCH" >&2; exit 1 ;;
    esac
    EXT="zip"
    ;;
  *)
    echo "unsupported OS: $OS" >&2
    exit 1
    ;;
esac

# --- resolve version (latest -> concrete tag) -----------------------------
if [ "$VERSION" = "latest" ]; then
  echo "> resolving latest release for $REPO ..." >&2
  VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
  if [ -z "$VERSION" ]; then
    echo "error: could not determine the latest release tag" >&2
    exit 1
  fi
fi

BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
ARTIFACT="$BIN-$TARGET.$EXT"

# --- download + verify ----------------------------------------------------
TMP="$(mktemp -d "${TMPDIR:-/tmp}/get-svg.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

echo "> downloading $ARTIFACT ($VERSION) ..." >&2
curl -fsSL -o "$TMP/$ARTIFACT" "$BASE_URL/$ARTIFACT"
curl -fsSL -o "$TMP/$ARTIFACT.sha256" "$BASE_URL/$ARTIFACT.sha256"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$TMP" && sha256sum "$1" | awk '{print $1}' )
  elif command -v shasum >/dev/null 2>&1; then
    ( cd "$TMP" && shasum -a 256 "$1" | awk '{print $1}' )
  else
    echo "error: no sha256 checksum tool (sha256sum/shasum) available" >&2
    exit 1
  fi
}

( cd "$TMP" && sha256_file "$ARTIFACT" >/dev/null 2>&1 ) || { echo "error: no checksum tool available" >&2; exit 1; }
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

BINARY="$(find "$EXTRACTED" -type f -name "$BIN" -o -type f -name "$BIN.exe" | head -n1)"
[ -n "$BINARY" ] || { echo "error: $(basename "$ARTIFACT") did not contain a $BIN binary" >&2; exit 1; }

# --- install ---------------------------------------------------------------
mkdir -p "$DIR"
install -m 755 "$BINARY" "$DIR/$BIN"
echo "> installed $DIR/$BIN ($VERSION)" >&2

if [ -x "$DIR/$BIN" ]; then
  "$DIR/$BIN" --version >/dev/null 2>&1 && echo "> $($DIR/$BIN --version)" >&2 \
    || echo "note: installed, but --version check failed ($OS)" >&2
fi

case ":$PATH:" in
  *":$DIR:"*) : ;;
  *) echo "> add to your PATH: export PATH=\"$DIR:\$PATH\"" >&2 ;;
esac

echo "$DIR/$BIN"