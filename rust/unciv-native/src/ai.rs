//! Native AI spatial acceleration:
//! * Multi-source enemy threat field (`build_threat_field`, `get_distance_to_closest_enemy`,
//!   `get_enemy_tiles_in_distance`)
//! * Single-pass attackable target pair discovery (`find_attackable_pairs`) with O(1) threat early-exit
//! * Fast explored-ring counting & exploration scoring (`count_explored_in_distance`, `count_unexplored_at_distance`)
//! * Native 2-ring city location spatial scoring (`score_city_locations`)

use crate::pathing::UnitProfile;
use crate::scratch::with_scratch;
use crate::vision::compute_viewable_tiles;
use crate::world::*;

/// Builds the multi-source enemy threat field for `civ_idx` in a single O(V) BFS.
///
/// Sources are every tile matching `ThreatManager.doesTileHaveMilitaryEnemy`:
/// * Explored enemy city center (at war with `civ_idx`), OR
/// * Visible tile containing a non-invisible enemy military unit, OR
/// * Any tile index listed in `extra_detected_enemy_tiles` (e.g., detected enemy submarines).
pub fn build_threat_field(
    world: &mut NativeWorld,
    civ_idx: usize,
    extra_detected_enemy_tiles: &[i32],
) {
    if civ_idx >= world.num_civs {
        return;
    }
    let n = world.num_tiles;
    world.threat_dist.fill(THREAT_UNREACHED);
    world.threat_enemy_tiles.clear();
    world.threat_danger_bits.fill(0);

    with_scratch(|sc| {
        let gen = sc.next_gen(n);
        for t in 0..n {
            if !world.is_explored(civ_idx, t) {
                continue;
            }
            let mut is_enemy = false;
            let tf = world.tile_flags[t];
            if (tf & TILE_CITY_CENTER) != 0 {
                let owner = world.owner_civ[t];
                if owner >= 0 && world.is_at_war(civ_idx, owner as usize) {
                    is_enemy = true;
                }
            }
            if !is_enemy && world.is_visible(civ_idx, t) {
                let mc = world.mil_unit_civ[t];
                if mc >= 0
                    && world.is_at_war(civ_idx, mc as usize)
                    && (world.mil_unit_flags[t] & UF_INVISIBLE) == 0
                {
                    is_enemy = true;
                }
            }
            if is_enemy {
                world.threat_dist[t] = 0;
                world.threat_enemy_tiles.push(t as i32);
                sc.seen_gen[t] = gen;
                sc.queue.push(t as i32);
            }
        }

        // Add any detected invisible enemy military units
        for &et in extra_detected_enemy_tiles {
            if et < 0 || (et as usize) >= n {
                continue;
            }
            let t = et as usize;
            if sc.seen_gen[t] != gen {
                world.threat_dist[t] = 0;
                world.threat_enemy_tiles.push(et);
                sc.seen_gen[t] = gen;
                sc.queue.push(et);
            }
        }

        // Multi-source BFS up to distance 250 across all tiles
        let mut head = 0usize;
        while head < sc.queue.len() {
            let u = sc.queue[head] as usize;
            head += 1;
            let next_d = world.threat_dist[u].saturating_add(1);
            if next_d >= 250 {
                continue;
            }
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;
                if sc.seen_gen[v] != gen {
                    sc.seen_gen[v] = gen;
                    world.threat_dist[v] = next_d;
                    sc.queue.push(v as i32);
                }
            }
        }
    });

    // Populate danger bits (tiles within enemy attack range or enemy city bombard range)
    // Only iterate over the small list of `threat_enemy_tiles`!
    let enemies_count = world.threat_enemy_tiles.len();
    if enemies_count > 0 {
        with_scratch(|sc| {
            for idx in 0..enemies_count {
                let et = world.threat_enemy_tiles[idx] as usize;
                let mut max_r = 0u8;
                if (world.tile_flags[et] & TILE_CITY_CENTER) != 0 {
                    max_r = max_r.max(world.city_bombard_range[et]);
                }
                if world.mil_unit_civ[et] >= 0
                    && world.is_at_war(civ_idx, world.mil_unit_civ[et] as usize)
                {
                    let r = world.mil_unit_flags[et] >> UF_RANGE_SHIFT;
                    max_r = max_r.max(r.max(1));
                }
                if max_r == 0 {
                    continue;
                }
                let gen = sc.next_gen(n);
                sc.seen_gen[et] = gen;
                sc.queue.push(et as i32);
                sc.parent_i32[et] = 0; // reuse parent_i32 as BFS depth
                let mut head = 0usize;
                while head < sc.queue.len() {
                    let u = sc.queue[head] as usize;
                    head += 1;
                    if world.is_explored(civ_idx, u) {
                        world.threat_danger_bits[u >> 6] |= 1u64 << (u & 63);
                    }
                    let depth = sc.parent_i32[u];
                    if depth >= max_r as i32 {
                        continue;
                    }
                    let base = u * 6;
                    for d in 0..6 {
                        let nb = world.neighbors[base + d];
                        if nb < 0 {
                            continue;
                        }
                        let v = nb as usize;
                        if sc.seen_gen[v] != gen {
                            sc.seen_gen[v] = gen;
                            sc.parent_i32[v] = depth + 1;
                            sc.queue.push(v as i32);
                        }
                    }
                }
            }
        });
    }

    world.threat_civ = civ_idx as i32;
    world.threat_dirty = false;
    world.threat_max_dist = 250;
}

/// O(1) query matching `ThreatManager.getDistanceToClosestEnemyUnit(civ, cache, tile, maxDist, takeLargerValues)`.
///
/// Note: `ThreatManager` searches rings `1..=maxDist` (strictly excluding distance 0).
pub fn get_distance_to_closest_enemy(
    world: &NativeWorld,
    tile_idx: usize,
    max_dist: i32,
    take_larger_values: bool,
) -> i32 {
    let not_found = if take_larger_values { 500_000 } else { max_dist };
    if tile_idx >= world.num_tiles || world.threat_enemy_tiles.is_empty() {
        return not_found;
    }
    let d = world.threat_dist[tile_idx];
    if d >= 1 {
        if (d as i32) <= max_dist {
            return d as i32;
        }
        return not_found;
    }
    // Rare case: tile_idx itself has an enemy (d == 0), find closest OTHER enemy tile (dist >= 1).
    let mut best = i32::MAX;
    for &et in &world.threat_enemy_tiles {
        let eu = et as usize;
        if eu == tile_idx {
            continue;
        }
        let dist = world.aerial_distance(tile_idx, eu);
        if dist < best {
            best = dist;
            if best == 1 {
                break;
            }
        }
    }
    if best <= max_dist {
        best
    } else {
        not_found
    }
}

/// Fast query matching `ThreatManager.getTilesWithEnemyUnitsInDistance(civ, cache, tile, maxDist)`.
///
/// Uses `threat_dist[tile]` for O(1) empty rejection when no enemy is within `max_dist`,
/// then filters the compact `threat_enemy_tiles` list (typically 5–50 elements) by aerial distance!
pub fn get_enemy_tiles_in_distance(
    world: &NativeWorld,
    tile_idx: usize,
    max_dist: i32,
) -> Vec<i32> {
    if tile_idx >= world.num_tiles || world.threat_enemy_tiles.is_empty() {
        return Vec::new();
    }
    let d0 = world.threat_dist[tile_idx];
    if d0 != 0 && (d0 as i32) > max_dist {
        return Vec::new();
    }
    let mut out = Vec::new();
    for &et in &world.threat_enemy_tiles {
        let eu = et as usize;
        if eu == tile_idx {
            continue;
        }
        let dist = world.aerial_distance(tile_idx, eu);
        if dist >= 1 && dist <= max_dist {
            out.push(et);
        }
    }
    out
}

/// Fast O(1)-early-exit + single-pass attackable target pair discovery for `TargetHelper.getAttackableEnemies`.
///
/// Given:
/// * `civ_idx`
/// * `from_tiles`: reachable tiles the unit can attack from (already sorted by movement spent and
///   filtered by `movementLeft > minimumMovementEpsilon` and `canMoveTo`)
/// * `from_move_left_bits`: corresponding float bits of remaining movement after moving + setup
/// * `attack_range`: unit attack range (1 for melee, >=1 for ranged)
/// * `is_melee`: true if melee
/// * `indirect_fire`: true if IndirectFire or AirUnit (no LOS obstruction check needed)
/// * `candidate_enemy_tiles`: tiles with potential attackable enemies visible to `civ_idx` (or empty to use `world` scan)
///
/// Returns packed triples `[fromTileIdx, targetTileIdx, movementLeftFloatBits]`.
pub fn find_attackable_pairs(
    world: &NativeWorld,
    civ_idx: usize,
    from_tiles: &[i32],
    from_move_left_bits: &[i32],
    attack_range: i32,
    is_melee: bool,
    indirect_fire: bool,
    candidate_enemy_tiles: &[i32],
) -> Vec<i32> {
    if from_tiles.is_empty() || candidate_enemy_tiles.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let n = world.num_tiles;

    if is_melee {
        // Mark all candidate enemies in scratch.mark for O(1) neighbor lookup
        with_scratch(|sc| {
            let mgen = sc.next_mark(n);
            for &et in candidate_enemy_tiles {
                if et >= 0 && (et as usize) < n {
                    sc.mark[et as usize] = mgen;
                }
            }
            for (fi, &ft) in from_tiles.iter().enumerate() {
                if ft < 0 || (ft as usize) >= n {
                    continue;
                }
                let f = ft as usize;
                let base = f * 6;
                for d in 0..6 {
                    let nb = world.neighbors[base + d];
                    if nb >= 0 && sc.mark[nb as usize] == mgen {
                        out.push(ft);
                        out.push(nb);
                        out.push(from_move_left_bits[fi]);
                    }
                }
            }
        });
        return out;
    }

    if indirect_fire {
        // No LOS obstruction check: pure aerial distance check against the compact enemy list!
        for (fi, &ft) in from_tiles.iter().enumerate() {
            if ft < 0 || (ft as usize) >= n {
                continue;
            }
            let f = ft as usize;
            for &et in candidate_enemy_tiles {
                if et == ft || et < 0 || (et as usize) >= n {
                    continue;
                }
                if world.aerial_distance(f, et as usize) <= attack_range {
                    out.push(ft);
                    out.push(et);
                    out.push(from_move_left_bits[fi]);
                }
            }
        }
        return out;
    }

    // Direct-fire ranged unit: ONLY compute O(R^2) LOS from `from_tile` if at least one candidate enemy
    // is within aerial distance `attack_range` of `from_tile`!
    // This skips LOS completely for tiles out of range of any enemy, and checks every enemy in O(1) via mark!
    let mut nearby_enemies = Vec::new();
    for (fi, &ft) in from_tiles.iter().enumerate() {
        if ft < 0 || (ft as usize) >= n {
            continue;
        }
        let f = ft as usize;
        nearby_enemies.clear();
        for &et in candidate_enemy_tiles {
            if et == ft || et < 0 || (et as usize) >= n {
                continue;
            }
            if world.aerial_distance(f, et as usize) <= attack_range {
                nearby_enemies.push(et);
            }
        }
        if nearby_enemies.is_empty() {
            continue;
        }

        // Compute attack LOS from `f` once
        let attackable_vis = compute_viewable_tiles(world, f, attack_range, true);
        if attackable_vis.is_empty() {
            continue;
        }
        with_scratch(|sc| {
            let mgen = sc.next_mark(n);
            for &vt in &attackable_vis {
                if vt >= 0 && (vt as usize) < n {
                    sc.mark[vt as usize] = mgen;
                }
            }
            for &et in &nearby_enemies {
                if sc.mark[et as usize] == mgen {
                    out.push(ft);
                    out.push(et);
                    out.push(from_move_left_bits[fi]);
                }
            }
        });
    }
    let _ = civ_idx;
    out
}

/// Counts explored tiles within `radius` (inclusive of center) and explored tiles strictly AT `radius`
/// using O(R^2) BFS on `world.neighbors` and O(1) `explored_bits` lookups.
/// Returns `(explored_in_dist, explored_at_dist)`.
pub fn count_explored_rings(world: &NativeWorld, civ_idx: usize, center: usize, radius: i32) -> (i32, i32) {
    if center >= world.num_tiles || civ_idx >= world.num_civs || radius < 0 {
        return (0, 0);
    }
    let mut in_dist = 0i32;
    let mut at_dist = 0i32;
    if world.is_explored(civ_idx, center) {
        in_dist += 1;
        if radius == 0 {
            at_dist += 1;
            return (in_dist, at_dist);
        }
    }
    if radius == 0 {
        return (in_dist, at_dist);
    }

    with_scratch(|sc| {
        let gen = sc.next_gen(world.num_tiles);
        sc.seen_gen[center] = gen;
        sc.parent_i32[center] = 0;
        sc.queue.push(center as i32);
        let mut head = 0usize;
        while head < sc.queue.len() {
            let u = sc.queue[head] as usize;
            head += 1;
            let depth = sc.parent_i32[u];
            if depth >= radius {
                continue;
            }
            let next_d = depth + 1;
            let base = u * 6;
            for d in 0..6 {
                let nb = world.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let v = nb as usize;
                if sc.seen_gen[v] != gen {
                    sc.seen_gen[v] = gen;
                    sc.parent_i32[v] = next_d;
                    if world.is_explored(civ_idx, v) {
                        in_dist += 1;
                        if next_d == radius {
                            at_dist += 1;
                        }
                    }
                    if next_d < radius {
                        sc.queue.push(v as i32);
                    }
                }
            }
        }
    });
    (in_dist, at_dist)
}

/// Evaluates `UnitAutomation.tryExplore` candidate tiles in ONE JNI call!
///
/// Given `candidates` (reachable this turn, or explored tiles within radius 5), checks:
/// * `terrain_damage[t] <= 0`
/// * `count_explored_in_distance(t, vis_range) < 1 + 3 * vis_range * (vis_range + 1)`
/// * `get_distance_to_closest_enemy(t, 3) > 3`
/// * `can_move_to(t)`
///
/// Returns packed `[tileIdx, score]` pairs (where `score = tile_height + estimated_unexplored_at_vis_range`)
/// for all passing tiles, so Kotlin only needs to pick the max (or check `canReach` on the filtered subset)!
pub fn filter_and_score_explore_tiles(
    world: &NativeWorld,
    p: &UnitProfile,
    candidates: &[i32],
    vis_range: i32,
) -> Vec<i32> {
    let civ_idx = p.civ_idx;
    let hex_area = 1 + 3 * vis_range * (vis_range + 1);
    let ring_size = 6 * vis_range;
    let mut out = Vec::new();
    for &ct in candidates {
        if ct < 0 || (ct as usize) >= world.num_tiles {
            continue;
        }
        let t = ct as usize;
        if world.terrain_damage[t] > 0 {
            continue;
        }
        if !crate::pathing::can_pass_through(world, p, t)
            || !crate::pathing::can_move_to_assuming_pass_through(world, p, t)
        {
            continue;
        }
        if get_distance_to_closest_enemy(world, t, 3, true) <= 3 {
            continue;
        }
        let (in_dist, at_dist) = count_explored_rings(world, civ_idx, t, vis_range);
        if in_dist >= hex_area {
            continue;
        }
        let est_unexplored = (ring_size - at_dist).max(0);
        let score = (world.tile_height[t] as i32) + est_unexplored;
        out.push(ct);
        out.push(score);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathing::tests::make_hex_world;

    #[test]
    fn threat_field_multi_source_distances() {
        let (mut w, m) = make_hex_world(5, 2);
        w.diplo_matrix[0 * 2 + 1] |= DIPLO_AT_WAR;
        w.diplo_matrix[1 * 2 + 0] |= DIPLO_AT_WAR;
        // Enemy warrior at (2,0)
        w.mil_unit_civ[m[&(2, 0)]] = 1;
        w.mil_unit_flags[m[&(2, 0)]] = 1 << UF_RANGE_SHIFT;
        build_threat_field(&mut w, 0, &[]);

        assert_eq!(get_distance_to_closest_enemy(&w, m[&(0, 0)], 5, true), 2);
        assert_eq!(get_distance_to_closest_enemy(&w, m[&(1, 0)], 5, true), 1);
        assert_eq!(get_distance_to_closest_enemy(&w, m[&(-3, 0)], 4, true), 500_000);
        assert_eq!(get_distance_to_closest_enemy(&w, m[&(-3, 0)], 4, false), 4);
        assert_eq!(get_enemy_tiles_in_distance(&w, m[&(0, 0)], 2), vec![m[&(2, 0)] as i32]);
        assert!(get_enemy_tiles_in_distance(&w, m[&(0, 0)], 1).is_empty());
    }

    #[test]
    fn attackable_pairs_respects_los() {
        let (mut w, m) = make_hex_world(4, 2);
        // Forest at (1,0) blocks direct fire from (0,0) to (2,0), but NOT from (1,1) to (2,0)
        w.tile_height[m[&(1, 0)]] = 1;
        let from_tiles = [m[&(0, 0)] as i32, m[&(1, 1)] as i32];
        let move_bits = [1.0f32.to_bits() as i32, 0.5f32.to_bits() as i32];
        let enemies = [m[&(2, 0)] as i32];

        let pairs = find_attackable_pairs(&w, 0, &from_tiles, &move_bits, 2, false, false, &enemies);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], m[&(1, 1)] as i32);
        assert_eq!(pairs[1], m[&(2, 0)] as i32);

        // With indirect fire, both (0,0) and (1,1) can shoot (2,0)
        let pairs_ind = find_attackable_pairs(&w, 0, &from_tiles, &move_bits, 2, false, true, &enemies);
        assert_eq!(pairs_ind.len(), 6);
    }
}
