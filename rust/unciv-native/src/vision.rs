//! O(R^2) hex line-of-sight and O(ring) reference-counted incremental civilization vision.
//!
//! Replaces `TileMap.getViewableTiles`'s O(R^4) linear scan and `CivInfoTransientCache`'s
//! full-empire `HashSet<Tile>` rebuild on every unit move.

use crate::scratch::with_scratch;
use crate::world::*;

/// Exact O(R^2) port of `TileMap.getViewableTiles(position, sightDistance, forAttack)`.
///
/// Returns the tile indices visible (or attackable if `for_attack`) from `origin_idx`.
pub fn compute_viewable_tiles(
    world: &NativeWorld,
    origin_idx: usize,
    sight_distance: i32,
    for_attack: bool,
) -> Vec<i32> {
    if origin_idx >= world.num_tiles {
        return Vec::new();
    }
    if sight_distance < 0 {
        return if for_attack {
            Vec::new()
        } else {
            vec![origin_idx as i32]
        };
    }

    let max_ring = (sight_distance + 1) as u32;
    let a_unit_height = world.unit_height[origin_idx];

    with_scratch(|sc| {
        let base_tag = sc.next_los_base(world.num_tiles, max_ring + 1);
        sc.los_ring_tag[origin_idx] = base_tag;
        sc.los_max_height[origin_idx] = a_unit_height;
        sc.los_ring_curr.push(origin_idx as i32);

        if !for_attack {
            sc.los_out.push(origin_idx as i32);
        }

        for r in 1..=max_ring {
            let prev_tag = base_tag + (r - 1);
            let cur_tag = base_tag + r;
            let is_peek_ring = r == max_ring;
            if is_peek_ring && for_attack {
                break;
            }

            sc.los_ring_next.clear();
            let curr_len = sc.los_ring_curr.len();
            for ci in 0..curr_len {
                let u = sc.los_ring_curr[ci] as usize;
                let u_base = u * 6;
                for d in 0..6 {
                    let nb = world.neighbors[u_base + d];
                    if nb < 0 {
                        continue;
                    }
                    let c = nb as usize;
                    // Skip tiles already assigned to an earlier ring or already visited in this ring
                    if sc.los_ring_tag[c] >= base_tag {
                        continue;
                    }
                    sc.los_ring_tag[c] = cur_tag;
                    sc.los_ring_next.push(c as i32);
                }
            }

            // Now evaluate every tile `c` in ring `r` against its neighbors in ring `r - 1`.
            let next_len = sc.los_ring_next.len();
            for ni in 0..next_len {
                let c = sc.los_ring_next[ni] as usize;
                let c_tile_height = world.tile_height[c];

                if is_peek_ring && c_tile_height <= a_unit_height {
                    continue;
                }

                let mut min_prev_height = i8::MAX;
                let c_base = c * 6;
                for d in 0..6 {
                    let nb = world.neighbors[c_base + d];
                    if nb < 0 {
                        continue;
                    }
                    let nbu = nb as usize;
                    if sc.los_ring_tag[nbu] == prev_tag {
                        let h = sc.los_max_height[nbu];
                        if h < min_prev_height {
                            min_prev_height = h;
                        }
                    }
                }
                if min_prev_height == i8::MAX {
                    continue;
                }

                sc.los_max_height[c] = c_tile_height.max(min_prev_height);

                if for_attack {
                    let c_unit_height = world.unit_height[c];
                    if a_unit_height >= min_prev_height || c_unit_height > min_prev_height {
                        sc.los_out.push(c as i32);
                    }
                } else if a_unit_height >= min_prev_height || c_tile_height > min_prev_height {
                    sc.los_out.push(c as i32);
                }
            }

            std::mem::swap(&mut sc.los_ring_curr, &mut sc.los_ring_next);
            if sc.los_ring_curr.is_empty() {
                break;
            }
        }

        sc.los_out.clone()
    })
}

/// Initializes a civilization's reference-counted vision state from:
/// * `static_tiles`: deduplicated union of `ourTilesAndNeighboringTiles`, allied city tiles, and spy tiles
/// * `unit_tiles_flat`: concatenation of each unit's deduplicated `viewableTiles` (all units of this civ)
///
/// Updates `vision_refcount`, `visible_bits`, and `explored_bits`, and marks `vision_synced[civ] = true`.
/// Returns all newly explored tile indices (tiles that were not previously explored by `civ`).
pub fn rebuild_civ_vision(
    world: &mut NativeWorld,
    civ_idx: usize,
    static_tiles: &[i32],
    unit_tiles_flat: &[i32],
) -> Vec<i32> {
    if civ_idx >= world.num_civs {
        return Vec::new();
    }
    let n = world.num_tiles;
    let w = world.bitset_words;
    let rc_off = civ_idx * n;
    let bs_off = civ_idx * w;

    world.vision_refcount[rc_off..rc_off + n].fill(0);
    world.visible_bits[bs_off..bs_off + w].fill(0);

    let mut newly_explored = Vec::new();

    let mut add_tile = |t: i32| {
        if t < 0 || (t as usize) >= n {
            return;
        }
        let ti = t as usize;
        let rc = &mut world.vision_refcount[rc_off + ti];
        let was_zero = *rc == 0;
        *rc = rc.saturating_add(1);
        if was_zero {
            let word = bs_off + (ti >> 6);
            let mask = 1u64 << (ti & 63);
            world.visible_bits[word] |= mask;
            if (world.explored_bits[word] & mask) == 0 {
                world.explored_bits[word] |= mask;
                newly_explored.push(t);
            }
        }
    };

    for &t in static_tiles {
        add_tile(t);
    }
    for &t in unit_tiles_flat {
        add_tile(t);
    }

    world.vision_synced[civ_idx] = true;
    if world.threat_civ == civ_idx as i32 {
        world.threat_dirty = true;
    }
    newly_explored
}

/// Applies an O(|old| + |new|) reference-counted vision delta for a single unit moving / spawning / dying.
///
/// Both `old_tiles` and `new_tiles` must be deduplicated sets of tile indices for that single unit.
/// Returns packed `[removed_len, added_len, newly_explored_len, ...removed, ...added, ...newly_explored]`.
/// Returns empty vec if `civ_idx` is not currently `vision_synced`.
pub fn apply_unit_vision_delta(
    world: &mut NativeWorld,
    civ_idx: usize,
    old_tiles: &[i32],
    new_tiles: &[i32],
) -> Vec<i32> {
    if civ_idx >= world.num_civs || !world.vision_synced[civ_idx] {
        return Vec::new();
    }
    let n = world.num_tiles;
    let w = world.bitset_words;
    let rc_off = civ_idx * n;
    let bs_off = civ_idx * w;

    let mut added = Vec::with_capacity(new_tiles.len());
    let mut newly_explored = Vec::new();
    let mut removed = Vec::with_capacity(old_tiles.len());
    let mut threat_changed = false;

    // Pass 1: increment new_tiles first so tiles in both old and new never drop to 0
    for &t in new_tiles {
        if t < 0 || (t as usize) >= n {
            continue;
        }
        let ti = t as usize;
        let rc = &mut world.vision_refcount[rc_off + ti];
        let was_zero = *rc == 0;
        *rc = rc.saturating_add(1);
        if was_zero {
            let word = bs_off + (ti >> 6);
            let mask = 1u64 << (ti & 63);
            world.visible_bits[word] |= mask;
            added.push(t);
            if (world.explored_bits[word] & mask) == 0 {
                world.explored_bits[word] |= mask;
                newly_explored.push(t);
            }
            if world.mil_unit_civ[ti] >= 0
                && world.is_at_war(civ_idx, world.mil_unit_civ[ti] as usize)
            {
                threat_changed = true;
            }
        }
    }

    // Pass 2: decrement old_tiles
    for &t in old_tiles {
        if t < 0 || (t as usize) >= n {
            continue;
        }
        let ti = t as usize;
        let rc = &mut world.vision_refcount[rc_off + ti];
        if *rc > 0 {
            *rc -= 1;
            if *rc == 0 {
                let word = bs_off + (ti >> 6);
                let mask = 1u64 << (ti & 63);
                world.visible_bits[word] &= !mask;
                removed.push(t);
                if world.mil_unit_civ[ti] >= 0
                    && world.is_at_war(civ_idx, world.mil_unit_civ[ti] as usize)
                {
                    threat_changed = true;
                }
            }
        }
    }

    if threat_changed && world.threat_civ == civ_idx as i32 {
        world.threat_dirty = true;
    }

    let mut packed = Vec::with_capacity(3 + removed.len() + added.len() + newly_explored.len());
    packed.push(removed.len() as i32);
    packed.push(added.len() as i32);
    packed.push(newly_explored.len() as i32);
    packed.extend_from_slice(&removed);
    packed.extend_from_slice(&added);
    packed.extend_from_slice(&newly_explored);
    packed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pathing::tests::make_hex_world;

    #[test]
    fn los_matches_hill_forest_mountain_rules() {
        let (mut w, m) = make_hex_world(5, 2);
        // Set (1,0) to forest (tile_height = 1, unit_height = 0)
        w.tile_height[m[&(1, 0)]] = 1;
        // (2,0) is plains (tile_height = 0) -> hidden behind forest!
        let vis = compute_viewable_tiles(&w, m[&(0, 0)], 2, false);
        assert!(vis.contains(&(m[&(1, 0)] as i32)));
        assert!(!vis.contains(&(m[&(2, 0)] as i32)));

        // Set (2,0) to hill+forest (tile_height = 2, unit_height = 1) -> visible over forest,
        // and ALSO visible for attack? unit_height=1 > b=1 is false -> NOT attackable!
        w.tile_height[m[&(2, 0)]] = 2;
        w.unit_height[m[&(2, 0)]] = 1;
        let vis2 = compute_viewable_tiles(&w, m[&(0, 0)], 2, false);
        assert!(vis2.contains(&(m[&(2, 0)] as i32)));
        let atk2 = compute_viewable_tiles(&w, m[&(0, 0)], 2, true);
        assert!(!atk2.contains(&(m[&(2, 0)] as i32)));
    }

    #[test]
    fn los_elevation_peek_ring() {
        let (mut w, m) = make_hex_world(5, 2);
        // (3,0) is a hill (tile_height = 1 > origin unit_height = 0), sight = 2
        w.tile_height[m[&(3, 0)]] = 1;
        let vis = compute_viewable_tiles(&w, m[&(0, 0)], 2, false);
        assert!(vis.contains(&(m[&(3, 0)] as i32)));
        // Plains at (0,3) is NOT visible at sight=2
        assert!(!vis.contains(&(m[&(0, 3)] as i32)));
        // forAttack must NOT include peek ring
        let atk = compute_viewable_tiles(&w, m[&(0, 0)], 2, true);
        assert!(!atk.contains(&(m[&(3, 0)] as i32)));
    }

    #[test]
    fn incremental_vision_refcounts() {
        let (mut w, _m) = make_hex_world(3, 2);
        // Clear explored/visible for civ 0
        w.explored_bits.fill(0);
        w.visible_bits.fill(0);
        let static_t = [0i32, 1];
        let unit_t = [1i32, 2, 3];
        let newly = rebuild_civ_vision(&mut w, 0, &static_t, &unit_t);
        assert_eq!(newly.len(), 4);
        assert_eq!(w.vision_refcount[1], 2); // seen by both static and unit

        // Unit moves from [1,2,3] to [1,3,4]
        let delta = apply_unit_vision_delta(&mut w, 0, &[1, 2, 3], &[1, 3, 4]);
        let (rem_n, add_n, exp_n) = (delta[0] as usize, delta[1] as usize, delta[2] as usize);
        assert_eq!(rem_n, 1); // tile 2 removed
        assert_eq!(add_n, 1); // tile 4 added
        assert_eq!(exp_n, 1); // tile 4 newly explored
        assert_eq!(delta[3], 2);
        assert_eq!(delta[4], 4);
        assert_eq!(delta[5], 4);
        assert!(!w.is_visible(0, 2));
        assert!(w.is_visible(0, 4));
        assert!(w.is_visible(0, 1));
    }
}
