//! Panic-safe JNI bindings for `com.unciv.nativeengine.UncivRustNative`.
//!
//! Every entry point wraps execution in `std::panic::catch_unwind` so a Rust panic never
//! terminates the host JVM or Android ART process.

use crate::ai;
use crate::compression;
use crate::connectivity;
use crate::mapgen;
use crate::pathing::{self, UnitProfile};
use crate::vision;
use crate::world::NativeWorld;

use jni::objects::{
    JByteArray, JClass, JDoubleArray, JFloatArray, JIntArray, JLongArray, JShortArray,
};
use jni::sys::{jboolean, jbyte, jdouble, jfloat, jint, jlong, jshort, JNI_TRUE};
use jni::JNIEnv;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::RwLock;

struct WorldHandle {
    world: RwLock<NativeWorld>,
}

#[inline]
unsafe fn handle_ref<'a>(handle: jlong) -> Option<&'a WorldHandle> {
    if handle == 0 {
        None
    } else {
        Some(&*(handle as *const WorldHandle))
    }
}

#[inline]
fn to_int_array<'local>(env: &JNIEnv<'local>, slice: &[i32]) -> jni::sys::jintArray {
    match env.new_int_array(slice.len() as i32) {
        Ok(arr) => {
            if !slice.is_empty() {
                let _ = env.set_int_array_region(&arr, 0, slice);
            }
            arr.into_raw()
        }
        Err(_) => std::ptr::null_mut(),
    }
}

#[inline]
fn empty_int_array<'local>(env: &JNIEnv<'local>) -> jni::sys::jintArray {
    to_int_array(env, &[])
}

#[inline]
fn read_i32_vec(env: &JNIEnv, arr: &JIntArray) -> Vec<i32> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0i32; len];
    if len > 0 {
        let _ = env.get_int_array_region(arr, 0, &mut v);
    }
    v
}

#[inline]
fn read_i16_vec(env: &JNIEnv, arr: &JShortArray) -> Vec<i16> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0i16; len];
    if len > 0 {
        let _ = env.get_short_array_region(arr, 0, &mut v);
    }
    v
}

#[inline]
fn read_u16_vec(env: &JNIEnv, arr: &JShortArray) -> Vec<u16> {
    read_i16_vec(env, arr).into_iter().map(|x| x as u16).collect()
}

#[inline]
fn read_i8_vec(env: &JNIEnv, arr: &JByteArray) -> Vec<i8> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0i8; len];
    if len > 0 {
        let _ = env.get_byte_array_region(arr, 0, &mut v);
    }
    v
}

#[inline]
fn read_u8_vec(env: &JNIEnv, arr: &JByteArray) -> Vec<u8> {
    let v = read_i8_vec(env, arr);
    // Safe zero-cost reinterpret of Vec<i8> -> Vec<u8> via map (compiler optimizes to memcpy/noop)
    v.into_iter().map(|x| x as u8).collect()
}

#[inline]
fn read_u32_vec(env: &JNIEnv, arr: &JIntArray) -> Vec<u32> {
    read_i32_vec(env, arr).into_iter().map(|x| x as u32).collect()
}

#[inline]
fn read_u64_vec(env: &JNIEnv, arr: &JLongArray) -> Vec<u64> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0i64; len];
    if len > 0 {
        let _ = env.get_long_array_region(arr, 0, &mut v);
    }
    v.into_iter().map(|x| x as u64).collect()
}

#[inline]
fn read_f32_vec(env: &JNIEnv, arr: &JFloatArray) -> Vec<f32> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0f32; len];
    if len > 0 {
        let _ = env.get_float_array_region(arr, 0, &mut v);
    }
    v
}

#[inline]
fn read_f64_vec(env: &JNIEnv, arr: &JDoubleArray) -> Vec<f64> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut v = vec![0f64; len];
    if len > 0 {
        let _ = env.get_double_array_region(arr, 0, &mut v);
    }
    v
}

// =========================================================================
// 1. Lifecycle & World Sync
// =========================================================================

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeCreateWorld(
    _env: JNIEnv,
    _class: JClass,
    num_tiles: jint,
    num_civs: jint,
    world_wrap: jboolean,
    wrap_radius: jint,
    map_width: jint,
) -> jlong {
    catch_unwind(|| {
        let w = NativeWorld::new(
            num_tiles.max(0) as usize,
            num_civs.max(0) as usize,
            world_wrap == JNI_TRUE,
            wrap_radius,
            map_width,
        );
        let boxed = Box::new(WorldHandle {
            world: RwLock::new(w),
        });
        Box::into_raw(boxed) as jlong
    })
    .unwrap_or(0)
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeDestroyWorld(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    let _ = catch_unwind(|| {
        if handle != 0 {
            unsafe {
                drop(Box::from_raw(handle as *mut WorldHandle));
            }
        }
    });
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeSyncStaticTopology(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    pos_x: JShortArray,
    pos_y: JShortArray,
    neighbors: JIntArray,
    river_mask: JByteArray,
    terrain_flags: JIntArray,
    base_move_cost: JByteArray,
    terrain_damage: JShortArray,
    tile_height: JByteArray,
    unit_height: JByteArray,
    continent: JShortArray,
    base_terrain_id: JByteArray,
    hill_terrain_id: JByteArray,
    last_terrain_id: JByteArray,
    feature_terrain_mask: JLongArray,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        w.pos_x = read_i16_vec(&env, &pos_x);
        w.pos_y = read_i16_vec(&env, &pos_y);
        w.neighbors = read_i32_vec(&env, &neighbors);
        w.river_mask = read_u8_vec(&env, &river_mask);
        w.terrain_flags = read_u32_vec(&env, &terrain_flags);
        w.base_move_cost = read_u8_vec(&env, &base_move_cost);
        w.terrain_damage = read_i16_vec(&env, &terrain_damage);
        w.tile_height = read_i8_vec(&env, &tile_height);
        w.unit_height = read_i8_vec(&env, &unit_height);
        w.continent = read_i16_vec(&env, &continent);
        w.base_terrain_id = read_u8_vec(&env, &base_terrain_id);
        w.hill_terrain_id = read_u8_vec(&env, &hill_terrain_id);
        w.last_terrain_id = read_u8_vec(&env, &last_terrain_id);
        w.feature_terrain_mask = read_u64_vec(&env, &feature_terrain_mask);
    }));
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeSyncDynamicState(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    num_civs: jint,
    owner_civ: JShortArray,
    tile_flags: JShortArray,
    mil_unit_civ: JShortArray,
    mil_unit_flags: JByteArray,
    civ_unit_civ: JShortArray,
    air_unit_civ: JShortArray,
    city_bombard_range: JByteArray,
    diplo_matrix: JByteArray,
    civ_flags: JIntArray,
    civ_road_speed_fpm: JShortArray,
    civ_road_speed_f32: JFloatArray,
    civ_passable_impassable_mask: JLongArray,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        w.resize_civs(num_civs.max(0) as usize);
        w.owner_civ = read_i16_vec(&env, &owner_civ);
        w.tile_flags = read_u16_vec(&env, &tile_flags);
        w.mil_unit_civ = read_i16_vec(&env, &mil_unit_civ);
        w.mil_unit_flags = read_u8_vec(&env, &mil_unit_flags);
        w.civ_unit_civ = read_i16_vec(&env, &civ_unit_civ);
        w.air_unit_civ = read_i16_vec(&env, &air_unit_civ);
        w.city_bombard_range = read_u8_vec(&env, &city_bombard_range);
        w.diplo_matrix = read_u8_vec(&env, &diplo_matrix);
        w.civ_flags = read_u32_vec(&env, &civ_flags);
        w.civ_road_speed_fpm = read_u16_vec(&env, &civ_road_speed_fpm);
        w.civ_road_speed_f32 = read_f32_vec(&env, &civ_road_speed_f32);
        w.civ_passable_impassable_mask = read_u64_vec(&env, &civ_passable_impassable_mask);
        w.zoc_dirty = true;
        w.threat_dirty = true;
    }));
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeSyncExploredBits(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    bits: JLongArray,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let c = civ_idx as usize;
        if c >= w.num_civs {
            return;
        }
        let words = read_u64_vec(&env, &bits);
        let copy_len = words.len().min(w.bitset_words);
        let off = c * w.bitset_words;
        w.explored_bits[off..off + copy_len].copy_from_slice(&words[..copy_len]);
    }));
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeSyncVisibleBits(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    bits: JLongArray,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let c = civ_idx as usize;
        if c >= w.num_civs {
            return;
        }
        let words = read_u64_vec(&env, &bits);
        let copy_len = words.len().min(w.bitset_words);
        let off = c * w.bitset_words;
        w.visible_bits[off..off + copy_len].copy_from_slice(&words[..copy_len]);
        if w.threat_civ == civ_idx {
            w.threat_dirty = true;
        }
    }));
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchTileUnits(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    tile_idx: jint,
    mil_civ: jshort,
    mil_flags: jbyte,
    civ_civ: jshort,
    air_civ: jshort,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        w.patch_tile_units(
            tile_idx as usize,
            mil_civ,
            mil_flags as u8,
            civ_civ,
            air_civ,
        );
    });
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchTileDynamic(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    tile_idx: jint,
    owner_civ: jshort,
    tile_flags: jshort,
    bombard_range: jbyte,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        w.patch_tile_dynamic(
            tile_idx as usize,
            owner_civ,
            tile_flags as u16,
            bombard_range as u8,
        );
    });
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchTileTerrain(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    tile_idx: jint,
    terrain_flags: jint,
    base_move_cost: jbyte,
    terrain_damage: jshort,
    tile_height: jbyte,
    unit_height: jbyte,
    base_terrain_id: jbyte,
    hill_terrain_id: jbyte,
    last_terrain_id: jbyte,
    feature_terrain_mask: jlong,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        w.patch_tile_terrain(
            tile_idx as usize,
            terrain_flags as u32,
            base_move_cost as u8,
            terrain_damage,
            tile_height,
            unit_height,
            base_terrain_id as u8,
            hill_terrain_id as u8,
            last_terrain_id as u8,
            feature_terrain_mask as u64,
        );
    });
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchTileExplored(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    tile_idx: jint,
    explored: jboolean,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let c = civ_idx as usize;
        let t = tile_idx as usize;
        if c < w.num_civs && t < w.num_tiles {
            w.set_explored_bit(c, t, explored == JNI_TRUE);
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchDiplo(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_a: jint,
    civ_b: jint,
    diplo_byte: jbyte,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let a = civ_a as usize;
        let b = civ_b as usize;
        if a < w.num_civs && b < w.num_civs {
            let nc = w.num_civs;
            w.diplo_matrix[a * nc + b] = diplo_byte as u8;
            w.zoc_dirty = true;
            w.threat_dirty = true;
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePatchCivFlags(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    civ_flags: jint,
    road_speed_fpm: jshort,
    road_speed_f32: jfloat,
    passable_impassable_mask: jlong,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let c = civ_idx as usize;
        if c < w.num_civs {
            w.civ_flags[c] = civ_flags as u32;
            w.civ_road_speed_fpm[c] = road_speed_fpm as u16;
            w.civ_road_speed_f32[c] = road_speed_f32;
            w.civ_passable_impassable_mask[c] = passable_impassable_mask as u64;
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeMarkVisionDirty(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
) {
    let _ = catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        if civ_idx < 0 {
            w.vision_synced.fill(false);
            w.threat_dirty = true;
        } else if (civ_idx as usize) < w.num_civs {
            w.vision_synced[civ_idx as usize] = false;
            if w.threat_civ == civ_idx {
                w.threat_dirty = true;
            }
        }
    });
}

// =========================================================================
// 2. Pathfinding & Movement
// =========================================================================

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeComputeDistanceWithinTurn(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    profile_flags: jint,
    current_move_f32: jfloat,
    current_move_fpm: jint,
    full_move_fpm: jint,
    cost_embark_fpm: jint,
    cost_disembark_fpm: jint,
    enemy_penalty_fpm: jint,
    dm_feature_mask: jlong,
    dm_base_mask: jlong,
    dm_hill_mask: jlong,
    start_tile: jint,
    include_unpassable_boundary: jboolean,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let p = UnitProfile {
            civ_idx: civ_idx as usize,
            flags: profile_flags as u32,
            current_move_f32,
            current_move_fpm,
            full_move_fpm,
            cost_to_embark_fpm: cost_embark_fpm,
            cost_to_disembark_fpm: cost_disembark_fpm,
            enemy_penalty_fpm,
            dm_feature_mask: dm_feature_mask as u64,
            dm_base_mask: dm_base_mask as u64,
            dm_hill_mask: dm_hill_mask as u64,
        };
        let res = pathing::compute_distance_within_turn(
            &mut w,
            &p,
            start_tile as usize,
            include_unpassable_boundary == JNI_TRUE,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeFindShortestPath(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    profile_flags: jint,
    current_move_f32: jfloat,
    current_move_fpm: jint,
    full_move_fpm: jint,
    cost_embark_fpm: jint,
    cost_disembark_fpm: jint,
    enemy_penalty_fpm: jint,
    dm_feature_mask: jlong,
    dm_base_mask: jlong,
    dm_hill_mask: jlong,
    start_tile: jint,
    dest_tile: jint,
    max_turns: jint,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let p = UnitProfile {
            civ_idx: civ_idx as usize,
            flags: profile_flags as u32,
            current_move_f32,
            current_move_fpm,
            full_move_fpm,
            cost_to_embark_fpm: cost_embark_fpm,
            cost_to_disembark_fpm: cost_disembark_fpm,
            enemy_penalty_fpm,
            dm_feature_mask: dm_feature_mask as u64,
            dm_base_mask: dm_base_mask as u64,
            dm_hill_mask: dm_hill_mask as u64,
        };
        let res = pathing::find_shortest_path(
            &mut w,
            &p,
            start_tile as usize,
            dest_tile as usize,
            max_turns,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeBatchFirstReachable(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    profile_flags: jint,
    current_move_f32: jfloat,
    current_move_fpm: jint,
    full_move_fpm: jint,
    cost_embark_fpm: jint,
    cost_disembark_fpm: jint,
    enemy_penalty_fpm: jint,
    dm_feature_mask: jlong,
    dm_base_mask: jlong,
    dm_hill_mask: jlong,
    start_tile: jint,
    candidates: JIntArray,
    max_turns: jint,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return to_int_array(&env, &[-1]);
        };
        let cands = read_i32_vec(&env, &candidates);
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let p = UnitProfile {
            civ_idx: civ_idx as usize,
            flags: profile_flags as u32,
            current_move_f32,
            current_move_fpm,
            full_move_fpm,
            cost_to_embark_fpm: cost_embark_fpm,
            cost_to_disembark_fpm: cost_disembark_fpm,
            enemy_penalty_fpm,
            dm_feature_mask: dm_feature_mask as u64,
            dm_base_mask: dm_base_mask as u64,
            dm_hill_mask: dm_hill_mask as u64,
        };
        let (idx, path) =
            pathing::batch_first_reachable(&mut w, &p, start_tile as usize, &cands, max_turns);
        let mut packed = Vec::with_capacity(1 + path.len());
        packed.push(idx);
        packed.extend_from_slice(&path);
        to_int_array(&env, &packed)
    }))
    .unwrap_or(std::ptr::null_mut())
}

// =========================================================================
// 3. Vision & Line-of-Sight
// =========================================================================

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeComputeViewableTiles(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    origin_idx: jint,
    sight_distance: jint,
    for_attack: jboolean,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let res = vision::compute_viewable_tiles(
            &w,
            origin_idx as usize,
            sight_distance,
            for_attack == JNI_TRUE,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeRebuildCivVision(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    static_tiles: JIntArray,
    unit_tiles_flat: JIntArray,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let st = read_i32_vec(&env, &static_tiles);
        let ut = read_i32_vec(&env, &unit_tiles_flat);
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let newly = vision::rebuild_civ_vision(&mut w, civ_idx as usize, &st, &ut);
        to_int_array(&env, &newly)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeApplyUnitVisionDelta(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    old_tiles: JIntArray,
    new_tiles: JIntArray,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let old_t = read_i32_vec(&env, &old_tiles);
        let new_t = read_i32_vec(&env, &new_tiles);
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        let packed = vision::apply_unit_vision_delta(&mut w, civ_idx as usize, &old_t, &new_t);
        to_int_array(&env, &packed)
    }))
    .unwrap_or(std::ptr::null_mut())
}

// =========================================================================
// 4. AI Threat Field, Attack Discovery & Exploration
// =========================================================================

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeBuildThreatField(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    extra_detected_enemies: JIntArray,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return;
        };
        let extra = read_i32_vec(&env, &extra_detected_enemies);
        let mut w = h.world.write().unwrap_or_else(|e| e.into_inner());
        ai::build_threat_field(&mut w, civ_idx as usize, &extra);
    }));
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeGetDistanceToClosestEnemy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    tile_idx: jint,
    max_dist: jint,
    take_larger_values: jboolean,
) -> jint {
    catch_unwind(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return if take_larger_values == JNI_TRUE {
                500_000
            } else {
                max_dist
            };
        };
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        ai::get_distance_to_closest_enemy(
            &w,
            tile_idx as usize,
            max_dist,
            take_larger_values == JNI_TRUE,
        )
    })
    .unwrap_or(500_000)
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeGetEnemyTilesInDistance(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    tile_idx: jint,
    max_dist: jint,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let res = ai::get_enemy_tiles_in_distance(&w, tile_idx as usize, max_dist);
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeFindAttackablePairs(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    from_tiles: JIntArray,
    from_move_left_bits: JIntArray,
    attack_range: jint,
    is_melee: jboolean,
    indirect_fire: jboolean,
    candidate_enemy_tiles: JIntArray,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let from_t = read_i32_vec(&env, &from_tiles);
        let from_m = read_i32_vec(&env, &from_move_left_bits);
        let enemies = read_i32_vec(&env, &candidate_enemy_tiles);
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let res = ai::find_attackable_pairs(
            &w,
            civ_idx as usize,
            &from_t,
            &from_m,
            attack_range,
            is_melee == JNI_TRUE,
            indirect_fire == JNI_TRUE,
            &enemies,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeFilterAndScoreExploreTiles(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    profile_flags: jint,
    cost_embark_fpm: jint,
    cost_disembark_fpm: jint,
    dm_feature_mask: jlong,
    dm_base_mask: jlong,
    dm_hill_mask: jlong,
    candidates: JIntArray,
    vis_range: jint,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let cands = read_i32_vec(&env, &candidates);
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let p = UnitProfile {
            civ_idx: civ_idx as usize,
            flags: profile_flags as u32,
            current_move_f32: 1.0,
            current_move_fpm: 30,
            full_move_fpm: 30,
            cost_to_embark_fpm: cost_embark_fpm,
            cost_to_disembark_fpm: cost_disembark_fpm,
            enemy_penalty_fpm: 0,
            dm_feature_mask: dm_feature_mask as u64,
            dm_base_mask: dm_base_mask as u64,
            dm_hill_mask: dm_hill_mask as u64,
        };
        let res = ai::filter_and_score_explore_tiles(&w, &p, &cands, vis_range);
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

// =========================================================================
// 5. Connectivity (Capital, Roads, Attack Paths)
// =========================================================================

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeFindCapitalConnections(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    city_tiles: JIntArray,
    city_flags: JIntArray,
    capital_slot: jint,
    road_researched: jboolean,
    railroad_researched: jboolean,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let ct = read_i32_vec(&env, &city_tiles);
        let cf = read_i32_vec(&env, &city_flags);
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let res = connectivity::find_capital_connections(
            &w,
            civ_idx as usize,
            &ct,
            &cf,
            capital_slot as usize,
            road_researched == JNI_TRUE,
            railroad_researched == JNI_TRUE,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeFindRoadPath(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    start_tile: jint,
    targets: JIntArray,
    valid_road_bits: JLongArray,
    max_half_cost: jint,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return empty_int_array(&env);
        };
        let tgs = read_i32_vec(&env, &targets);
        let bits = read_u64_vec(&env, &valid_road_bits);
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        let res = connectivity::find_road_path(
            &w,
            civ_idx as usize,
            start_tile as usize,
            &tgs,
            &bits,
            max_half_cost,
        );
        to_int_array(&env, &res)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeEvalAttackPaths(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    civ_idx: jint,
    target_civ_idx: jint,
    our_city_tiles: JIntArray,
    enemy_city_tiles: JIntArray,
    capital_tile: jint,
    all_enemy_city_tiles: JIntArray,
) -> jfloat {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(h) = (unsafe { handle_ref(handle) }) else {
            return -3.0;
        };
        let ours = read_i32_vec(&env, &our_city_tiles);
        let enemies = read_i32_vec(&env, &enemy_city_tiles);
        let all_enemies = read_i32_vec(&env, &all_enemy_city_tiles);
        let w = h.world.read().unwrap_or_else(|e| e.into_inner());
        connectivity::eval_attack_paths(
            &w,
            civ_idx as usize,
            target_civ_idx as usize,
            &ours,
            &enemies,
            capital_tile,
            &all_enemies,
        )
    }))
    .unwrap_or(-3.0)
}

// =========================================================================
// 6. Stateless Map Generation & Compression
// =========================================================================

#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativePerlinBatch(
    env: JNIEnv,
    _class: JClass,
    xs: JDoubleArray,
    ys: JDoubleArray,
    zs: JDoubleArray,
    n_octaves: jint,
    persistence: jdouble,
    lacunarity: jdouble,
    scale: jdouble,
    ridged: jboolean,
) -> jni::sys::jdoubleArray {
    catch_unwind(AssertUnwindSafe(|| {
        let xv = read_f64_vec(&env, &xs);
        let yv = read_f64_vec(&env, &ys);
        let zv = read_f64_vec(&env, &zs);
        let n = xv.len().min(yv.len()).min(zv.len());
        let mut out = vec![0.0f64; n];
        mapgen::perlin_batch(
            &xv,
            &yv,
            &zv,
            n_octaves,
            persistence,
            lacunarity,
            scale,
            ridged == JNI_TRUE,
            &mut out,
        );
        match env.new_double_array(n as i32) {
            Ok(arr) => {
                if n > 0 {
                    let _ = env.set_double_array_region(&arr, 0, &out);
                }
                arr.into_raw()
            }
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeAssignContinents(
    env: JNIEnv,
    _class: JClass,
    neighbors: JIntArray,
    passable_land: JByteArray,
    out_continents: JIntArray,
) -> jni::sys::jintArray {
    catch_unwind(AssertUnwindSafe(|| {
        let nb = read_i32_vec(&env, &neighbors);
        let pl_raw = read_u8_vec(&env, &passable_land);
        let pl: Vec<bool> = pl_raw.into_iter().map(|b| b != 0).collect();
        let mut out = vec![-1i32; pl.len()];
        let sizes = mapgen::assign_continents(&nb, &pl, &mut out);
        if !out.is_empty() {
            let _ = env.set_int_array_region(&out_continents, 0, &out);
        }
        to_int_array(&env, &sizes)
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeGzipBase64Encode(
    env: JNIEnv,
    _class: JClass,
    input: JByteArray,
    level: jint,
) -> jni::sys::jbyteArray {
    catch_unwind(AssertUnwindSafe(|| {
        let bytes = env.convert_byte_array(&input).unwrap_or_default();
        let encoded = compression::gzip_base64_encode(&bytes, level.clamp(0, 9) as u32);
        match env.byte_array_from_slice(&encoded) {
            Ok(arr) => arr.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_unciv_nativeengine_UncivRustNative_nativeGzipBase64Decode(
    env: JNIEnv,
    _class: JClass,
    input: JByteArray,
) -> jni::sys::jbyteArray {
    catch_unwind(AssertUnwindSafe(|| {
        let bytes = env.convert_byte_array(&input).unwrap_or_default();
        match compression::gzip_base64_decode(&bytes) {
            Ok(decoded) => match env.byte_array_from_slice(&decoded) {
                Ok(arr) => arr.into_raw(),
                Err(_) => std::ptr::null_mut(),
            },
            Err(_) => std::ptr::null_mut(),
        }
    }))
    .unwrap_or(std::ptr::null_mut())
}
