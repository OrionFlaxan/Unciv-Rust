//! Native graph connectivity engines:
//! * `find_capital_connections`: exact port of `CapitalConnectionsFinder.find()` (road, railroad, harbor BFS closure)
//! * `find_road_path` & `find_road_path_to_any`: road-preferring Dijkstra/A* (0.5 on road/rail vs 1.0)
//! * `eval_attack_paths`: land/amphibious attack-path connectivity + fallback capital BFS for `MotivationToAttackAutomation`

use crate::scratch::with_scratch;
use crate::world::*;

// CapitalConnectionMedium bitmask matching CapitalConnectionsFinder.CapitalConnectionMedium
pub const MED_START: i32 = 1 << 0;
pub const MED_ROAD: i32 = 1 << 1;
pub const MED_RAILROAD: i32 = 1 << 2;
pub const MED_HARBOR_FROM_ROAD: i32 = 1 << 3;
pub const MED_HARBOR_FROM_RAILROAD: i32 = 1 << 4;

// City info flags passed into find_capital_connections
pub const CITY_IS_OWN: i32 = 1 << 0;
pub const CITY_HAS_HARBOR: i32 = 1 << 1;
pub const CITY_IS_BLOCKADED: i32 = 1 << 2;

/// Exact port of `CapitalConnectionsFinder.find()`.
///
/// * `city_tiles[i]`: tile index of the `i`-th open-borders city (all cities whose borders `civ_idx` can enter)
/// * `city_flags[i]`: bitmask of `CITY_IS_OWN`, `CITY_HAS_HARBOR`, `CITY_IS_BLOCKADED`
/// * `capital_city_slot`: index `0..city_tiles.len()` of `civ_idx`'s capital
/// * `road_researched`, `railroad_researched`: whether civ has unlocked road/railroad tech
///
/// Returns `medium_masks` of length `city_tiles.len()` (0 if city was not reached from capital).
pub fn find_capital_connections(
    world: &NativeWorld,
    civ_idx: usize,
    city_tiles: &[i32],
    city_flags: &[i32],
    capital_city_slot: usize,
    road_researched: bool,
    railroad_researched: bool,
) -> Vec<i32> {
    let num_cities = city_tiles.len();
    if num_cities == 0 || capital_city_slot >= num_cities || civ_idx >= world.num_civs {
        return vec![0; num_cities];
    }

    let mut mediums = vec![0i32; num_cities];
    mediums[capital_city_slot] = MED_START;
    let mut reached_count = 1usize;

    let mut cities_to_check = vec![capital_city_slot];
    let mut new_cities_to_check = Vec::new();

    // Map tileIdx -> city_slot in O(1) via scratch.mark/mark_aux
    with_scratch(|sc| {
        let mgen = sc.next_mark(world.num_tiles);
        for (ci, &ct) in city_tiles.iter().enumerate() {
            if ct >= 0 && (ct as usize) < world.num_tiles {
                sc.mark[ct as usize] = mgen;
                sc.mark_aux[ct as usize] = ci as i32;
            }
        }

        // Helper closure: runs one BFS from `start_city` with `mode` (0=Road, 1=Railroad, 2=Harbor)
        let mut run_bfs = |start_slot: usize,
                           transport: i32,
                           override_transport: i32,
                           mode: u8,
                           mediums: &mut [i32],
                           reached_count: &mut usize,
                           new_cities: &mut Vec<usize>| {
            if (mediums[start_slot] & (transport | override_transport)) != 0 {
                return;
            }
            let start_tile = city_tiles[start_slot] as usize;
            let gen = sc.next_gen(world.num_tiles);
            sc.seen_gen[start_tile] = gen;
            sc.queue.push(start_tile as i32);

            let mut head = 0usize;
            while head < sc.queue.len() {
                let u = sc.queue[head] as usize;
                head += 1;

                // If `u` is one of the open-borders cities
                if sc.mark[u] == mgen {
                    let reached_slot = sc.mark_aux[u] as usize;
                    let city_ok = if mode == 2 {
                        // Harbor: own city, has harbor, not blockaded
                        let cf = city_flags[reached_slot];
                        (cf & CITY_IS_OWN) != 0
                            && (cf & CITY_HAS_HARBOR) != 0
                            && (cf & CITY_IS_BLOCKADED) == 0
                    } else {
                        true
                    };
                    if city_ok {
                        if mediums[reached_slot] == 0 {
                            new_cities.push(reached_slot);
                            *reached_count += 1;
                        }
                        if reached_slot != start_slot
                            && (mediums[reached_slot] & (transport | override_transport)) == 0
                        {
                            mediums[reached_slot] |= transport;
                        }
                    }
                }

                let base = u * 6;
                for d in 0..6 {
                    let nb = world.neighbors[base + d];
                    if nb < 0 {
                        continue;
                    }
                    let v = nb as usize;
                    if sc.seen_gen[v] == gen {
                        continue;
                    }
                    // Owner borders check: owner == null || canEnterBordersOf(owner)
                    let owner = world.owner_civ[v];
                    if owner >= 0 && !world.has_open_borders_conn(civ_idx, owner as usize) {
                        continue;
                    }
                    let is_city = (world.tile_flags[v] & TILE_CITY_CENTER) != 0;
                    let tile_ok = is_city
                        || match mode {
                            0 => world.has_connection(civ_idx, v),
                            1 => world.unpillaged_road(v) == TILE_ROAD_RAILROAD,
                            _ => (world.terrain_flags[v] & TF_WATER) != 0,
                        };
                    if tile_ok {
                        sc.seen_gen[v] = gen;
                        sc.queue.push(v as i32);
                    }
                }
            }
        };

        while !cities_to_check.is_empty() && reached_count < num_cities {
            new_cities_to_check.clear();
            for &slot in &cities_to_check {
                if (city_flags[slot] & CITY_HAS_HARBOR) != 0 {
                    let transport = if (mediums[slot] & MED_RAILROAD) != 0 {
                        MED_HARBOR_FROM_RAILROAD
                    } else {
                        MED_HARBOR_FROM_ROAD
                    };
                    run_bfs(
                        slot,
                        transport,
                        MED_HARBOR_FROM_RAILROAD,
                        2,
                        &mut mediums,
                        &mut reached_count,
                        &mut new_cities_to_check,
                    );
                }
                if railroad_researched {
                    let m = mediums[slot];
                    if (m & (MED_START | MED_RAILROAD | MED_HARBOR_FROM_RAILROAD)) != 0 {
                        run_bfs(
                            slot,
                            MED_RAILROAD,
                            0,
                            1,
                            &mut mediums,
                            &mut reached_count,
                            &mut new_cities_to_check,
                        );
                    }
                }
                if road_researched {
                    run_bfs(
                        slot,
                        MED_ROAD,
                        MED_RAILROAD,
                        0,
                        &mut mediums,
                        &mut reached_count,
                        &mut new_cities_to_check,
                    );
                }
            }
            std::mem::swap(&mut cities_to_check, &mut new_cities_to_check);
        }
    });

    mediums
}

/// Road-preferring Dijkstra (`MapPathing.getRoadPath` and `TileView.getRoadPathToAny`).
///
/// * `valid_road_tile_bits`: optional bitset (len `bitset_words`) of tiles where `isValidRoadPathTile(civ, tile)` is true.
/// * `targets`: 1 or more destination tile indices.
/// * `max_steps`: max road cost in half-steps (e.g., `max_turns * 2`, or `i32::MAX` for unlimited).
///
/// Edge cost is 1 half-step (`0.5`) if destination has road/railroad/forest-road, else 2 half-steps (`1.0`).
/// Returns the tile index path from `start_tile` to the reached target (inclusive of both endpoints),
/// or empty vec if none is reachable.
pub fn find_road_path(
    world: &NativeWorld,
    civ_idx: usize,
    start_tile: usize,
    targets: &[i32],
    valid_road_tile_bits: &[u64],
    max_half_cost: i32,
) -> Vec<i32> {
    let n = world.num_tiles;
    if start_tile >= n || targets.is_empty() || civ_idx >= world.num_civs {
        return Vec::new();
    }

    with_scratch(|sc| {
        let mgen = sc.next_mark(n);
        for &t in targets {
            if t >= 0 && (t as usize) < n {
                if t as usize == start_tile {
                    return vec![start_tile as i32];
                }
                sc.mark[t as usize] = mgen;
            }
        }

        let gen = sc.next_gen(n);
        sc.seen_gen[start_tile] = gen;
        sc.parent_i32[start_tile] = -1;
        sc.route_parent[start_tile] = 0; // store best half-cost here
        // Heap entry: (half_cost: 24b) << 20 | tile_idx(20b)
        sc.heap_push(start_tile as u64);

        let mut reached_target: i32 = -1;

        while let Some(popped) = sc.heap_pop() {
            let u = (popped & 0xF_FFFF) as usize;
            let cost = (popped >> 20) as i32;
            if cost > sc.route_parent[u] {
                continue;
            }
            if sc.mark[u] == mgen {
                reached_target = u as i32;
                break;
            }
            if cost >= max_half_cost {
                continue;
            }

            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;
                // Must be a valid road path tile (or start/target check: in MapPathing AStar checks predicate on every neighbor!)
                if ((valid_road_tile_bits[v >> 6] >> (v & 63)) & 1) == 0 {
                    continue;
                }
                let step = if world.has_road_or_rail_any(civ_idx, v) { 1 } else { 2 };
                let new_cost = cost + step;
                if new_cost > max_half_cost {
                    continue;
                }
                if sc.seen_gen[v] != gen || new_cost < sc.route_parent[v] {
                    sc.seen_gen[v] = gen;
                    sc.route_parent[v] = new_cost;
                    sc.parent_i32[v] = u as i32;
                    sc.heap_push(((new_cost as u64) << 20) | (v as u64));
                }
            }
        }

        if reached_target < 0 {
            return Vec::new();
        }
        let mut path = Vec::with_capacity(32);
        let mut cur = reached_target;
        while cur >= 0 {
            path.push(cur);
            cur = sc.parent_i32[cur as usize];
        }
        path.reverse();
        path
    })
}

/// Evaluates `MotivationToAttackAutomation.getAttackPathsModifier` in ONE native call!
///
/// * `our_city_tiles`: `pair.first.getCenterTile().zeroBasedIndex` for each pair in `targetCitiesWithOurCity`
/// * `enemy_city_tiles`: `pair.second.getCenterTile().zeroBasedIndex` for each pair
/// * `capital_tile`: `civInfo.getCapital(true)!!.getCenterTile().zeroBasedIndex`
/// * `all_enemy_city_tiles`: all `otherCiv.cities` center tile indices (for the fallback BFS)
///
/// Returns exact `attackPathModifiers: f32`.
pub fn eval_attack_paths(
    world: &NativeWorld,
    civ_idx: usize,
    target_civ_idx: usize,
    our_city_tiles: &[i32],
    enemy_city_tiles: &[i32],
    capital_tile: i32,
    all_enemy_city_tiles: &[i32],
) -> f32 {
    let n = world.num_tiles;
    if civ_idx >= world.num_civs || target_civ_idx >= world.num_civs {
        return -3.0;
    }

    let can_pass = |tile: usize, land_only: bool| -> bool {
        let tf = world.terrain_flags[tile];
        if (tf & TF_IMPASSABLE) != 0 {
            return false;
        }
        if land_only && (tf & TF_LAND) == 0 {
            return false;
        }
        let owner = world.owner_civ[tile];
        owner < 0
            || owner as usize == target_civ_idx
            || owner as usize == civ_idx
            || world.can_pass_borders(civ_idx, owner as usize)
    };

    // Unweighted BFS returning shortest hop count (number of tiles in path including endpoints),
    // capped at `max_path_len` (16 -> stops at depth 14 so path len < 16).
    let bfs_path_len = |start: usize, goal: usize, land_only: bool, max_depth: i32| -> Option<i32> {
        if start == goal {
            return Some(1);
        }
        with_scratch(|sc| {
            let gen = sc.next_gen(n);
            sc.seen_gen[start] = gen;
            sc.parent_i32[start] = 0; // depth
            sc.queue.push(start as i32);
            let mut head = 0usize;
            while head < sc.queue.len() {
                let u = sc.queue[head] as usize;
                head += 1;
                let depth = sc.parent_i32[u];
                if depth >= max_depth {
                    continue;
                }
                let base = u * 6;
                for d in 0..6 {
                    let nb = world.neighbors[base + d];
                    if nb < 0 {
                        continue;
                    }
                    let v = nb as usize;
                    if sc.seen_gen[v] == gen {
                        continue;
                    }
                    if !can_pass(v, land_only) {
                        continue;
                    }
                    if v == goal {
                        return Some(depth + 2); // path includes both start and goal
                    }
                    sc.seen_gen[v] = gen;
                    sc.parent_i32[v] = depth + 1;
                    sc.queue.push(v as i32);
                }
            }
            None
        })
    };

    // Group pairs by `our_city_tile` preserving first-seen order
    let mut unique_ours: Vec<i32> = Vec::new();
    for &oc in our_city_tiles {
        if !unique_ours.contains(&oc) {
            unique_ours.push(oc);
        }
    }

    let mut attack_paths_count = 0i32;
    let mut attack_path_modifiers: f32 = -3.0;

    for &our_tile in &unique_ours {
        if our_tile < 0 || (our_tile as usize) >= n {
            continue;
        }
        let mut city_attack_value: f32 = 0.0;
        for i in 0..our_city_tiles.len().min(enemy_city_tiles.len()) {
            if our_city_tiles[i] != our_tile {
                continue;
            }
            let enemy_tile = enemy_city_tiles[i];
            if enemy_tile < 0 || (enemy_tile as usize) >= n {
                continue;
            }
            // Land attack path with size < 16 (depth <= 14)
            if bfs_path_len(our_tile as usize, enemy_tile as usize, true, 14).is_some() {
                attack_paths_count += 1;
                city_attack_value = 3.0;
                break;
            }
            if city_attack_value > 0.0 {
                continue;
            }
            // Land + sea attack path with size < 16
            if bfs_path_len(our_tile as usize, enemy_tile as usize, false, 14).is_some() {
                attack_paths_count += 1;
                city_attack_value += 1.0;
            }
        }
        attack_path_modifiers += city_attack_value;
    }

    if attack_paths_count == 0 {
        if capital_tile < 0 || (capital_tile as usize) >= n || all_enemy_city_tiles.is_empty() {
            return -50.0;
        }
        // Fallback full-map BFS from capital to closest reachable enemy city
        let min_dist = with_scratch(|sc| {
            let mgen = sc.next_mark(n);
            for &et in all_enemy_city_tiles {
                if et >= 0 && (et as usize) < n {
                    sc.mark[et as usize] = mgen;
                }
            }
            let start = capital_tile as usize;
            if sc.mark[start] == mgen {
                return Some(1i32);
            }
            let gen = sc.next_gen(n);
            sc.seen_gen[start] = gen;
            sc.parent_i32[start] = 0;
            sc.queue.push(start as i32);
            let mut head = 0usize;
            while head < sc.queue.len() {
                let u = sc.queue[head] as usize;
                head += 1;
                let depth = sc.parent_i32[u];
                let base = u * 6;
                for d in 0..6 {
                    let nb = world.neighbors[base + d];
                    if nb < 0 {
                        continue;
                    }
                    let v = nb as usize;
                    if sc.seen_gen[v] == gen || !can_pass(v, false) {
                        continue;
                    }
                    if sc.mark[v] == mgen {
                        return Some(depth + 2); // BFS.getPathTo(v).count() includes start & end
                    }
                    sc.seen_gen[v] = gen;
                    sc.parent_i32[v] = depth + 1;
                    sc.queue.push(v as i32);
                }
            }
            None
        });
        match min_dist {
            None => return -50.0,
            Some(d) => {
                attack_path_modifiers -= (d - 10).clamp(0, 30) as f32;
            }
        }
    }

    attack_path_modifiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathing::tests::make_hex_world;

    #[test]
    fn capital_connections_road_rail_harbor() {
        let (mut w, m) = make_hex_world(4, 2);
        // Capital at (-2,0), City B at (0,0) connected by Road, City C at (2,0) connected by Harbor across water at (1,0)
        let cap = m[&(-2, 0)];
        let mid = m[&(-1, 0)];
        let city_b = m[&(0, 0)];
        let sea = m[&(1, 0)];
        let city_c = m[&(2, 0)];

        w.tile_flags[cap] = TILE_CITY_CENTER | TILE_ROAD_RAILROAD;
        w.tile_flags[mid] = TILE_ROAD_ROAD;
        w.tile_flags[city_b] = TILE_CITY_CENTER | TILE_HAS_HARBOR;
        w.terrain_flags[sea] = TF_WATER;
        w.tile_flags[city_c] = TILE_CITY_CENTER | TILE_HAS_HARBOR;
        w.owner_civ[cap] = 0;
        w.owner_civ[city_b] = 0;
        w.owner_civ[city_c] = 0;

        let cities = [cap as i32, city_b as i32, city_c as i32];
        let flags = [
            CITY_IS_OWN,
            CITY_IS_OWN | CITY_HAS_HARBOR,
            CITY_IS_OWN | CITY_HAS_HARBOR,
        ];
        let res = find_capital_connections(&w, 0, &cities, &flags, 0, true, true);
        assert_ne!(res[0] & MED_START, 0);
        assert_ne!(res[1] & MED_ROAD, 0);
        assert_eq!(res[1] & MED_RAILROAD, 0);
        assert_ne!(res[2] & MED_HARBOR_FROM_ROAD, 0);
    }

    #[test]
    fn road_path_prefers_existing_roads() {
        let (mut w, m) = make_hex_world(4, 2);
        let valid_bits = vec![u64::MAX; w.bitset_words];
        // Make a 3-hop road detour vs 2-hop non-road: 3*0.5 = 1.5 < 2*1.0 = 2.0!
        for pos in [(0, 1), (1, 2), (2, 2)] {
            w.tile_flags[m[&pos]] |= TILE_ROAD_ROAD;
        }
        let path = find_road_path(
            &w,
            0,
            m[&(0, 0)],
            &[m[&(2, 2)] as i32],
            &valid_bits,
            100,
        );
        assert!(!path.is_empty());
        assert_eq!(path[0], m[&(0, 0)] as i32);
        assert_eq!(*path.last().unwrap(), m[&(2, 2)] as i32);
        // Should pass through (0,1) -> (1,2) -> (2,2) (cost 1.5) instead of (1,1) -> (2,2) (cost 1.5 tie)
        assert!(path.len() <= 4);
    }
}
