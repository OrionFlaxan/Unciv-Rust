//! Native Rust single-turn movement and multi-turn A* pathfinding engine.
//!
//! Matches Unciv's `MovementCost.kt`, `UnitMovement.kt`, `RouteNode.kt`, and
//! `PathingMapAStarPathfinder.kt` using base-30 fixed-point movement (`1.0f == 30`)
//! and a zero-allocation thread-local generation-stamped `PathScratch` arena.

use crate::scratch::{with_scratch, PathScratch};
use crate::world::*;

// UnitProfile flags (u32)
pub const UP_IS_LAND: u32 = 1 << 0;
pub const UP_IS_WATER: u32 = 1 << 1;
pub const UP_IS_CIVILIAN: u32 = 1 << 2;
pub const UP_CANNOT_MOVE: u32 = 1 << 3;
pub const UP_ALL_TILES_COST_1: u32 = 1 << 4;
pub const UP_IGNORES_TERRAIN_COST: u32 = 1 << 5;
pub const UP_IGNORES_ZOC: u32 = 1 << 6;
pub const UP_ROUGH_TERRAIN_PENALTY: u32 = 1 << 7;
pub const UP_CAN_MOVE_ON_WATER: u32 = 1 << 8;
pub const UP_CAN_PASS_IMPASSABLE: u32 = 1 << 9;
pub const UP_CAN_ENTER_ICE: u32 = 1 << 10;
pub const UP_CANNOT_EMBARK: u32 = 1 << 11;
pub const UP_CANNOT_ENTER_OCEAN: u32 = 1 << 12;
pub const UP_SPECIFIC_ALLOW_OCEAN: u32 = 1 << 13;
pub const UP_CAN_ENTER_FOREIGN: u32 = 1 << 14;
pub const UP_CAN_ENTER_CITY_STATES: u32 = 1 << 15;
pub const UP_AVOID_ENEMY_TERRITORY: u32 = 1 << 16;
pub const UP_NO_TERRAIN_MOVEMENT_UNIQUES: u32 = 1 << 17;
pub const UP_NO_BASE_OR_HILL_DM: u32 = 1 << 18;
pub const UP_CONSIDER_ZOC: u32 = 1 << 19;
pub const UP_AVOID_DAMAGING_TERRAIN: u32 = 1 << 20;
pub const UP_ALLOW_UNPASSABLE_DEST: u32 = 1 << 21;
pub const UP_ASSUME_UNEXPLORED_PASSABLE: u32 = 1 << 22;

pub const FPM_BASE: i32 = 30;
pub const FPM_RAILROAD: i32 = 3; // 0.1f * 30
pub const FPM_HUNDRED: i32 = 3000; // 100f * 30
pub const MAX_MOVE_THIS_TURN_FPM: i32 = 511; // 9 bits (RouteNode.MAX_MOVE_THIS_TURN)
pub const MAX_UNDERESTIMATED_FPM: i32 = 16383; // 14 bits
pub const MAX_TURNS: i32 = 63; // 6 bits
pub const MAX_DAMAGING_TILES: i32 = 3; // 2 bits
pub const MIN_MOVEMENT_EPSILON: f32 = 0.05;

// RouteNode bit layout (exact match to RouteNode.kt / PrioritizedNode.kt)
const TILE_IDX_OFFSET: u32 = 0;
const TILE_IDX_MASK: u64 = (1u64 << 20) - 1;
const REL_OFFSET: u32 = 20;
const MOVE_SINCE_STOP_OFFSET: u32 = 23;
const MOVE_SINCE_STOP_MASK: u64 = 511;
const MOVE_THIS_TURN_OFFSET: u32 = 32;
const MOVE_THIS_TURN_MASK: u64 = 511;
const TURNS_OFFSET: u32 = 41;
const TURNS_MASK: u64 = 63;
const UNDEREST_OFFSET: u32 = 47;
const UNDEREST_MASK: u64 = (1u64 << 14) - 1;
// In RouteNode (stored in route_nodes[]), bits 47..60 hold parent_dir(3b) + damaging_self(1b) + 0
const DAMAGING_SELF_BIT: u64 = 1u64 << 50;
const DAMAGING_TILES_OFFSET: u32 = 61;
const DAMAGING_TILES_MASK: u64 = 3;
const UNDEREST_HI_MASK: u64 = UNDEREST_MASK << UNDEREST_OFFSET;

#[derive(Clone, Copy, Debug)]
pub struct UnitProfile {
    pub civ_idx: usize,
    pub flags: u32,
    /// Unit's remaining movement this turn (float, exact for classic single-turn)
    pub current_move_f32: f32,
    /// Unit's remaining movement this turn (base-30 fixed point)
    pub current_move_fpm: i32,
    /// Unit's max movement per turn (base-30 fixed point)
    pub full_move_fpm: i32,
    /// Reduced embark cost in FPM, or -1 if default (100f)
    pub cost_to_embark_fpm: i32,
    /// Reduced disembark cost in FPM, or -1 if default (100f)
    pub cost_to_disembark_fpm: i32,
    /// Extra movement cost in FPM when entering warring civ with `hasActiveEnemyMovementPenalty`
    pub enemy_penalty_fpm: i32,
    pub dm_feature_mask: u64,
    pub dm_base_mask: u64,
    pub dm_hill_mask: u64,
}

#[inline(always)]
pub fn fpm_from_f32(move_f: f32) -> i32 {
    if move_f <= 0.0 {
        return 0;
    }
    let plus_one_bit = (move_f * 60.0) as i32;
    (plus_one_bit >> 1) + (plus_one_bit & 1)
}

#[inline(always)]
fn bit_get(bits: &[u64], idx: usize) -> bool {
    ((bits[idx >> 6] >> (idx & 63)) & 1) != 0
}

/// Checks if moving from `from` to adjacent `to = world.neighbors[from*6 + dir_idx]` is affected by ZoC.
#[inline]
pub fn is_affected_by_zoc(
    world: &NativeWorld,
    p: &UnitProfile,
    from: usize,
    to: usize,
    dir_idx: usize,
) -> bool {
    if (p.flags & UP_CONSIDER_ZOC) == 0 || (p.flags & UP_IGNORES_ZOC) != 0 {
        return false;
    }
    let is_land = (p.flags & UP_IS_LAND) != 0;
    let zoc_bits = if is_land {
        &world.zoc_land_bits
    } else {
        &world.zoc_water_bits
    };
    // O(1) fast rejection: >98% of edges exit right here
    if !bit_get(zoc_bits, from) || !bit_get(zoc_bits, to) {
        return false;
    }

    let civ = p.civ_idx;
    let base = from * 6;
    if !world.world_wrap || world.wrap_radius >= 3 {
        // On a hex grid, the only two shared neighbors of `from` and `neighbors[from*6 + dir_idx]`
        // are at `(dir_idx + 5) % 6` and `(dir_idx + 1) % 6`.
        let d1 = if dir_idx == 0 { 5 } else { dir_idx - 1 };
        let d2 = if dir_idx == 5 { 0 } else { dir_idx + 1 };
        for d in [d1, d2] {
            let nb = world.neighbors[base + d];
            if nb >= 0 && tile_exerts_zoc(world, civ, is_land, nb as usize) {
                return true;
            }
        }
        false
    } else {
        // Tiny wrapped maps (radius < 3): check all neighbors of `from` that are adjacent to `to`.
        let to_base = to * 6;
        for d in 0..6 {
            let nb = world.neighbors[base + d];
            if nb < 0 {
                continue;
            }
            let mut adj_to = false;
            for k in 0..6 {
                if world.neighbors[to_base + k] == nb {
                    adj_to = true;
                    break;
                }
            }
            if adj_to && tile_exerts_zoc(world, civ, is_land, nb as usize) {
                return true;
            }
        }
        false
    }
}

#[inline(always)]
fn tile_exerts_zoc(world: &NativeWorld, civ: usize, mover_is_land: bool, tile: usize) -> bool {
    if (world.tile_flags[tile] & TILE_CITY_CENTER) != 0 {
        let owner = world.owner_civ[tile];
        return owner >= 0 && world.is_at_war(civ, owner as usize);
    }
    let mc = world.mil_unit_civ[tile];
    if mc >= 0 && world.is_at_war(civ, mc as usize) {
        let mf = world.mil_unit_flags[tile];
        return (mf & UF_WATER) != 0 || (mover_is_land && (mf & UF_EMBARKED) == 0);
    }
    false
}

/// Exact port of `UnitMovement.canPassThrough(tile)`.
#[inline]
pub fn can_pass_through(world: &NativeWorld, p: &UnitProfile, tile: usize) -> bool {
    let tf = world.terrain_flags[tile];
    let civ = p.civ_idx;
    let cflags = world.civ_flags[civ];

    if (tf & TF_IMPASSABLE) != 0 {
        let can_pass_imp = (p.flags & UP_CAN_PASS_IMPASSABLE) != 0;
        let can_ice = (p.flags & UP_CAN_ENTER_ICE) != 0 && (tf & TF_ICE) != 0;
        let last_tid = world.last_terrain_id[tile];
        let civ_pass_imp = (cflags & CF_PASS_IMPASSABLE) != 0
            && last_tid < 64
            && ((world.civ_passable_impassable_mask[civ] >> last_tid) & 1) != 0;
        if !can_pass_imp && !can_ice && !civ_pass_imp {
            return false;
        }
    }

    let tile_flags = world.tile_flags[tile];
    if (tf & TF_LAND) != 0 && (p.flags & UP_IS_WATER) != 0 && (tile_flags & TILE_CITY_CENTER) == 0 {
        return false;
    }

    let unit_specific_ocean = (p.flags & UP_SPECIFIC_ALLOW_OCEAN) != 0;
    if (tf & TF_WATER) != 0 && (p.flags & UP_IS_LAND) != 0 && (p.flags & UP_CAN_MOVE_ON_WATER) == 0
    {
        if (cflags & CF_CAN_EMBARK) == 0 || (p.flags & UP_CANNOT_EMBARK) != 0 {
            return false;
        }
        if (tf & TF_OCEAN) != 0 && (cflags & CF_EMBARK_OCEAN) == 0 && !unit_specific_ocean {
            return false;
        }
    }

    if (tf & TF_OCEAN) != 0 && (cflags & CF_ALL_OCEAN) == 0 {
        if !unit_specific_ocean && (p.flags & UP_CANNOT_ENTER_OCEAN) != 0 {
            return false;
        }
    }

    let owner = world.owner_civ[tile];
    if (p.flags & UP_CAN_ENTER_CITY_STATES) != 0
        && owner >= 0
        && (world.civ_flags[owner as usize] & CF_IS_CITY_STATE) != 0
    {
        return true;
    }
    if (p.flags & UP_CAN_ENTER_FOREIGN) == 0 && !world.can_civ_pass_through(civ, tile) {
        return false;
    }

    let (first_civ, first_is_civilian) = if world.mil_unit_civ[tile] >= 0 {
        (world.mil_unit_civ[tile], false)
    } else if world.civ_unit_civ[tile] >= 0 {
        (world.civ_unit_civ[tile], true)
    } else if world.air_unit_civ[tile] >= 0 {
        (world.air_unit_civ[tile], false)
    } else {
        (NO_CIV, false)
    };

    if first_civ >= 0 && first_civ as usize != civ {
        let at_war = world.is_at_war(civ, first_civ as usize);
        if !((p.flags & UP_IS_LAND) != 0
            && (tf & TF_WATER) != 0
            && (p.flags & UP_CAN_MOVE_ON_WATER) == 0)
            && first_is_civilian
            && at_war
        {
            return true;
        }
        if at_war {
            return false;
        }
    }

    true
}

/// Exact port of `UnitMovement.canMoveTo(tile, assumeCanPassThrough = true, allowSwap = false)`.
#[inline]
pub fn can_move_to_assuming_pass_through(
    world: &NativeWorld,
    p: &UnitProfile,
    tile: usize,
) -> bool {
    let civ = p.civ_idx;
    let tf = world.tile_flags[tile];
    if (tf & TILE_CITY_CENTER) != 0
        && world.owner_civ[tile] as usize != civ
        && (tf & TILE_JUST_CONQUERED) == 0
    {
        return false;
    }
    let mil = world.mil_unit_civ[tile];
    let civ_u = world.civ_unit_civ[tile];
    if (p.flags & UP_IS_CIVILIAN) != 0 {
        if civ_u >= 0 {
            return false;
        }
        if mil >= 0 && mil as usize != civ {
            return false;
        }
    } else {
        if mil >= 0 {
            return false;
        }
        if civ_u >= 0 && civ_u as usize != civ && !world.is_at_war(civ, civ_u as usize) {
            return false;
        }
    }
    true
}

/// Exact port of `MovementCost.getMovementCostBetweenAdjacentTiles` in base-30 fixed point.
#[inline]
pub fn edge_cost_fpm(
    world: &NativeWorld,
    p: &UnitProfile,
    from: usize,
    to: usize,
    dir_idx: usize,
    consider_zoc: bool,
) -> i32 {
    if (p.flags & UP_CANNOT_MOVE) != 0 {
        return FPM_HUNDRED;
    }
    let from_land = (world.terrain_flags[from] & TF_LAND) != 0;
    let to_tf = world.terrain_flags[to];
    let to_land = (to_tf & TF_LAND) != 0;
    if from_land != to_land
        && (p.flags & UP_IS_LAND) != 0
        && (p.flags & UP_CAN_MOVE_ON_WATER) == 0
    {
        return if !from_land && to_land {
            if p.cost_to_disembark_fpm >= 0 {
                p.cost_to_disembark_fpm
            } else {
                FPM_HUNDRED
            }
        } else if p.cost_to_embark_fpm >= 0 {
            p.cost_to_embark_fpm
        } else {
            FPM_HUNDRED
        };
    }

    if consider_zoc && is_affected_by_zoc(world, p, from, to, dir_idx) {
        return FPM_HUNDRED;
    }

    if (p.flags & UP_ALL_TILES_COST_1) != 0 {
        return FPM_BASE;
    }

    let civ = p.civ_idx;
    let to_owner = world.owner_civ[to];
    let extra_cost = if p.enemy_penalty_fpm > 0
        && to_owner >= 0
        && (world.civ_flags[to_owner as usize] & CF_HAS_ENEMY_PENALTY) != 0
        && world.is_at_war(civ, to_owner as usize)
    {
        p.enemy_penalty_fpm
    } else {
        0
    };

    if world.unpillaged_road(from) == TILE_ROAD_RAILROAD
        && world.unpillaged_road(to) == TILE_ROAD_RAILROAD
    {
        return FPM_RAILROAD + extra_cost;
    }

    let connected_by_road = world.has_connection(civ, from) && world.has_connection(civ, to);
    let connected_by_river = (world.river_mask[from] & (1u8 << dir_idx)) != 0;
    let cflags = world.civ_flags[civ];

    if connected_by_road && (!connected_by_river || (cflags & CF_ROADS_ACROSS_RIVERS) != 0) {
        return world.civ_road_speed_fpm[civ] as i32 + extra_cost;
    }

    if (p.flags & UP_IGNORES_TERRAIN_COST) != 0 {
        return FPM_BASE + extra_cost;
    }
    if connected_by_river {
        return FPM_HUNDRED;
    }

    let terrain_cost = if (world.tile_flags[to] & TILE_CITY_CENTER) != 0 {
        FPM_BASE
    } else {
        (world.base_move_cost[to] as i32) * FPM_BASE
    };

    if (p.flags & UP_NO_TERRAIN_MOVEMENT_UNIQUES) != 0 {
        return terrain_cost + extra_cost;
    }

    if (world.feature_terrain_mask[to] & p.dm_feature_mask) != 0 {
        return (terrain_cost >> 1) + extra_cost;
    }

    if (p.flags & UP_ROUGH_TERRAIN_PENALTY) != 0 && (to_tf & TF_ROUGH) != 0 {
        return FPM_HUNDRED;
    }

    if (cflags & CF_IGNORE_HILL) != 0 && (to_tf & TF_HILL) != 0 {
        return FPM_BASE + extra_cost;
    }

    if (p.flags & UP_NO_BASE_OR_HILL_DM) != 0 {
        return terrain_cost + extra_cost;
    }

    let base_id = world.base_terrain_id[to];
    if base_id < 64 && ((p.dm_base_mask >> base_id) & 1) != 0 {
        return (terrain_cost >> 1) + extra_cost;
    }
    let hill_id = world.hill_terrain_id[to];
    if hill_id < 64 && ((p.dm_hill_mask >> hill_id) & 1) != 0 {
        return (terrain_cost >> 1) + extra_cost;
    }

    terrain_cost + extra_cost
}

#[inline]
fn edge_cost_f32(
    world: &NativeWorld,
    p: &UnitProfile,
    from: usize,
    to: usize,
    dir_idx: usize,
    consider_zoc: bool,
) -> f32 {
    let fpm = edge_cost_fpm(world, p, from, to, dir_idx, consider_zoc);
    if fpm == FPM_HUNDRED {
        100.0
    } else {
        (fpm as f32) / (FPM_BASE as f32)
    }
}

/// Computes single-turn reachable tiles (`getMovementToTilesAtPosition` / `getDistanceToTiles`).
///
/// Returns packed triples `[tileIdx, parentIdx, totalMovementFloatBits]` for every reachable tile.
/// When `include_unpassable_boundary` is true (Classic mode), matches `UnitMovement.getMovementToTilesAtPosition`
/// exactly (including `minimumMovementEpsilon`, unexplored tiles costing 1f, and unpassable boundary tiles
/// clamped to `usable_movement`).
pub fn compute_distance_within_turn(
    world: &mut NativeWorld,
    p: &UnitProfile,
    start_tile: usize,
    include_unpassable_boundary: bool,
) -> Vec<i32> {
    if start_tile >= world.num_tiles {
        return Vec::new();
    }
    if (p.flags & UP_CONSIDER_ZOC) != 0 && (p.flags & UP_IGNORES_ZOC) == 0 {
        world.ensure_zoc(p.civ_idx);
    }

    let usable = p.current_move_f32;
    if usable == 0.0 || (!include_unpassable_boundary && usable <= 0.0) || (p.flags & UP_CANNOT_MOVE) != 0 {
        return vec![start_tile as i32, start_tile as i32, 0.0f32.to_bits() as i32];
    }

    let civ = p.civ_idx;
    let consider_zoc = (p.flags & UP_CONSIDER_ZOC) != 0;
    let assume_unexplored = (p.flags & UP_ASSUME_UNEXPLORED_PASSABLE) != 0;

    with_scratch(|sc| {
        let gen = sc.next_gen(world.num_tiles);
        sc.seen_gen[start_tile] = gen;
        sc.dist_f32[start_tile] = 0.0;
        sc.parent_i32[start_tile] = start_tile as i32;
        sc.reached_list.push(start_tile as i32);
        sc.wave_curr.push(start_tile as i32);

        // Clock indices in exact `Tile.neighbors` (`HexMath.getHexCoordsAtDistance(1)`) order:
        // clock 6, 12, 8, 2, 10, 4 -> indices 2, 5, 3, 0, 4, 1
        const NEIGHBOR_ITER_ORDER: [usize; 6] = [2, 5, 3, 0, 4, 1];

        let limit = usable - MIN_MOVEMENT_EPSILON;
        while !sc.wave_curr.is_empty() {
            sc.wave_next.clear();
            let curr_len = sc.wave_curr.len();
            for wi in 0..curr_len {
                let u = sc.wave_curr[wi] as usize;
                let u_dist = sc.dist_f32[u];
                let base = u * 6;
                for &d in &NEIGHBOR_ITER_ORDER {
                    let nb = world.neighbors[base + d];
                    if nb < 0 {
                        continue;
                    }
                    let v = nb as usize;
                    let explored = world.is_explored(civ, v);
                    let total_dist = if assume_unexplored && !explored {
                        u_dist + 1.0
                    } else if !can_pass_through(world, p, v) {
                        if !include_unpassable_boundary {
                            continue;
                        }
                        usable
                    } else {
                        u_dist + edge_cost_f32(world, p, u, v, d, consider_zoc)
                    };

                    let seen = sc.seen_gen[v] == gen;
                    if !seen || sc.dist_f32[v] > total_dist {
                        let stored = if total_dist < limit {
                            sc.wave_next.push(v as i32);
                            total_dist
                        } else {
                            usable
                        };
                        if !seen {
                            sc.seen_gen[v] = gen;
                            sc.reached_list.push(v as i32);
                        }
                        sc.dist_f32[v] = stored;
                        sc.parent_i32[v] = u as i32;
                    }
                }
            }
            std::mem::swap(&mut sc.wave_curr, &mut sc.wave_next);
        }

        let mut out = Vec::with_capacity(sc.reached_list.len() * 3);
        for &t in &sc.reached_list {
            let ti = t as usize;
            out.push(t);
            out.push(sc.parent_i32[ti]);
            out.push(sc.dist_f32[ti].to_bits() as i32);
        }
        out
    })
}

// ---------- RouteNode / PrioritizedNode packing helpers ----------

#[inline(always)]
fn pack_route_node(
    tile: usize,
    rel_ord: u8,
    move_since_stop: i32,
    move_this_turn: i32,
    turns: i32,
    damaging_self: bool,
    damaging_tiles: i32,
) -> u64 {
    let rel_bits = (7u64 - (rel_ord.min(7) as u64)) << REL_OFFSET;
    let mss = (move_since_stop.clamp(0, MAX_MOVE_THIS_TURN_FPM) as u64) << MOVE_SINCE_STOP_OFFSET;
    let mtt = (move_this_turn.clamp(0, MAX_MOVE_THIS_TURN_FPM) as u64) << MOVE_THIS_TURN_OFFSET;
    let trn = (turns.clamp(0, MAX_TURNS) as u64) << TURNS_OFFSET;
    let dmg_self = if damaging_self { DAMAGING_SELF_BIT } else { 0 };
    let dmg_t = (damaging_tiles.clamp(0, MAX_DAMAGING_TILES) as u64) << DAMAGING_TILES_OFFSET;
    (tile as u64 & TILE_IDX_MASK) | rel_bits | mss | mtt | trn | dmg_self | dmg_t
}

#[inline(always)]
fn rn_tile(bits: u64) -> usize {
    ((bits >> TILE_IDX_OFFSET) & TILE_IDX_MASK) as usize
}
#[inline(always)]
fn rn_move_since_stop(bits: u64) -> i32 {
    ((bits >> MOVE_SINCE_STOP_OFFSET) & MOVE_SINCE_STOP_MASK) as i32
}
#[inline(always)]
fn rn_can_stop_on(bits: u64) -> bool {
    rn_move_since_stop(bits) == 0
}
#[inline(always)]
fn rn_move_this_turn(bits: u64) -> i32 {
    ((bits >> MOVE_THIS_TURN_OFFSET) & MOVE_THIS_TURN_MASK) as i32
}
#[inline(always)]
fn rn_turns(bits: u64) -> i32 {
    ((bits >> TURNS_OFFSET) & TURNS_MASK) as i32
}
#[inline(always)]
fn rn_end_turn_without_damage(bits: u64) -> bool {
    (bits & DAMAGING_SELF_BIT) == 0
}
#[inline(always)]
fn rn_damaging_tiles(bits: u64) -> i32 {
    ((bits >> DAMAGING_TILES_OFFSET) & DAMAGING_TILES_MASK) as i32
}
#[inline(always)]
fn pack_prioritized(route_bits: u64, underestimated: i32) -> u64 {
    let u = underestimated.clamp(1, MAX_UNDERESTIMATED_FPM) as u64;
    (route_bits & !UNDEREST_HI_MASK) | (u << UNDEREST_OFFSET)
}

#[inline(always)]
fn tile_road_heuristic_fpm(world: &NativeWorld, civ: usize, tile: usize) -> i32 {
    let r = world.unpillaged_road(tile);
    if r == TILE_ROAD_RAILROAD {
        FPM_RAILROAD
    } else if r == TILE_ROAD_ROAD || world.forest_or_jungle_are_roads(civ, tile) {
        world.civ_road_speed_fpm[civ] as i32
    } else {
        FPM_BASE
    }
}

#[inline(always)]
fn calc_underestimated(
    world: &NativeWorld,
    civ: usize,
    node_bits: u64,
    full_move: i32,
    dest: Option<usize>,
) -> i32 {
    let tile = rn_tile(node_bits);
    let turns = rn_turns(node_bits);
    let used = rn_move_this_turn(node_bits).min(full_move);
    let movement_so_far = full_move * turns + used;
    let min_rem_tiles = match dest {
        Some(d) => world.aerial_distance(tile, d),
        None => 1,
    };
    let min_rem_cost = if min_rem_tiles <= 0 {
        0
    } else {
        tile_road_heuristic_fpm(world, civ, tile) + (min_rem_tiles - 1) * FPM_RAILROAD
    };
    (movement_so_far + min_rem_cost).clamp(1, MAX_UNDERESTIMATED_FPM)
}

#[inline]
fn find_damage_free_anchor(
    world: &NativeWorld,
    p: &UnitProfile,
    sc: &PathScratch,
    start_node: u64,
    first_hop_cost: i32,
    full_move: i32,
    consider_zoc: bool,
) -> Option<(u64, i32)> {
    let mut ancestor = start_node;
    let mut total_cost = first_hop_cost;
    loop {
        if total_cost >= full_move {
            return None;
        }
        if rn_can_stop_on(ancestor) && rn_end_turn_without_damage(ancestor) {
            return Some((ancestor, total_cost));
        }
        let anc_tile = rn_tile(ancestor);
        let par_tile = sc.route_parent[anc_tile];
        if par_tile < 0 || par_tile as usize == anc_tile {
            return None;
        }
        let par_u = par_tile as usize;
        // Find direction from par_u -> anc_tile
        let mut dir = 0usize;
        let base = par_u * 6;
        for d in 0..6 {
            if world.neighbors[base + d] == anc_tile as i32 {
                dir = d;
                break;
            }
        }
        ancestor = sc.route_nodes[par_u];
        let hop = edge_cost_fpm(world, p, par_u, anc_tile, dir, consider_zoc);
        total_cost = (total_cost + hop).min(MAX_MOVE_THIS_TURN_FPM);
    }
}

/// Core multi-turn A* node calculation matching `AStarPathfinder.calculateNeighborNode`.
#[inline]
fn calculate_neighbor_node(
    world: &NativeWorld,
    p: &UnitProfile,
    sc: &PathScratch,
    current_node: u64,
    from: usize,
    to: usize,
    dir_idx: usize,
    full_move: i32,
    is_unpassable_dest: bool,
) -> Option<u64> {
    let civ = p.civ_idx;
    let assume_unexplored = (p.flags & UP_ASSUME_UNEXPLORED_PASSABLE) != 0;
    let explored = world.is_explored(civ, to);
    let consider_zoc = (p.flags & UP_CONSIDER_ZOC) != 0;

    let (hop_cost, can_move_to, end_turn_dmg, rel_ord) = if is_unpassable_dest {
        // Classic behavior: moving onto an unpassable destination (e.g., enemy city/unit) uses remaining turn movement.
        (full_move, true, 0, DIPLO_REL_FAVORABLE)
    } else if assume_unexplored && !explored {
        (FPM_BASE.min(full_move), true, 0, DIPLO_REL_FAVORABLE)
    } else {
        let raw_cost = edge_cost_fpm(world, p, from, to, dir_idx, consider_zoc);
        let cost = raw_cost.min(full_move);
        let cmt = rn_turns(current_node) > 0 || can_move_to_assuming_pass_through(world, p, to);
        let dmg = if world.terrain_damage[to] > 0 { 1 } else { 0 };
        let rel = world.relationship_level(civ, world.owner_civ[to]);
        (cost, cmt, dmg, rel)
    };

    let neighbor_damaging = end_turn_dmg != 0;
    let true_safe = can_move_to && !neighbor_damaging;
    let damaging_tiles = rn_damaging_tiles(current_node);
    let move_since_stop = (rn_move_since_stop(current_node) + hop_cost).min(MAX_MOVE_THIS_TURN_FPM);
    let new_move_since_stop = if can_move_to { 0 } else { move_since_stop };
    let mid_turn = rn_move_this_turn(current_node) < full_move;
    let used_so_far = if mid_turn {
        rn_move_this_turn(current_node)
    } else {
        0
    };
    let new_used = (used_so_far + hop_cost).min(full_move);
    let this_turn_ok = new_used < full_move || true_safe;
    let cur_turns = rn_turns(current_node);

    if (mid_turn || rn_can_stop_on(current_node)) && this_turn_ok {
        let turns_delta = if mid_turn { 0 } else { 1 };
        return Some(pack_route_node(
            to,
            rel_ord,
            new_move_since_stop,
            new_used,
            cur_turns + turns_delta,
            neighbor_damaging,
            damaging_tiles,
        ));
    }

    if !can_move_to && move_since_stop < full_move {
        let cur_dmg = if world.terrain_damage[from] > 0 { 1 } else { 0 };
        let retreat_dmg = if rn_can_stop_on(current_node) {
            (damaging_tiles + cur_dmg).min(MAX_DAMAGING_TILES)
        } else {
            damaging_tiles
        };
        if (p.flags & UP_AVOID_DAMAGING_TERRAIN) != 0 && retreat_dmg > 0 {
            return None;
        }
        let turns_delta = if rn_can_stop_on(current_node) { 1 } else { 0 };
        return Some(pack_route_node(
            to,
            rel_ord,
            move_since_stop,
            move_since_stop,
            cur_turns + turns_delta,
            neighbor_damaging,
            retreat_dmg,
        ));
    }

    if !can_move_to {
        return None;
    }

    // Can stop here, but it's damaging and we ran out of movement budget.
    // Look backward for a damage-free anchor to pause on instead.
    if let Some((anchor, anchor_cost)) =
        find_damage_free_anchor(world, p, sc, current_node, hop_cost, full_move, consider_zoc)
    {
        return Some(pack_route_node(
            to,
            rel_ord,
            0,
            anchor_cost,
            rn_turns(anchor) + 1,
            neighbor_damaging,
            rn_damaging_tiles(anchor),
        ));
    }

    if (p.flags & UP_AVOID_DAMAGING_TERRAIN) != 0 {
        return None;
    }

    let new_dmg_tiles = (damaging_tiles + end_turn_dmg).min(MAX_DAMAGING_TILES);
    Some(pack_route_node(
        to,
        rel_ord,
        0,
        hop_cost,
        cur_turns + 1,
        neighbor_damaging,
        new_dmg_tiles,
    ))
}

/// Reconstructs turn-by-turn end-of-turn waypoints matching `PathingMap.pathAsListNoDamage`
/// and `PathingMap.pathAsListWithDamage`.
fn reconstruct_waypoints(
    world: &NativeWorld,
    p: &UnitProfile,
    sc: &PathScratch,
    start_tile: usize,
    dest_tile: usize,
    full_move: i32,
) -> Vec<i32> {
    let target_node = sc.route_nodes[dest_tile];
    if rn_damaging_tiles(target_node) == 0 {
        let mut res = vec![dest_tile as i32];
        let mut cur_tile = dest_tile;
        let mut turns = rn_turns(target_node);
        let mut guard = 0usize;
        while cur_tile != start_tile && guard <= world.num_tiles {
            guard += 1;
            let par = sc.route_parent[cur_tile];
            if par < 0 || par as usize == start_tile {
                break;
            }
            let par_u = par as usize;
            let par_node = sc.route_nodes[par_u];
            if rn_turns(par_node) < turns && rn_end_turn_without_damage(par_node) {
                res.push(par);
                turns = rn_turns(par_node);
            }
            cur_tile = par_u;
        }
        res.reverse();
        return res;
    }

    // Damaging route reconstruction (matches PathingMap.pathAsListWithDamage)
    let mut chain = Vec::with_capacity(32);
    let mut walk = dest_tile;
    let mut guard = 0usize;
    while walk != start_tile && guard <= world.num_tiles {
        guard += 1;
        chain.push(walk);
        let par = sc.route_parent[walk];
        if par < 0 {
            break;
        }
        walk = par as usize;
    }
    chain.reverse();

    let consider_zoc = (p.flags & UP_CONSIDER_ZOC) != 0;
    let mut result = Vec::with_capacity(16);
    let mut add_waypoint = |t: usize| {
        if t != start_tile && result.last().copied() != Some(t as i32) {
            result.push(t as i32);
        }
    };
    let mut move_this_turn = 0i32;
    let mut prev_tile = start_tile;
    let mut prev_node = sc.route_nodes[start_tile];
    let mut last_full_safe = start_tile;
    let mut move_since_full_safe = 0i32;
    let mut last_stoppable = start_tile;
    let mut move_since_stoppable = 0i32;

    for &next_tile in &chain {
        let next_node = sc.route_nodes[next_tile];
        let move_cost = if !rn_end_turn_without_damage(next_node) {
            let mut dir = 0usize;
            let base = prev_tile * 6;
            for d in 0..6 {
                if world.neighbors[base + d] == next_tile as i32 {
                    dir = d;
                    break;
                }
            }
            edge_cost_fpm(world, p, prev_tile, next_tile, dir, consider_zoc).min(full_move)
        } else if rn_turns(next_node) == rn_turns(prev_node) {
            rn_move_this_turn(next_node) - rn_move_this_turn(prev_node)
        } else {
            rn_move_this_turn(next_node)
        };

        if rn_end_turn_without_damage(prev_node) && !rn_end_turn_without_damage(next_node) {
            last_full_safe = prev_tile;
            move_since_full_safe = 0;
        }
        if rn_can_stop_on(prev_node) && !rn_can_stop_on(next_node) {
            last_stoppable = prev_tile;
            move_since_stoppable = 0;
        }
        move_since_full_safe += move_cost;
        move_since_stoppable += move_cost;
        move_this_turn += move_cost;

        if move_this_turn > full_move {
            let end_turn_tile = if move_since_full_safe < move_this_turn {
                move_this_turn = move_since_full_safe;
                last_full_safe
            } else if move_since_stoppable < move_this_turn {
                move_this_turn = move_since_stoppable;
                last_stoppable
            } else {
                move_this_turn = move_cost;
                prev_tile
            };
            add_waypoint(end_turn_tile);
            last_full_safe = end_turn_tile;
            last_stoppable = end_turn_tile;
            move_since_full_safe = move_this_turn;
            move_since_stoppable = move_this_turn;
        }
        prev_tile = next_tile;
        prev_node = next_node;
    }
    add_waypoint(dest_tile);
    result
}

/// Fast rules-preserving continent & destination-surrounded pre-checks.
#[inline]
fn quick_unreachable_check(
    world: &NativeWorld,
    p: &UnitProfile,
    start_tile: usize,
    dest_tile: usize,
) -> bool {
    let civ = p.civ_idx;
    let cflags = world.civ_flags[civ];
    let assume_unexplored = (p.flags & UP_ASSUME_UNEXPLORED_PASSABLE) != 0;

    if (p.flags & UP_AVOID_ENEMY_TERRITORY) != 0 {
        let owner = world.owner_civ[dest_tile];
        if owner >= 0 && world.is_at_war(civ, owner as usize) {
            return true;
        }
    }

    // If strict A* mode (cannot end on unpassable dest) and dest is explored & unpassable
    if (p.flags & UP_ALLOW_UNPASSABLE_DEST) == 0
        && !(assume_unexplored && !world.is_explored(civ, dest_tile))
        && !can_pass_through(world, p, dest_tile)
    {
        return true;
    }

    // Continent pre-check for non-embarking land units
    if (p.flags & UP_IS_LAND) != 0
        && (p.flags & UP_CAN_MOVE_ON_WATER) == 0
        && ((cflags & CF_CAN_EMBARK) == 0 || (p.flags & UP_CANNOT_EMBARK) != 0)
        && (p.flags & UP_CAN_PASS_IMPASSABLE) == 0
        && (cflags & CF_PASS_IMPASSABLE) == 0
        && world.is_explored(civ, dest_tile)
        && (world.terrain_flags[dest_tile] & TF_LAND) != 0
    {
        let sc = world.continent[start_tile];
        let dc = world.continent[dest_tile];
        if sc >= 0 && dc >= 0 && sc != dc {
            return true;
        }
    }

    // Destination-surrounded O(6) check
    let base = dest_tile * 6;
    let mut any_entry = false;
    for d in 0..6 {
        let nb = world.neighbors[base + d];
        if nb < 0 {
            continue;
        }
        let nbu = nb as usize;
        if nbu == start_tile
            || (assume_unexplored && !world.is_explored(civ, nbu))
            || can_pass_through(world, p, nbu)
        {
            any_entry = true;
            break;
        }
    }
    !any_entry
}

/// Multi-turn A* shortest path returning turn-by-turn waypoint tile indices (excluding start,
/// except `[start]` when `start == dest`). Returns empty Vec if unreachable.
pub fn find_shortest_path(
    world: &mut NativeWorld,
    p: &UnitProfile,
    start_tile: usize,
    dest_tile: usize,
    max_turns: i32,
) -> Vec<i32> {
    if start_tile >= world.num_tiles || dest_tile >= world.num_tiles {
        return Vec::new();
    }
    if (p.flags & UP_CANNOT_MOVE) != 0 {
        return Vec::new();
    }
    if start_tile == dest_tile {
        return vec![start_tile as i32];
    }
    if p.current_move_fpm <= 0 || p.full_move_fpm <= 0 {
        return Vec::new();
    }

    if (p.flags & UP_CONSIDER_ZOC) != 0 && (p.flags & UP_IGNORES_ZOC) == 0 {
        world.ensure_zoc(p.civ_idx);
    }

    if quick_unreachable_check(world, p, start_tile, dest_tile) {
        return Vec::new();
    }

    let (path, hit_unstoppable) =
        find_shortest_path_inner(world, p, start_tile, dest_tile, max_turns, true);
    if !path.is_empty() || !hit_unstoppable {
        return path;
    }
    // Rare allied-unit Turn-0 wall case: retry with pure Dijkstra order (identical to batch_first_reachable)
    find_shortest_path_inner(world, p, start_tile, dest_tile, max_turns, false).0
}

fn find_shortest_path_inner(
    world: &NativeWorld,
    p: &UnitProfile,
    start_tile: usize,
    dest_tile: usize,
    max_turns: i32,
    use_heuristic: bool,
) -> (Vec<i32>, bool) {
    let time_limit = max_turns.clamp(1, MAX_TURNS - 1);
    let full_move = p.full_move_fpm.min(MAX_MOVE_THIS_TURN_FPM);
    let move_used_init = (full_move - p.current_move_fpm).clamp(0, MAX_MOVE_THIS_TURN_FPM);
    let civ = p.civ_idx;
    let assume_unexplored = (p.flags & UP_ASSUME_UNEXPLORED_PASSABLE) != 0;
    let avoid_enemy_civ = (p.flags & UP_AVOID_ENEMY_TERRITORY) != 0;
    let allow_unpassable_dest = (p.flags & UP_ALLOW_UNPASSABLE_DEST) != 0;
    let heur_dest = if use_heuristic { Some(dest_tile) } else { None };

    with_scratch(|sc| {
        let gen = sc.next_gen(world.num_tiles);
        let root = pack_route_node(
            start_tile,
            DIPLO_REL_FAVORABLE,
            0,
            move_used_init,
            0,
            false,
            0,
        );
        sc.seen_gen[start_tile] = gen;
        sc.route_nodes[start_tile] = root;
        sc.route_parent[start_tile] = start_tile as i32;
        sc.todo_gen[start_tile] = gen;
        sc.todo_dmg[start_tile] = 0;

        let init_est = calc_underestimated(world, civ, root, full_move, heur_dest);
        sc.heap_push(pack_prioritized(root, init_est));

        let mut hit_unstoppable = false;

        while let Some(popped) = sc.heap_pop() {
            let u = rn_tile(popped);
            let cur_node = sc.route_nodes[u];
            // Skip stale heap entry if a strictly better route to `u` superseded it
            if (popped & !UNDEREST_HI_MASK) != (cur_node & !UNDEREST_HI_MASK) {
                continue;
            }
            if sc.closed_gen[u] == gen && sc.todo_gen[u] != gen {
                continue;
            }
            sc.todo_gen[u] = 0;
            sc.closed_gen[u] = gen;

            if u == dest_tile {
                return (
                    reconstruct_waypoints(world, p, sc, start_tile, dest_tile, full_move),
                    hit_unstoppable,
                );
            }

            let nt_u = rn_turns(cur_node);
            let nu_u = rn_move_this_turn(cur_node);
            if !(nt_u < time_limit - 1 || (nt_u == time_limit - 1 && nu_u < full_move)) {
                continue;
            }

            let cur_dmg = rn_damaging_tiles(cur_node);
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;

                // If `v` was already closed with <= cur_dmg, we cannot improve it
                if sc.closed_gen[v] == gen && rn_damaging_tiles(sc.route_nodes[v]) <= cur_dmg {
                    continue;
                }

                // Civilian automated avoids enemy territory
                if avoid_enemy_civ {
                    let owner = world.owner_civ[v];
                    if owner >= 0 && world.is_at_war(civ, owner as usize) {
                        continue;
                    }
                }

                let explored = world.is_explored(civ, v);
                let passable = (assume_unexplored && !explored) || can_pass_through(world, p, v);
                let is_unpassable_dest = !passable && allow_unpassable_dest && v == dest_tile;
                if !passable && !is_unpassable_dest {
                    sc.closed_gen[v] = gen;
                    sc.seen_gen[v] = gen;
                    sc.route_nodes[v] = pack_route_node(
                        v,
                        0,
                        MAX_MOVE_THIS_TURN_FPM,
                        MAX_MOVE_THIS_TURN_FPM,
                        MAX_TURNS,
                        false,
                        MAX_DAMAGING_TILES,
                    );
                    continue;
                }

                let Some(new_node) = calculate_neighbor_node(
                    world,
                    p,
                    sc,
                    cur_node,
                    u,
                    v,
                    d,
                    full_move,
                    is_unpassable_dest,
                ) else {
                    hit_unstoppable = true;
                    continue;
                };

                // Relax only if unseen or strictly better (single u64 comparison on RouteNode key!)
                if sc.seen_gen[v] == gen && new_node >= sc.route_nodes[v] {
                    continue;
                }

                sc.seen_gen[v] = gen;
                sc.route_nodes[v] = new_node;
                sc.route_parent[v] = u as i32;

                let nt = rn_turns(new_node);
                let nu = rn_move_this_turn(new_node);
                let can_expand = nt < time_limit - 1 || (nt == time_limit - 1 && nu < full_move);
                if v == dest_tile || can_expand {
                    let est = calc_underestimated(world, civ, new_node, full_move, heur_dest);
                    sc.heap_push(pack_prioritized(new_node, est));
                    sc.todo_gen[v] = gen;
                    sc.todo_dmg[v] = rn_damaging_tiles(new_node) as u8;
                }
            }
        }

        (Vec::new(), hit_unstoppable)
    })
}

/// Single-expansion batch reachability query!
///
/// Given a priority-ordered slice of `candidates`, finds the **lowest-index** candidate that is
/// reachable within `max_turns` using ONE Dijkstra expansion instead of K separate A* searches.
/// Returns `best_index` (`-1` if none reachable) followed by the waypoint path to that candidate!
pub fn batch_first_reachable(
    world: &mut NativeWorld,
    p: &UnitProfile,
    start_tile: usize,
    candidates: &[i32],
    max_turns: i32,
) -> (i32, Vec<i32>) {
    if candidates.is_empty() || start_tile >= world.num_tiles || (p.flags & UP_CANNOT_MOVE) != 0 {
        return (-1, Vec::new());
    }
    if p.current_move_fpm <= 0 || p.full_move_fpm <= 0 {
        // Check if start_tile itself is a candidate
        for (idx, &c) in candidates.iter().enumerate() {
            if c as usize == start_tile {
                return (idx as i32, vec![start_tile as i32]);
            }
        }
        return (-1, Vec::new());
    }
    if candidates.len() == 1 {
        let dest = candidates[0] as usize;
        let path = find_shortest_path(world, p, start_tile, dest, max_turns);
        return if path.is_empty() { (-1, path) } else { (0, path) };
    }

    if (p.flags & UP_CONSIDER_ZOC) != 0 && (p.flags & UP_IGNORES_ZOC) == 0 {
        world.ensure_zoc(p.civ_idx);
    }

    // Check if candidate 0 is start_tile
    if candidates[0] as usize == start_tile {
        return (0, vec![start_tile as i32]);
    }

    let time_limit = max_turns.clamp(1, MAX_TURNS - 1);
    let full_move = p.full_move_fpm.min(MAX_MOVE_THIS_TURN_FPM);
    let move_used_init = (full_move - p.current_move_fpm).clamp(0, MAX_MOVE_THIS_TURN_FPM);
    let civ = p.civ_idx;
    let assume_unexplored = (p.flags & UP_ASSUME_UNEXPLORED_PASSABLE) != 0;
    let avoid_enemy_civ = (p.flags & UP_AVOID_ENEMY_TERRITORY) != 0;
    let allow_unpassable_dest = (p.flags & UP_ALLOW_UNPASSABLE_DEST) != 0;

    with_scratch(|sc| {
        let mgen = sc.next_mark(world.num_tiles);
        let mut any_valid_cand = false;
        let mut best_idx = i32::MAX;
        for (i, &c) in candidates.iter().enumerate() {
            if c < 0 || c as usize >= world.num_tiles {
                continue;
            }
            let cu = c as usize;
            if cu == start_tile {
                if (i as i32) < best_idx {
                    best_idx = i as i32;
                }
                continue;
            }
            if quick_unreachable_check(world, p, start_tile, cu) {
                continue;
            }
            // Keep lowest (highest-priority) candidate index per tile
            if sc.mark[cu] != mgen {
                sc.mark[cu] = mgen;
                sc.mark_aux[cu] = i as i32;
                any_valid_cand = true;
            }
        }
        if best_idx == 0 {
            return (0, vec![start_tile as i32]);
        }
        if !any_valid_cand {
            return if best_idx < i32::MAX {
                (best_idx, vec![start_tile as i32])
            } else {
                (-1, Vec::new())
            };
        }

        let gen = sc.next_gen(world.num_tiles);
        let root = pack_route_node(
            start_tile,
            DIPLO_REL_FAVORABLE,
            0,
            move_used_init,
            0,
            false,
            0,
        );
        sc.seen_gen[start_tile] = gen;
        sc.route_nodes[start_tile] = root;
        sc.route_parent[start_tile] = start_tile as i32;
        sc.todo_gen[start_tile] = gen;
        sc.todo_dmg[start_tile] = 0;

        let init_est = calc_underestimated(world, civ, root, full_move, None);
        sc.heap_push(pack_prioritized(root, init_est));

        while let Some(popped) = sc.heap_pop() {
            let u = rn_tile(popped);
            let cur_node = sc.route_nodes[u];
            if (popped & !UNDEREST_HI_MASK) != (cur_node & !UNDEREST_HI_MASK) {
                continue;
            }
            if sc.closed_gen[u] == gen && sc.todo_gen[u] != gen {
                continue;
            }
            sc.todo_gen[u] = 0;
            sc.closed_gen[u] = gen;

            let nt_u = rn_turns(cur_node);
            let nu_u = rn_move_this_turn(cur_node);
            if !(nt_u < time_limit - 1 || (nt_u == time_limit - 1 && nu_u < full_move)) {
                continue;
            }

            let cur_dmg = rn_damaging_tiles(cur_node);
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;

                if sc.closed_gen[v] == gen && rn_damaging_tiles(sc.route_nodes[v]) <= cur_dmg {
                    continue;
                }
                if avoid_enemy_civ {
                    let owner = world.owner_civ[v];
                    if owner >= 0 && world.is_at_war(civ, owner as usize) {
                        continue;
                    }
                }

                let is_cand = sc.mark[v] == mgen;
                let explored = world.is_explored(civ, v);
                let passable = (assume_unexplored && !explored) || can_pass_through(world, p, v);
                let is_unpassable_dest = !passable && allow_unpassable_dest && is_cand;
                if !passable && !is_unpassable_dest {
                    sc.closed_gen[v] = gen;
                    sc.seen_gen[v] = gen;
                    sc.route_nodes[v] = pack_route_node(v, 0, MAX_MOVE_THIS_TURN_FPM, MAX_MOVE_THIS_TURN_FPM, MAX_TURNS, false, MAX_DAMAGING_TILES);
                    continue;
                }

                let Some(new_node) = calculate_neighbor_node(
                    world,
                    p,
                    sc,
                    cur_node,
                    u,
                    v,
                    d,
                    full_move,
                    is_unpassable_dest,
                ) else {
                    continue;
                };

                if sc.seen_gen[v] == gen && new_node >= sc.route_nodes[v] {
                    continue;
                }

                sc.seen_gen[v] = gen;
                sc.route_nodes[v] = new_node;
                sc.route_parent[v] = u as i32;

                if is_cand {
                    let ci = sc.mark_aux[v];
                    if ci < best_idx {
                        best_idx = ci;
                        if best_idx == 0 {
                            let path = reconstruct_waypoints(world, p, sc, start_tile, v, full_move);
                            return (0, path);
                        }
                    }
                }

                if is_unpassable_dest {
                    // Cannot expand past an unpassable destination
                    sc.closed_gen[v] = gen;
                    continue;
                }

                let nt = rn_turns(new_node);
                let nu = rn_move_this_turn(new_node);
                if nt < time_limit - 1 || (nt == time_limit - 1 && nu < full_move) {
                    let est = calc_underestimated(world, civ, new_node, full_move, None);
                    sc.heap_push(pack_prioritized(new_node, est));
                    sc.todo_gen[v] = gen;
                    sc.todo_dmg[v] = rn_damaging_tiles(new_node) as u8;
                }
            }
        }

        if best_idx < i32::MAX {
            let dest = candidates[best_idx as usize] as usize;
            let path = if dest == start_tile {
                vec![start_tile as i32]
            } else {
                reconstruct_waypoints(world, p, sc, start_tile, dest, full_move)
            };
            (best_idx, path)
        } else {
            (-1, Vec::new())
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a hexagonal grid of radius `r` matching Unciv's axial `(x, y)` coordinates
    /// where clock 2=(0,1), 4=(-1,0), 6=(-1,-1), 8=(0,-1), 10=(1,0), 12=(1,1).
    pub fn make_hex_world(radius: i32, num_civs: usize) -> (NativeWorld, std::collections::HashMap<(i32, i32), usize>) {
        let mut coords = Vec::new();
        let mut map = std::collections::HashMap::new();
        for x in -radius..=radius {
            for y in -radius..=radius {
                if (x - y).abs() <= radius {
                    let idx = coords.len();
                    coords.push((x, y));
                    map.insert((x, y), idx);
                }
            }
        }
        let n = coords.len();
        let mut w = NativeWorld::new(n, num_civs, false, radius, radius * 2 + 1);
        let dirs = [(0, 1), (-1, 0), (-1, -1), (0, -1), (1, 0), (1, 1)];
        for (i, &(x, y)) in coords.iter().enumerate() {
            w.pos_x[i] = x as i16;
            w.pos_y[i] = y as i16;
            w.terrain_flags[i] = TF_LAND;
            w.base_move_cost[i] = 1;
            w.continent[i] = 0;
            for (d, &(dx, dy)) in dirs.iter().enumerate() {
                if let Some(&nb) = map.get(&(x + dx, y + dy)) {
                    w.neighbors[i * 6 + d] = nb as i32;
                }
            }
            for c in 0..num_civs {
                w.set_explored_bit(c, i, true);
                w.set_visible_bit(c, i, true);
            }
        }
        for c in 0..num_civs {
            w.diplo_matrix[c * num_civs + c] = DIPLO_CAN_PASS | DIPLO_OPEN_BORDERS | DIPLO_FRIENDLY_TERRITORY | (DIPLO_REL_FAVORABLE << DIPLO_REL_SHIFT);
        }
        (w, map)
    }

    fn default_land_profile(civ: usize, move_pts: i32) -> UnitProfile {
        UnitProfile {
            civ_idx: civ,
            flags: UP_IS_LAND
                | UP_NO_TERRAIN_MOVEMENT_UNIQUES
                | UP_NO_BASE_OR_HILL_DM
                | UP_CONSIDER_ZOC
                | UP_ALLOW_UNPASSABLE_DEST
                | UP_ASSUME_UNEXPLORED_PASSABLE,
            current_move_f32: move_pts as f32,
            current_move_fpm: move_pts * FPM_BASE,
            full_move_fpm: move_pts * FPM_BASE,
            cost_to_embark_fpm: -1,
            cost_to_disembark_fpm: -1,
            enemy_penalty_fpm: 0,
            dm_feature_mask: 0,
            dm_base_mask: 0,
            dm_hill_mask: 0,
        }
    }

    #[test]
    fn shortest_path_with_hills_and_turns() {
        let (mut w, m) = make_hex_world(5, 2);
        // Hills at (0,1) and (0,2), movement = 1 => through hills takes 3 turns, around takes 4
        for pos in [(0, 1), (0, 2)] {
            let idx = m[&pos];
            w.terrain_flags[idx] |= TF_HILL | TF_ROUGH;
            w.base_move_cost[idx] = 2;
        }
        let p = default_land_profile(0, 1);
        let path = find_shortest_path(&mut w, &p, m[&(0, 0)], m[&(0, 3)], 62);
        assert_eq!(path.len(), 3);
        assert_eq!(*path.last().unwrap(), m[&(0, 3)] as i32);
    }

    #[test]
    fn retroactive_pause_before_mountains() {
        let (mut w, m) = make_hex_world(8, 2);
        // Make everything a damaging mountain except (0,0), (0,1), (0,4), (0,5), (0,8)
        for t in 0..w.num_tiles {
            w.terrain_flags[t] = TF_LAND | TF_IMPASSABLE;
            w.terrain_damage[t] = 50;
            w.last_terrain_id[t] = 5; // Mountain id = 5
            w.base_move_cost[t] = 1;
        }
        for pos in [(0, 0), (0, 1), (0, 4), (0, 5), (0, 8)] {
            let t = m[&pos];
            w.terrain_flags[t] = TF_LAND;
            w.terrain_damage[t] = 0;
            w.last_terrain_id[t] = 1;
        }
        w.civ_flags[0] |= CF_PASS_IMPASSABLE;
        w.civ_passable_impassable_mask[0] |= 1u64 << 5;
        let mut p = default_land_profile(0, 3);
        p.flags |= UP_AVOID_DAMAGING_TERRAIN;
        let path = find_shortest_path(&mut w, &p, m[&(0, 0)], m[&(0, 8)], 62);
        let expected = vec![m[&(0, 1)] as i32, m[&(0, 4)] as i32, m[&(0, 5)] as i32, m[&(0, 8)] as i32];
        assert_eq!(path, expected);
    }

    #[test]
    fn zoc_stops_movement() {
        let (mut w, m) = make_hex_world(4, 2);
        // Civ 0 and Civ 1 at war; enemy military at (0,1)
        w.diplo_matrix[0 * 2 + 1] |= DIPLO_AT_WAR;
        w.diplo_matrix[1 * 2 + 0] |= DIPLO_AT_WAR;
        w.mil_unit_civ[m[&(0, 1)]] = 1;
        // Block the non-ZoC detour at (1,0) so (1,1) -> (0,0) must cross ZoC directly
        w.terrain_flags[m[&(1, 0)]] |= TF_IMPASSABLE;
        let p = default_land_profile(0, 3);
        let triples = compute_distance_within_turn(&mut w, &p, m[&(1, 1)], true);
        let mut cost_0_0 = -1.0f32;
        for chunk in triples.chunks_exact(3) {
            if chunk[0] == m[&(0, 0)] as i32 {
                cost_0_0 = f32::from_bits(chunk[2] as u32);
            }
        }
        assert!((cost_0_0 - 3.0).abs() < 1e-4, "expected ZoC cost 3.0, got {cost_0_0}");
    }

    #[test]
    fn batch_first_reachable_skips_walled_candidate() {
        let (mut w, m) = make_hex_world(5, 2);
        // Surround (2,0) with mountains
        for d in 0..6 {
            let nb = w.neighbors[m[&(2, 0)] * 6 + d];
            if nb >= 0 {
                w.terrain_flags[nb as usize] |= TF_IMPASSABLE;
            }
        }
        let p = default_land_profile(0, 2);
        let cands = [m[&(2, 0)] as i32, m[&(-2, 0)] as i32, m[&(-3, 0)] as i32];
        let (idx, path) = batch_first_reachable(&mut w, &p, m[&(0, 0)], &cands, 62);
        assert_eq!(idx, 1);
        assert_eq!(*path.last().unwrap(), m[&(-2, 0)] as i32);
    }
}
