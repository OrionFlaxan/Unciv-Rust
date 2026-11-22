# Unciv-Rust — Pure-Performance Rust-Accelerated Unciv

![](/extraImages/GithubPreviewImage.jpg)

[![Latest Release](https://img.shields.io/github/v/release/OrionFlaxan/Unciv-Rust?include_prereleases&label=Download%20Release&logo=github&color=2ea44f)](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest)
[![Android ABIs](https://img.shields.io/badge/Android-arm64--v8a%20%7C%20armeabi--v7a%20%7C%20x86__64%20%7C%20x86-3DDC84?logo=android&logoColor=white)](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest)
[![Rust Native Engine](https://img.shields.io/badge/Native%20Engine-Rust%20%2B%20JNI-DEA584?logo=rust&logoColor=white)](#whats-accelerated)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-brightgreen.svg)](LICENSE)

## What is Unciv-Rust?

**Unciv-Rust** is a pure-performance, Rust-accelerated fork of [Unciv](https://github.com/yairm210/Unciv) (the open-source Civilization V remake for Android & Desktop) built so **any Android phone can keep playing smoothly past 300+ turns on Large and Huge maps**.

- **12× faster late-game turns (300+ turns on Huge 22-player maps)** — turning 34–56 second late-game turn freezes into ~2.9 second turns.
- **Zero Original UI Changes** — `core/src/com/unciv/ui/` is 100% untouched upstream Unciv.
- **100% Vanilla Rules, Mods & Save Compatibility** — saves load identically in vanilla Unciv and Unciv-Rust; exotic mod uniques and unsupported platforms transparently fall back to pure Kotlin.
- **All 4 Android ABIs Built In** — ships stripped `libunciv_native.so` binaries for `arm64-v8a`, `armeabi-v7a`, `x86_64`, and `x86` (Android 5.0 / API 21+).

---

## Download

| Platform | Direct Download | Notes |
|---|---|---|
| **Android (Phone / Tablet)** | [**Download `Unciv-Rust-Android.apk`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Android.apk) | Universal APK (Android 5.0+ / API 21+) with all 4 Rust native ABIs |
| **Desktop (Linux / Windows / macOS)** | [**Download `Unciv-Rust-Desktop.jar`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Desktop.jar) | Run with `java -jar Unciv-Rust-Desktop.jar` (bundles `linux-x86_64` native lib; transparent Kotlin fallback on other OSes) |
| **All Releases** | [**GitHub Releases**](https://github.com/OrionFlaxan/Unciv-Rust/releases) | Browse all versions, checksums, and release notes |

### Installing on Android
1. Tap [**Download `Unciv-Rust-Android.apk`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Android.apk) on your phone (or open the [Releases page](https://github.com/OrionFlaxan/Unciv-Rust/releases)).
2. Open the downloaded `.apk` file and allow **Install unknown apps** if prompted by Android.
3. **Already have Unciv from Google Play or F-Droid?** Back up your saves first and uninstall the store version before installing this APK, since this build uses Unciv's standard `com.unciv.app` package ID with a different signing key.

---

## Performance Benchmarks

### Huge Map — 22 AI Civilizations, Seed 42, 350 Turns

Measured head-to-head via `OjhBenchmark` (4,921 tiles, ~1,200+ units by Turn 350) against unmodified upstream Unciv (`0d2032e`). Unmodified Unciv timed out after Turn 325 (33.3 minutes); **Unciv-Rust finished all 350 turns in 9.16 minutes**.

| Turn Bucket | Vanilla Unciv Mean | Vanilla Max | Unciv-Rust Mean | Unciv-Rust P95 | Unciv-Rust Max | Speedup (Mean) |
|---|---:|---:|---:|---:|---:|---:|
| **Boot + Mapgen** | 3.595s | — | **2.678s** | — | — | **1.34×** |
| **Turns 1–50** | 0.396s | 1.346s | **0.272s** | 0.416s | 0.877s | **1.46×** |
| **Turns 51–100** | 1.155s | 2.185s | **0.641s** | 0.881s | 1.103s | **1.80×** |
| **Turns 101–150** | 2.775s | 4.207s | **1.121s** | 1.249s | 1.407s | **2.47×** |
| **Turns 151–200** | 3.994s | 7.113s | **1.326s** | 1.479s | 1.600s | **3.01×** |
| **Turns 201–250** | 6.196s | 14.929s | **1.939s** | 2.244s | 3.660s | **3.19×** |
| **Turns 251–300** | 8.188s | 21.170s | **2.618s** | 2.936s | 3.015s | **3.13×** |
| **Turns 301–325** | **34.492s** | **56.442s** | **2.861s** | **3.344s** | **3.483s** | **12.06×** |
| **Turns 326–350** | *(timeout)* | *(timeout)* | **3.283s** | **4.043s** | **4.127s** | — |

- **Turns 1–325 Total Wall-Clock:** `33.29 min` → **`7.79 min`** (**4.27× overall speedup**).
- **Peak Late-Game Spike (Turns 301–325):** `56.44s` → **`3.48s`** (**16.2× lower peak latency**).
- **Large Map (16 AI Civs, Seed 42, 350 Turns):** Completed all 350 turns in **4.41 min** (**0.755s/turn** overall mean; **1.408s/turn** mean across Turns 301–350).
- **Save / Load Compression (4.51 MB Turn-350 Save JSON):** **2.74× faster save** (`32.75 ms → 11.96 ms`) and **2.75× faster load** (`13.21 ms → 4.81 ms`), 100% wire-compatible with vanilla saves.

---

## What's Accelerated?

### Rust Native Simulation Engine (`rust/unciv-native/`)
- **Flat SoA World Mirror & Zero-Alloc Scratch Arenas (`world.rs`, `scratch.rs`):** Compact Structure-of-Arrays representation of tiles, neighbors, unit occupancy, diplomacy matrix, and `u64` vision/exploration bitsets with generation-stamped reusable BFS/A\*/Dijkstra scratch arenas (zero heap allocations per query).
- **Pathfinding & Movement (`pathing.rs`):** Exact fixed-point (`1/60` FPM) movement costs, Zone of Control, embark/disembark, retroactive Turn-0 pause, `compute_distance_within_turn`, `find_shortest_path`, and `batch_first_reachable`.
- **Incremental Vision & Fog-of-War (`vision.rs`):** Exact hex line-of-sight raycasting and O(sight²) reference-counted civ vision updates.
- **AI Threat Field, Combat & Exploration (`ai.rs`):** Multi-source BFS enemy-distance threat field, batched LOS attack-pair discovery (`TargetHelper`), and native explore-tile filtering/scoring.
- **Trade & Road Connectivity (`connectivity.rs`):** Native `CapitalConnectionsFinder` (road/rail/harbor/river BFS), `WorkerAutomation` road A\*, and `MotivationToAttackAutomation` attack-path evaluation.
- **Map Generation & Save Compression (`mapgen.rs`, `compression.rs`):** Bit-exact batched Perlin noise, iterative continent flood-fill, and native `zlib-rs` + `base64` save compression.
- **Thin Primitive-Array JNI Bridge (`jni_bridge.rs`, `core/src/com/unciv/nativeengine/`):** Zero Java object allocations across JNI, incremental dirty patching, and automatic fallback for exotic mods.

### Rules-Preserving Kotlin Hot-Path Optimizations
- Lazy `Ruleset.hasResourceLocalUnique` `EnumSet` cache avoiding 36-tile scans in `City.forEachLocalMatchingUnique`.
- Precomputed `Unique.hasMultiplierModifiers` O(1) fast path and zero-allocation indexed `UniqueMap` lookups.
- Hoisted `Automation.CityWorkRankContext` + primitive `rankStatsForCityWorkFast` in `CityPopulationManager`.
- Per-civ `LongArray` exploration bitsets (preserving `Tile.exploredBy` JSON) and zero-allocation `TileStatFunctions.getTileStatsFast`.
- Schwartzian unit sorting in `NextTurnAutomation` and first-waypoint prefix caching in `PathfindingCache`.

---

## Building from Source

### Prerequisites
- **JDK 21**
- **Rust (`rustup` + `cargo`)**
- For Android builds: **Android SDK + NDK** (`ANDROID_HOME` / `ANDROID_NDK_HOME`) and Rust Android targets:
  ```bash
  rustup target add \
    aarch64-linux-android \
    armv7-linux-androideabi \
    x86_64-linux-android \
    i686-linux-android
  ```

### Build & Test Commands
```bash
# Run Rust unit tests (release)
cargo test --manifest-path rust/Cargo.toml --release

# Run full Unciv JUnit test suite (auto-builds desktop Rust library)
./gradlew :tests:test

# Build Desktop JAR (bundles native library)
./gradlew :desktop:dist

# Build Android APK for all 4 ABIs (auto-runs rust/build-android.sh)
./gradlew :android:assembleDebug
```

### Useful Flags
- `-PskipRustBuild=true` — skip Cargo compilation in Gradle when iterating only on Kotlin code.
- `-Dunciv.disableNative=true` — disable the Rust native library at runtime to force the pure-Kotlin fallback path.

---

## Upstream Unciv, Community & Licensing

Unciv-Rust is based on the amazing open-source [yairm210/Unciv](https://github.com/yairm210/Unciv) project built with [LibGDX](https://github.com/libgdx/libgdx).

- **Official Unciv:** [GitHub](https://github.com/yairm210/Unciv) · [Google Play](https://play.google.com/store/apps/details?id=com.unciv.app) · [F-Droid](https://f-droid.org/en/packages/com.unciv.app/) · [Discord](https://discord.gg/bjrB4Xw) · [Wiki](https://yairm210.github.io/Unciv/)
- **Gameplay Help:** All tutorials are available in-game under *Menu → Civilopedia → Tutorials* and on the [Civ V Wiki](https://civilization.fandom.com/wiki/).

### Licensing & Credits
Unciv and Unciv-Rust source code is available under the **Mozilla Public License 2.0** (see [LICENSE](LICENSE)), except where otherwise noted.
`TextureArraySpriteBatch.java`, forked from [carlislefox/libgdx-texture-array-batch](https://github.com/carlislefox/libgdx-texture-array-batch), is available under CC0 1.0 Universal (see [LICENSE-TextureArraySpriteBatch](LICENSE-TextureArraySpriteBatch)).
Media files are authored by many contributors under CC BY-SA 4.0, CC BY 3.0/4.0, CC0, and public domain terms (see [docs/Credits.md](docs/Credits.md)).
