#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Locate Android NDK
NDK_DIR="${ANDROID_NDK_HOME:-}"
if [[ -z "$NDK_DIR" ]]; then
  SDK_DIR="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}"
  if [[ -d "$SDK_DIR/ndk" ]]; then
    NDK_DIR="$(ls -d "$SDK_DIR"/ndk/* 2>/dev/null | sort -V | tail -n 1 || true)"
  fi
fi

if [[ -z "$NDK_DIR" || ! -d "$NDK_DIR" ]]; then
  echo "ERROR: Android NDK not found. Set ANDROID_NDK_HOME or install NDK under \$ANDROID_HOME/ndk/." >&2
  exit 1
fi

HOST_TAG=""
for candidate in linux-x86_64 darwin-x86_64 darwin-arm64 windows-x86_64; do
  if [[ -d "$NDK_DIR/toolchains/llvm/prebuilt/$candidate/bin" ]]; then
    HOST_TAG="$candidate"
    break
  fi
done
if [[ -z "$HOST_TAG" ]]; then
  echo "ERROR: Could not find LLVM prebuilt toolchain inside $NDK_DIR" >&2
  exit 1
fi

TOOLCHAIN_BIN="$NDK_DIR/toolchains/llvm/prebuilt/$HOST_TAG/bin"
API_LEVEL=21

build_abi() {
  local rust_target="$1"
  local abi_dir="$2"
  local clang_prefix="$3"
  local env_target_upper
  env_target_upper="$(echo "$rust_target" | tr '[:lower:]-' '[:upper:]_')"
  local env_target_under
  env_target_under="$(echo "$rust_target" | tr '-' '_')"

  local linker="$TOOLCHAIN_BIN/${clang_prefix}${API_LEVEL}-clang"
  local ar="$TOOLCHAIN_BIN/llvm-ar"

  if [[ ! -x "$linker" ]]; then
    echo "ERROR: Missing NDK linker: $linker" >&2
    exit 1
  fi

  echo "=== Building unciv-native for Android $abi_dir ($rust_target, API $API_LEVEL) ==="
  env \
    "CARGO_TARGET_${env_target_upper}_LINKER=$linker" \
    "CC_${env_target_under}=$linker" \
    "AR_${env_target_under}=$ar" \
    cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --release --target "$rust_target"

  local out_dir="$ROOT_DIR/android/libs/$abi_dir"
  mkdir -p "$out_dir"
  cp -f "$SCRIPT_DIR/target/$rust_target/release/libunciv_native.so" "$out_dir/libunciv_native.so"
  echo "Staged $out_dir/libunciv_native.so ($(du -h "$out_dir/libunciv_native.so" | cut -f1))"
}

build_abi "aarch64-linux-android"    "arm64-v8a"   "aarch64-linux-android"
build_abi "armv7-linux-androideabi"  "armeabi-v7a" "armv7a-linux-androideabi"
build_abi "x86_64-linux-android"     "x86_64"      "x86_64-linux-android"
build_abi "i686-linux-android"       "x86"         "i686-linux-android"

echo "All 4 Android ABIs built successfully."
