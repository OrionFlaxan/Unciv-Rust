#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  linux*)  LIB_NAME="libunciv_native.so"; OS_TAG="linux" ;;
  darwin*) LIB_NAME="libunciv_native.dylib"; OS_TAG="macos" ;;
  msys*|mingw*|cygwin*) LIB_NAME="unciv_native.dll"; OS_TAG="windows" ;;
  *) echo "Unsupported host OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH_TAG="x86_64" ;;
  aarch64|arm64) ARCH_TAG="arm64" ;;
  *) ARCH_TAG="$ARCH" ;;
esac

cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --release

OUT_DIR="$ROOT_DIR/core/build/rust-natives/${OS_TAG}-${ARCH_TAG}"
mkdir -p "$OUT_DIR"
cp -f "$SCRIPT_DIR/target/release/$LIB_NAME" "$OUT_DIR/$LIB_NAME"
echo "Staged desktop native library -> $OUT_DIR/$LIB_NAME"
