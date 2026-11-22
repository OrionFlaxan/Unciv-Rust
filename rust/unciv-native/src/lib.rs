//! `unciv_native` — native performance engine for Unciv.
//!
//! The JVM side (`com.unciv.nativeengine`) maintains a compact Structure-of-Arrays mirror of the
//! map (see [`world`]) inside this library and invokes queries via primitive arrays only, so
//! there are zero JNI callbacks in hot loops.

pub mod ai;
pub mod compression;
pub mod connectivity;
pub mod mapgen;
pub mod pathing;
pub mod scratch;
pub mod vision;
pub mod world;

mod jni_bridge;
