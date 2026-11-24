# Unciv-Rust — Multi-Threaded Rust-Accelerated Unciv

![](/extraImages/GithubPreviewImage.jpg)

[![Latest Release](https://img.shields.io/github/v/release/OrionFlaxan/Unciv-Rust?include_prereleases&label=Download%20Release&logo=github&color=2ea44f)](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest)
[![Android ABIs](https://img.shields.io/badge/Android-arm64--v8a%20%7C%20armeabi--v7a%20%7C%20x86__64%20%7C%20x86-3DDC84?logo=android&logoColor=white)](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest)
[![Rust Native Engine](https://img.shields.io/badge/Native%20Engine-Rust%20%2B%20Rayon%20%2B%20JNI-DEA584?logo=rust&logoColor=white)](#whats-accelerated)
[![Upstream Unciv](https://img.shields.io/badge/Built%20on-yairm210%2FUnciv%205.0.2-1f6feb?logo=github)](https://github.com/yairm210/Unciv)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-brightgreen.svg)](LICENSE)

> **Built on the shoulders of giants:** **Unciv-Rust** is a community performance fork of [**Unciv**](https://github.com/yairm210/Unciv), created by **Yair Morgenstern ([`@yairm210`](https://github.com/yairm210))** and hundreds of open-source **Unciv developers, UI designers, tileset & unit artists, icon creators, and translators**. **100% of Unciv's game design, Civ V ruleset, UI, skins, artwork, tilesets, Civilopedia, modding framework, and translations belong to the upstream Unciv project and its contributors** — Unciv-Rust solely adds an optional Rust + Rayon native simulation engine beneath Unciv's existing UI so mobile & desktop players can enjoy smooth 300+ turn campaigns on Large and Huge maps. See [**Full Credits & Attribution**](#credits--attribution) below!

---

## What is Unciv-Rust `v0.2.0-rust`?

**Unciv-Rust** integrates a multi-threaded Rust + Rayon native simulation engine (combining `v0.1.0-rust` with the theater flow-field & parallel combat architecture from `HexForge-Rust`) directly into upstream **[Unciv `5.0.2`](https://github.com/yairm210/Unciv)** so **any Android phone can play smoothly past Turn 300–350+ on Large and Huge maps**:

- **15.1× faster late-game turns on Huge 22-player maps** (`34.49s` Vanilla → **`2.28s`** on Turns 301–325; **19.0× lower peak spike**, with **zero turns over 3.0 seconds** across all 350 turns!).
- **Sub-second late-game median on Large 16-player maps** (**`0.925s` median** / **`1.031s` mean** across Turns 301–350).
- **100% Authentic Unciv `5.0.2` UI, Rules, Mods & Save Compatibility** — preserves Unciv's entire LibGDX UI, tilesets, skins, Religion, Espionage, Diplomacy, City-States, World Congress, and save format (only adds one toggle in **Options → Display → Performance**).
- **Optional `Turbo Rust Native AI` Mode (ON by default in single-player)** — runs a bounded multi-core Rayon 2-phase combat & flow-field planner while still executing every attack and move through Unciv's real `Battle.movePreparingAttack`, `Battle.attackOrNuke`, and `UnitMovement.moveToTile`. Automatically disabled in **Online Multiplayer** for strict cross-client determinism, and transparently falls back to Kotlin for air/nuke/civilian/modded units.
- **All 4 Android ABIs Built In** — ships stripped `libunciv_native.so` binaries for `arm64-v8a`, `armeabi-v7a`, `x86_64`, and `x86` (Android 5.0 / API 21+).

---

## Download

| Platform | Direct Download | Notes |
|---|---|---|
| **Android (Phone / Tablet)** | [**Download `Unciv-Rust-Android.apk`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Android.apk) | Universal APK (Android 5.0+ / API 21+) with all 4 Rust native ABIs |
| **Desktop (Linux / Windows / macOS)** | [**Download `Unciv-Rust-Desktop.jar`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Desktop.jar) | Run with `java -jar Unciv-Rust-Desktop.jar` (bundles `linux-x86_64` native lib; transparent Kotlin fallback on other OSes) |
| **All Releases** | [**GitHub Releases**](https://github.com/OrionFlaxan/Unciv-Rust/releases) | Browse all versions and release notes |

### Installing on Android
1. Tap [**Download `Unciv-Rust-Android.apk`**](https://github.com/OrionFlaxan/Unciv-Rust/releases/latest/download/Unciv-Rust-Android.apk) on your phone (or open the [Releases page](https://github.com/OrionFlaxan/Unciv-Rust/releases)).
2. Open the downloaded `.apk` file and allow **Install unknown apps** if prompted by Android.
3. **Already have Unciv from Google Play or F-Droid?** Back up your saves first and uninstall the store version before installing this APK, since this build uses Unciv's standard `com.unciv.app` package ID with a different signing key.
4. To toggle between **Turbo Rust Native AI** (fastest, default ON) and **Standard Exact Mode**, open **Options → Display → Performance → Turbo Rust Native AI**.

---

## Performance Benchmarks

### Huge Map — 22 AI Civilizations, Seed 42, 350 Turns (4,921 Tiles, ~1,200+ Units)

Measured head-to-head via `OjhBenchmark` (`turns --turns 350 --players 22 --size huge --seed 42`) on the same Linux VM:

| Turn Bucket | Vanilla Unciv | Unciv-Rust `v0.1.0-rust` | `v0.2.0-rust` Standard Exact (`5.0.2`) | **`v0.2.0-rust` Turbo Rust AI (`5.0.2`)** | Speedup vs Vanilla | Speedup vs Standard |
|---|---:|---:|---:|---:|---:|---:|
| **Turns 1–50** | 0.396s | 0.272s | 0.271s | **0.281s** | **1.41×** | — |
| **Turns 51–100** | 1.155s | 0.641s | 0.664s | **0.670s** | **1.72×** | — |
| **Turns 101–150** | 2.775s | 1.121s | 1.071s | **1.004s** | **2.76×** | **1.07×** |
| **Turns 151–200** | 3.994s | 1.326s | 1.387s | **1.119s** | **3.57×** | **1.24×** |
| **Turns 201–250** | 6.196s | 1.939s | 1.809s | **1.781s** | **3.48×** | **1.02×** |
| **Turns 251–300** | 8.188s | 2.618s | 2.869s | **2.145s** | **3.82×** | **1.34×** |
| **Turns 301–325** | **34.492s** | 2.861s | 3.657s | **2.280s** | **15.13×** | **1.60×** |
| **Turns 326–350** | *(timeout)* | 3.283s | 3.804s | **2.360s** | — | **1.61×** |
| **Turns 301–350 Mean** | *(timeout)* | 3.072s | 3.730s | **2.320s** | — | **1.61×** |
| **Peak Spike (Max)** | **56.442s** | 4.127s | 5.391s | **2.973s** | **18.98×** | **1.81×** |
| **Turns 1–325 Total** | `33.29 min` | `7.79 min` | `8.25 min` | **`6.78 min`** | **4.91×** | **1.22×** |
| **Turns 1–350 Total** | *(timeout)* | `9.16 min` | `9.83 min` | **`7.77 min`** | — | **1.27×** |

### Large Map — 16 AI Civilizations, Seed 42, 350 Turns

| Metric | Unciv-Rust `v0.1.0-rust` | **`v0.2.0-rust` Turbo Rust AI (`5.0.2`)** | Improvement |
|---|---:|---:|---:|
| **Turns 301–350 Mean** | 1.408s | **1.031s** | **1.37× faster** |
| **Turns 301–350 Median** | 1.360s | **0.925s** *(sub-second!)* | **1.47× faster** |
| **Peak Spike (All 350 Turns)** | 2.10s+ | **1.466s** | **>1.43× lower spike** |
| **Total 350-Turn Wall-Clock** | 4.41 min (`0.755s/turn`) | **3.88 min (`0.666s/turn`)** | **1.14× faster overall** |

- **Save / Load Compression (4.51 MB Turn-350 Save JSON):** **2.74× faster save** (`32.75 ms → 11.96 ms`) and **2.75× faster load** (`13.21 ms → 4.81 ms`), 100% wire-compatible with vanilla Unciv saves.

---

## What's Accelerated?

### New in `v0.2.0-rust` (Synced to Upstream Unciv `5.0.2`)
- **Bounded Multi-Threaded `rayon` Pool (`world.rs`):** Lazily initialized Rayon pool capped at `min(cores, 4)` threads so Android phones get parallel late-game AI throughput without starving the LibGDX render thread.
- **O(V) Parallel Theater Flow-Fields (`flowfield.rs`):** Computes 5 multi-source reverse-Dijkstra flow-fields (`Enemy City Land`, `Enemy City Water`, `Friendly City`, `Sieged City`, `Explore Frontier`) once per civ turn in parallel via `rayon::join`, giving units O(1) strategic navigation in `HeadTowardsEnemyCityAutomation`, `UnitAutomation.tryExplore`, and `Turbo Rust Native AI`.
- **Turbo Rust Native AI (`turbo_ai.rs`):** 2-phase deterministic parallel combat & movement planner calibrated to Unciv's exact Civ V exponential combat formula, river crossing penalties, terrain defense, flanking, and LOS — with conflict-free hex reservation and projected enemy HP overkill avoidance. Executes every move/attack through real Unciv logic so XP, promotions, captures, and mod triggers fire authentically.
- **Zero-Allocation `BattleDamage` Numeric Fast Path (`BattleDamage.kt`, `GreatGeneralImplementation.kt`):** Sums combat modifiers directly as primitive `Int`s without allocating `Counter<String>` maps or calling `.tr()` translation lookups on every AI combat evaluation (keeping UI Combat Preview breakdowns 100% untouched).
- **Single-Expansion `batchCanReach` & Cached `WorkerAutomation` (`pathing.rs`, `WorkerAutomation.kt`):** Replaces K separate worker `canReach(tile)` A\* searches with one shared Rust Dijkstra pass, pre-filters civ-buildable improvements per turn, and eliminates duplicate priority evaluations.
- **JFR-Driven Kotlin Hot-Path Optimizations:**
  - `ConstructionAutomation.kt`: Reuses a single cloned `scratchCity` across all candidate buildings (~30× fewer `City.clone()` + `setTransients()` calls per city).
  - `NativeWorldMirror.findRoadPath`: Caches per-civ `validRoadBits` bitsets across city pairs with automatic dirty invalidation.
  - `MotivationToAttackAutomation.kt` & `AutomationCache.kt`: Replaces world-wide 250-city scans with direct city-pair checks and per-civ wartime border caches.
  - `NextTurnAutomation.trainSettler`: Replaces the `C × 331` tile scan with Rust's O(V) `nativeHasEnoughSettlerRoom` bitset pass.
- **Background Speculative Pre-Turn Warming & `memcpy` Topology Cloning (`NativeSpeculativePrewarmer.kt`, `WorldScreen.kt`):** Keeps static terrain topology warm during the human's turn and clones it into `gameInfoClone` via `nativeCloneWorldTopology` (~0.05 ms) the instant **Next Turn** is tapped.

### Core `v0.1.0-rust` Native Engine (`rust/unciv-native/`)
- **Flat SoA World Mirror & Zero-Alloc Scratch Arenas (`world.rs`, `scratch.rs`):** Generation-stamped thread-local BFS/A\*/Dijkstra scratch arenas.
- **Exact Fixed-Point Pathfinding (`pathing.rs`):** `1/60` FPM movement costs, ZoC, embark/disembark, retroactive Turn-0 pause, `compute_distance_within_turn`, `find_shortest_path`, `batch_first_reachable`, and `batch_can_reach`.
- **Incremental Vision & Hex LOS (`vision.rs`):** Exact hex raycasting and O(sight²) reference-counted civ vision updates.
- **Threat Field & Attack Discovery (`ai.rs`):** Multi-source BFS enemy-distance threat field and batched LOS attack-pair discovery.
- **Trade & Road Connectivity (`connectivity.rs`):** Native `CapitalConnectionsFinder`, road A\*, and attack-path evaluation.
- **Map Generation & Save Compression (`mapgen.rs`, `compression.rs`):** Bit-exact batched Perlin noise, iterative continent flood-fill, and native `zlib-rs` + `base64` save compression.

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

# Run full Unciv JUnit test suite (1,039 tests; auto-builds desktop Rust library)
./gradlew :tests:test

# Build Desktop JAR (bundles native library)
./gradlew :desktop:dist

# Build Android APK for all 4 ABIs (auto-runs rust/build-android.sh)
./gradlew :android:assembleDebug
```

### Useful Runtime & Build Flags
- `-PskipRustBuild=true` — skip Cargo compilation in Gradle when iterating only on Kotlin code.
- `-Dunciv.disableNative=true` — disable the Rust native library at runtime to force the pure-Kotlin fallback path.
- `-Dunciv.turboRustAi=false` — disable Turbo Rust Native AI for A/B benchmarking against Standard Exact mode.

---

## Credits & Attribution

**Unciv-Rust would not exist without the extraordinary work of the Unciv creator, core developers, UI designers, artists, and community.** Every screen, button, icon, ruleset, tutorial, AI architecture, and mod hook you see in-game comes directly from upstream **Unciv**:

### Upstream Unciv Project & Creator
- **Yair Morgenstern ([`@yairm210`](https://github.com/yairm210))** — Creator and lead developer of [**Unciv**](https://github.com/yairm210/Unciv). Please support the official Unciv project!
  - **Official Unciv Links:** [GitHub](https://github.com/yairm210/Unciv) · [Google Play](https://play.google.com/store/apps/details?id=com.unciv.app) · [F-Droid](https://f-droid.org/en/packages/com.unciv.app/) · [Discord](https://discord.gg/bjrB4Xw) · [Wiki](https://yairm210.github.io/Unciv/)
- **Unciv Core Contributors & Community Developers** — All 400+ open-source contributors who built Unciv's LibGDX UI, Civ V mechanics, `View` architecture, Purity verification plugin (`yairm210/Purity`), `OjhBenchmark`, AI automation, modding API, multiplayer, and translations.

### UI, Art, Tilesets, Icons, Audio & Libraries
- **Unciv UI, Skins & Artwork Contributors** — All original UI screens, skins (`Minimal`), icons, and unit/tileset artwork are credited in detail in [**`docs/Credits.md`**](docs/Credits.md), including:
  - **Unciv Logo & Banner:** `yairm210`, `u-ndefine`, `letstalkaboutdune`, and `The Bucketeer / @GeneralWadaling`.
  - **Fantasy Hex Tileset:** [CuddlyClover](https://cuddlyclover.itch.io/fantasy-hex-tiles) & community contributors (CC BY 4.0).
  - **Icons & Flags:** [The Noun Project](https://thenounproject.com) artists, [Freepik](https://www.flaticon.com/authors/freepik), and community flag/unit/building/tech icon artists listed in [**`docs/Credits.md`**](docs/Credits.md).
- **[LibGDX](https://github.com/libgdx/libgdx)** — The cross-platform Android & Desktop game framework powering Unciv's UI and rendering.
- **[`carlislefox/libgdx-texture-array-batch`](https://github.com/carlislefox/libgdx-texture-array-batch)** — `TextureArraySpriteBatch.java` (CC0 1.0 Universal, see [LICENSE-TextureArraySpriteBatch](LICENSE-TextureArraySpriteBatch)).
- **Rust Ecosystem Crates** — [`rayon`](https://github.com/rayon-rs/rayon), [`jni`](https://github.com/jni-rs/jni-rs), [`flate2`](https://github.com/rust-lang/flate2-rs) (`zlib-rs`), and [`base64`](https://github.com/marshallpierce/rust-base64).

### Licensing
- Unciv and Unciv-Rust source code is licensed under the **Mozilla Public License 2.0 (MPL-2.0)** (see [LICENSE](LICENSE)), except where otherwise noted.
- Media and artwork files remain under their respective CC BY-SA 4.0, CC BY 3.0/4.0, CC0, and Public Domain licenses as documented in [**`docs/Credits.md`**](docs/Credits.md).
- Gameplay tutorials and Civ V references are available in-game under *Menu → Civilopedia → Tutorials* and on the [Civ V Wiki](https://civilization.fandom.com/wiki/).
