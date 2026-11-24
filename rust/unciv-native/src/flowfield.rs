//! O(V) Multi-Source Reverse-Dijkstra Theater Flow-Fields & Spatial Helpers.
//!
//! Ported from HexForge-Rust's `crates/sim/src/ai/theater.rs` architecture and adapted to
//! Unciv's exact hex topology, diplomacy passability, ocean/embark rules, and roads.
//!
//! Computes all 5 strategic flow-fields for the active AI civ in parallel via our bounded
//! Rayon pool so every unit gets O(1) strategic direction queries instead of O(U × A*) searches!

use crate::scratch::with_scratch;
use crate::world::*;

/// Edge cost weight (scaled integers: road=1, flat=2, rough/hill=4, embark/water-crossing=6).
const COST_ROAD: u16 = 1;
const COST_FLAT: u16 = 2;
const COST_ROUGH: u16 = 4;
const COST_WATER_EMBARK: u16 = 6;
const COST_WATER_NAVAL: u16 = 2;

/// Compute a single reverse-Dijkstra flow-field into `(dist_out, next_out)` using thread-local scratch.
///
/// For any tile `t` with `dist_out[t] < FLOW_UNREACHED`:
/// - `next_out[t]` is the neighbor tile that steps closer to the nearest goal seed.
/// - `dist_out[t]` is the weighted cost to reach the nearest goal seed.
fn compute_reverse_flow_field(
    world: &NativeWorld,
    civ_idx: usize,
    seeds: &[i32],
    is_water_field: bool,
    max_cost: u16,
    dist_out: &mut [u16],
    next_out: &mut [i32],
) {
    let n = world.num_tiles;
    dist_out.fill(FLOW_UNREACHED);
    next_out.fill(-1);
    if seeds.is_empty() || civ_idx >= world.num_civs || n == 0 {
        return;
    }

    let cflags = world.civ_flags[civ_idx];
    let can_embark = (cflags & CF_CAN_EMBARK) != 0;
    let embark_ocean = (cflags & CF_EMBARK_OCEAN) != 0;
    let all_ocean = (cflags & CF_ALL_OCEAN) != 0;
    let pass_impassable = (cflags & CF_PASS_IMPASSABLE) != 0;
    let pass_impassable_mask = world.civ_passable_impassable_mask[civ_idx];

    with_scratch(|sc| {
        let gen = sc.next_gen(n);

        for &s in seeds {
            if s < 0 || (s as usize) >= n {
                continue;
            }
            let su = s as usize;
            if sc.seen_gen[su] == gen {
                continue;
            }
            sc.seen_gen[su] = gen;
            dist_out[su] = 0;
            next_out[su] = s;
            sc.heap_push(su as u64);
        }

        while let Some(packed) = sc.heap_pop() {
            let cost_u = (packed >> 32) as u16;
            let u = (packed & 0xFFFF_FFFF) as usize;
            if cost_u > dist_out[u] || cost_u >= max_cost {
                continue;
            }

            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;
                if !world.is_explored(civ_idx, v) {
                    continue;
                }

                // Check if a unit standing on `v` can pass through `v` and step into `u`
                let tf = world.terrain_flags[v];
                if (tf & TF_IMPASSABLE) != 0 {
                    let allowed = pass_impassable
                        || (pass_impassable_mask != 0
                            && (world.feature_terrain_mask[v] & pass_impassable_mask) != 0);
                    if !allowed {
                        continue;
                    }
                }
                if (tf & TF_ICE) != 0 {
                    continue;
                }
                if !world.can_civ_pass_through(civ_idx, v) {
                    continue;
                }

                let step_cost = if is_water_field {
                    // Naval units travel on water (or coastal friendly city centers)
                    let is_water = (tf & TF_WATER) != 0;
                    if !is_water {
                        let is_friendly_city = (world.tile_flags[v] & TILE_CITY_CENTER) != 0
                            && world.owner_civ[v] == civ_idx as i16;
                        if !is_friendly_city {
                            continue;
                        }
                    }
                    if (tf & TF_OCEAN) != 0 && !all_ocean {
                        continue;
                    }
                    COST_WATER_NAVAL
                } else {
                    // Land units travel on land, or embark on water if tech allows
                    let is_water = (tf & TF_WATER) != 0;
                    if is_water {
                        if !can_embark {
                            continue;
                        }
                        if (tf & TF_OCEAN) != 0 && !embark_ocean && !all_ocean {
                            continue;
                        }
                        COST_WATER_EMBARK
                    } else if world.has_connection(civ_idx, v) && world.has_connection(civ_idx, u) {
                        COST_ROAD
                    } else if (tf & (TF_ROUGH | TF_HILL)) != 0 || world.base_move_cost[v] > 1 {
                        COST_ROUGH
                    } else {
                        COST_FLAT
                    }
                };

                let next_cost = cost_u.saturating_add(step_cost);
                if next_cost < dist_out[v] && next_cost <= max_cost {
                    dist_out[v] = next_cost;
                    next_out[v] = u as i32;
                    sc.seen_gen[v] = gen;
                    sc.heap_push(((next_cost as u64) << 32) | (v as u64));
                }
            }
        }
    });
}

/// Collect safe unexplored frontier tiles (explored passable tiles adjacent to at least one
/// unexplored tile, outside enemy danger bits) for `civ_idx` in a single O(V) pass.
fn collect_explore_frontier_seeds(world: &NativeWorld, civ_idx: usize) -> Vec<i32> {
    let n = world.num_tiles;
    let is_cs = (world.civ_flags[civ_idx] & CF_IS_CITY_STATE) != 0;
    let mut seeds = Vec::with_capacity(64);

    for t in 0..n {
        if !world.is_explored(civ_idx, t) {
            continue;
        }
        let tf = world.terrain_flags[t];
        if (tf & (TF_IMPASSABLE | TF_ICE)) != 0 || world.terrain_damage[t] > 0 {
            continue;
        }
        // Skip tiles owned by city-states or in enemy danger zone
        let owner = world.owner_civ[t];
        if owner >= 0 {
            let ou = owner as usize;
            if (world.civ_flags[ou] & CF_IS_CITY_STATE) != 0 && ou != civ_idx {
                continue;
            }
        }
        if is_cs && owner != civ_idx as i16 {
            continue;
        }
        if ((world.threat_danger_bits[t >> 6] >> (t & 63)) & 1) != 0 {
            continue;
        }
        if !world.can_civ_pass_through(civ_idx, t) {
            continue;
        }

        // Check if any immediate neighbor is unexplored
        let base = t * 6;
        let mut adj_unexplored = false;
        for d in 0..6 {
            let nb = world.neighbors[base + d];
            if nb >= 0 && !world.is_explored(civ_idx, nb as usize) {
                adj_unexplored = true;
                break;
            }
        }
        if adj_unexplored {
            seeds.push(t as i32);
        }
    }
    seeds
}

/// Build all 5 O(V) Theater Flow-Fields for `civ_idx` in parallel via our bounded Rayon pool!
///
/// - `enemy_city_tiles`: prioritized enemy city centers (at war with `civ_idx`).
/// - `sieged_city_tiles`: own cities with `hp < max_hp`.
/// - `own_city_tiles`: all own cities (for retreat / garrison guidance).
pub fn build_civ_flow_fields(
    world: &mut NativeWorld,
    civ_idx: usize,
    enemy_city_tiles: &[i32],
    sieged_city_tiles: &[i32],
    own_city_tiles: &[i32],
) {
    if civ_idx >= world.num_civs || world.num_tiles == 0 {
        return;
    }
    let n = world.num_tiles;

    // Collect water attack stations adjacent to enemy cities (within range 2 water) for naval flow-field
    let mut naval_seeds: Vec<i32> = Vec::with_capacity(enemy_city_tiles.len() * 6);
    for &ec in enemy_city_tiles {
        if ec < 0 || (ec as usize) >= n {
            continue;
        }
        let base = (ec as usize) * 6;
        for d in 0..6 {
            let nb = world.neighbors[base + d];
            if nb >= 0 {
                let nu = nb as usize;
                if (world.terrain_flags[nu] & TF_WATER) != 0 {
                    naval_seeds.push(nb);
                }
                // Also include range-2 coastal water tiles for ranged ships
                let base2 = nu * 6;
                for d2 in 0..6 {
                    let nb2 = world.neighbors[base2 + d2];
                    if nb2 >= 0 && (world.terrain_flags[nb2 as usize] & TF_WATER) != 0 {
                        naval_seeds.push(nb2);
                    }
                }
            }
        }
    }

    let explore_seeds = collect_explore_frontier_seeds(world, civ_idx);

    // Temporarily move FlowFieldCache out of `world` so Rayon threads can borrow `&NativeWorld`
    // read-only while populating all 5 fields concurrently with zero locking!
    let mut cache = std::mem::replace(&mut world.flow_fields, FlowFieldCache::new(0));
    if cache.enemy_city_land_dist.len() != n {
        cache = FlowFieldCache::new(n);
    }

    let w_ref: &NativeWorld = world;
    let FlowFieldCache {
        ref mut enemy_city_land_dist,
        ref mut enemy_city_land_next,
        ref mut enemy_city_water_dist,
        ref mut enemy_city_water_next,
        ref mut friendly_city_dist,
        ref mut friendly_city_next,
        ref mut sieged_city_dist,
        ref mut sieged_city_next,
        ref mut explore_frontier_dist,
        ref mut explore_frontier_next,
        ..
    } = cache;

    with_rayon_pool(|| {
        rayon::join(
            || {
                rayon::join(
                    || {
                        compute_reverse_flow_field(
                            w_ref,
                            civ_idx,
                            enemy_city_tiles,
                            false,
                            240,
                            enemy_city_land_dist,
                            enemy_city_land_next,
                        )
                    },
                    || {
                        compute_reverse_flow_field(
                            w_ref,
                            civ_idx,
                            &naval_seeds,
                            true,
                            240,
                            enemy_city_water_dist,
                            enemy_city_water_next,
                        )
                    },
                )
            },
            || {
                rayon::join(
                    || {
                        compute_reverse_flow_field(
                            w_ref,
                            civ_idx,
                            own_city_tiles,
                            false,
                            180,
                            friendly_city_dist,
                            friendly_city_next,
                        )
                    },
                    || {
                        rayon::join(
                            || {
                                compute_reverse_flow_field(
                                    w_ref,
                                    civ_idx,
                                    sieged_city_tiles,
                                    false,
                                    120,
                                    sieged_city_dist,
                                    sieged_city_next,
                                )
                            },
                            || {
                                compute_reverse_flow_field(
                                    w_ref,
                                    civ_idx,
                                    &explore_seeds,
                                    false,
                                    80,
                                    explore_frontier_dist,
                                    explore_frontier_next,
                                )
                            },
                        )
                    },
                )
            },
        );
    });

    cache.civ = civ_idx as i32;
    cache.dirty = false;
    world.flow_fields = cache;
}

/// O(1) lookup of the best next-step tile in a precomputed theater flow-field (`-1` if unreachable).
#[inline]
pub fn query_flow_next_step(world: &NativeWorld, field_kind: i32, from_tile: usize) -> i32 {
    if from_tile >= world.num_tiles || world.flow_fields.dirty {
        return -1;
    }
    let next_slice = match field_kind {
        FLOW_ENEMY_CITY_LAND => &world.flow_fields.enemy_city_land_next,
        FLOW_ENEMY_CITY_WATER => &world.flow_fields.enemy_city_water_next,
        FLOW_FRIENDLY_CITY => &world.flow_fields.friendly_city_next,
        FLOW_SIEGED_CITY => &world.flow_fields.sieged_city_next,
        FLOW_EXPLORE_FRONTIER => &world.flow_fields.explore_frontier_next,
        _ => return -1,
    };
    if from_tile < next_slice.len() {
        next_slice[from_tile]
    } else {
        -1
    }
}

/// O(1) lookup of the weighted distance in a precomputed theater flow-field (`u16::MAX` if unreachable).
#[inline]
pub fn query_flow_dist(world: &NativeWorld, field_kind: i32, from_tile: usize) -> u16 {
    if from_tile >= world.num_tiles || world.flow_fields.dirty {
        return FLOW_UNREACHED;
    }
    let dist_slice = match field_kind {
        FLOW_ENEMY_CITY_LAND => &world.flow_fields.enemy_city_land_dist,
        FLOW_ENEMY_CITY_WATER => &world.flow_fields.enemy_city_water_dist,
        FLOW_FRIENDLY_CITY => &world.flow_fields.friendly_city_dist,
        FLOW_SIEGED_CITY => &world.flow_fields.sieged_city_dist,
        FLOW_EXPLORE_FRONTIER => &world.flow_fields.explore_frontier_dist,
        _ => return FLOW_UNREACHED,
    };
    if from_tile < dist_slice.len() {
        dist_slice[from_tile]
    } else {
        FLOW_UNREACHED
    }
}

/// Returns all explored tiles within `radius` of `center_tile` that are either inside
/// `world.threat_danger_bits` OR inflict `terrain_damage > 0`.
/// Replaces `UnitAutomation.getDangerousTiles`'s nested `flatMap` ring scans!
pub fn get_dangerous_tiles_in_radius(
    world: &NativeWorld,
    civ_idx: usize,
    center_tile: usize,
    radius: i32,
) -> Vec<i32> {
    if civ_idx >= world.num_civs || center_tile >= world.num_tiles || radius < 0 {
        return Vec::new();
    }
    let n = world.num_tiles;
    let mut out = Vec::new();

    with_scratch(|sc| {
        let gen = sc.next_gen(n);
        sc.seen_gen[center_tile] = gen;
        sc.parent_i32[center_tile] = 0; // depth
        sc.queue.push(center_tile as i32);

        let mut head = 0usize;
        while head < sc.queue.len() {
            let u = sc.queue[head] as usize;
            head += 1;
            if world.is_explored(civ_idx, u) {
                let in_danger = ((world.threat_danger_bits[u >> 6] >> (u & 63)) & 1) != 0;
                if in_danger || world.terrain_damage[u] > 0 {
                    out.push(u as i32);
                }
            }
            let depth = sc.parent_i32[u];
            if depth >= radius {
                continue;
            }
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb >= 0 {
                    let v = nb as usize;
                    if sc.seen_gen[v] != gen {
                        sc.seen_gen[v] = gen;
                        sc.parent_i32[v] = depth + 1;
                        sc.queue.push(nb);
                    }
                }
            }
        }
    });

    out
}

/// Fast native replacement for `NextTurnAutomation.trainSettler`'s late-game 331-tile-per-city scan.
/// Returns `true` iff at least `min_count` unowned tiles (with all neighbors unowned) exist
/// within distance 10 of any tile in `city_tiles`.
pub fn has_enough_settler_room(
    world: &NativeWorld,
    city_tiles: &[i32],
    min_count: i32,
) -> bool {
    if min_count <= 0 {
        return true;
    }
    let n = world.num_tiles;
    if city_tiles.is_empty() || n == 0 {
        return false;
    }

    with_scratch(|sc| {
        let gen = sc.next_gen(n);
        for &ct in city_tiles {
            if ct >= 0 && (ct as usize) < n {
                let u = ct as usize;
                if sc.seen_gen[u] != gen {
                    sc.seen_gen[u] = gen;
                    sc.parent_i32[u] = 0;
                    sc.queue.push(ct);
                }
            }
        }

        let mut found = 0i32;
        let mut head = 0usize;
        while head < sc.queue.len() {
            let u = sc.queue[head] as usize;
            head += 1;
            let depth = sc.parent_i32[u];

            if world.owner_civ[u] == NO_CIV {
                let base = u * 6;
                let mut all_unowned = true;
                for d in 0..6 {
                    let nb = world.neighbors[base + d];
                    if nb >= 0 && world.owner_civ[nb as usize] != NO_CIV {
                        all_unowned = false;
                        break;
                    }
                }
                if all_unowned {
                    found += 1;
                    if found >= min_count {
                        return true;
                    }
                }
            }

            if depth >= 10 {
                continue;
            }
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb >= 0 {
                    let v = nb as usize;
                    if sc.seen_gen[v] != gen {
                        sc.seen_gen[v] = gen;
                        sc.parent_i32[v] = depth + 1;
                        sc.queue.push(nb);
                    }
                }
            }
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathing::tests::make_hex_world;

    #[test]
    fn flow_fields_route_around_mountains_and_respect_ocean() {
        let (mut w, map) = make_hex_world(5, 2);
        for t in 0..w.num_tiles {
            w.set_explored_bit(0, t, true);
        }
        // Make a mountain barrier at x = 0 except y = 2
        for (&(x, y), &idx) in &map {
            if x == 0 && y != 2 {
                w.terrain_flags[idx] |= TF_IMPASSABLE;
            }
        }
        let enemy_city = map[&(3, 0)] as i32;
        let own_city = map[&(-3, 0)] as i32;
        w.tile_flags[enemy_city as usize] |= TILE_CITY_CENTER;
        w.owner_civ[enemy_city as usize] = 1;
        w.diplo_matrix[0 * 2 + 1] = DIPLO_AT_WAR | DIPLO_CAN_PASS;

        build_civ_flow_fields(&mut w, 0, &[enemy_city], &[own_city], &[own_city]);

        let start = map[&(-2, 0)];
        let d0 = query_flow_dist(&w, FLOW_ENEMY_CITY_LAND, start);
        assert!(d0 < FLOW_UNREACHED, "Start tile should reach enemy city via gap");

        // Follow flow steps — must never step onto an impassable mountain and must reach enemy_city
        let mut cur = start;
        let mut steps = 0;
        while cur != enemy_city as usize && steps < 40 {
            let nxt = query_flow_next_step(&w, FLOW_ENEMY_CITY_LAND, cur);
            assert!(nxt >= 0, "Flow step should exist at tile {cur}");
            assert_eq!(w.terrain_flags[nxt as usize] & TF_IMPASSABLE, 0);
            cur = nxt as usize;
            steps += 1;
        }
        assert_eq!(cur, enemy_city as usize);
    }

    #[test]
    fn settler_room_check_counts_unowned_clusters() {
        let (mut w, map) = make_hex_world(4, 2);
        let c0 = map[&(0, 0)] as i32;
        assert!(has_enough_settler_room(&w, &[c0], 6));
        // Claim every tile
        w.owner_civ.fill(0);
        assert!(!has_enough_settler_room(&w, &[c0], 1));
    }
}
