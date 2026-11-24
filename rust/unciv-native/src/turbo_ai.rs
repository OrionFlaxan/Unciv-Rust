//! Multi-threaded Rayon Parallel Combat & Movement Planner ("Turbo Rust Native AI").
//!
//! Ported from HexForge-Rust's deterministic 2-phase parallel AI (`crates/sim/src/ai/mod.rs`)
//! and calibrated to Unciv's exact Civ V exponential combat formula, ZoC, terrain defense,
//! river crossing penalties, flanking bonuses, and O(V) Theater Flow-Fields!
//!
//! Phase 1 (Parallel `rayon::par_iter`): Every eligible combat unit evaluates its single-turn
//! reachable tiles, exact attack pairs (via LOS), Civ V combat value scores, and O(1) theater
//! flow-field destinations across CPU cores with zero JVM callbacks and zero lock contention.
//!
//! Phase 2 (Deterministic Sequential Reservation): Resolves target hex collisions & overkill
//! in exact unit priority order, outputting flat packed records `[action, moveTile, attackTile]`
//! for Kotlin to execute through Unciv's real `Battle.moveAndAttack` / `moveToTile` / `fortify`.

use crate::ai::find_attackable_pairs;
use crate::flowfield::{query_flow_dist, query_flow_next_step};
use crate::pathing::{
    can_move_to_assuming_pass_through, can_pass_through, compute_distance_within_turn_ro,
    UnitProfile, UP_CONSIDER_ZOC, UP_IGNORES_ZOC, UP_IS_LAND, UP_IS_WATER,
};
use crate::world::*;
use rayon::prelude::*;

// Output action codes returned per unit in packed triples `[action_code, move_tile, attack_tile]`
pub const ACTION_FALLBACK_KOTLIN: i32 = 0;
pub const ACTION_ATTACK: i32 = 1;
pub const ACTION_MOVE: i32 = 2;
pub const ACTION_FORTIFY_HEAL: i32 = 3;
pub const ACTION_STAY_IDLE: i32 = 4;

// Unit combat flags passed from Kotlin per eligible unit (u32)
pub const TCF_IS_MELEE: u32 = 1 << 0;
pub const TCF_IS_RANGED: u32 = 1 << 1;
pub const TCF_IS_SIEGE: u32 = 1 << 2;
pub const TCF_INDIRECT_FIRE: u32 = 1 << 3;
pub const TCF_NO_WOUNDED_PENALTY: u32 = 1 << 4;
pub const TCF_CAN_MOVE_AFTER_ATTACK: u32 = 1 << 5;
pub const TCF_CANNOT_CAPTURE_CITIES: u32 = 1 << 6;
pub const TCF_ATTACK_ACROSS_RIVER: u32 = 1 << 7;
pub const TCF_ATTACK_ACROSS_COAST: u32 = 1 << 8;
pub const TCF_MUST_SETUP: u32 = 1 << 9;
pub const TCF_IS_SETUP: u32 = 1 << 10;
pub const TCF_CAN_FORTIFY: u32 = 1 << 11;
pub const TCF_IS_EMBARKED: u32 = 1 << 12;
pub const TCF_IS_CITY_STATE: u32 = 1 << 13;

/// Per-unit input descriptor packed from Kotlin primitive arrays.
#[derive(Clone, Copy)]
pub struct TurboUnitInput {
    pub start_tile: usize,
    pub hp: i32,
    pub attack_str: f32,
    pub ranged_str: f32,
    pub range: i32,
    pub combat_flags: u32,
    pub profile: UnitProfile,
}

#[derive(Clone, Copy)]
struct CandidateAttack {
    from_tile: i32,
    target_tile: i32,
    score: i32,
    est_dmg_to_def: i32,
    is_city: bool,
    is_melee_capture_kill: bool,
}

#[derive(Clone)]
struct UnitPlanCandidates {
    /// Top ranked attack options (sorted descending by score, up to 4 kept for conflict fallback)
    attacks: Vec<CandidateAttack>,
    /// Best tactical move options (sorted descending by score, up to 3 kept for conflict fallback)
    moves: Vec<(i32, i32)>, // (dest_tile, score)
    /// Whether fortifying / healing in place is preferred when no attack fires
    prefer_heal_here: bool,
    /// True if unit should fall back to full Kotlin automation (e.g., unusual edge case)
    fallback_kotlin: bool,
}

/// Exact Civ V / Unciv exponential combat damage formula (`BattleDamage.kt:337-351`).
/// Uses midpoint randomness (`randomnessFactor = 0.5 -> base 30.0`) matching expected combat value.
#[inline]
pub fn civ5_expected_damage(
    attacker_str: f32,
    defender_str: f32,
    attacker_hp: i32,
    defender_hp: i32,
    attacker_no_wound_penalty: bool,
    defender_no_wound_penalty: bool,
    is_ranged_attack: bool,
) -> (i32, i32) {
    let ratio = (attacker_str.max(1.0)) / (defender_str.max(1.0));
    let stronger_to_weaker = if ratio < 1.0 { 1.0 / ratio } else { ratio };
    let base_term = (stronger_to_weaker + 3.0) * 0.25;
    let base_pow4 = base_term * base_term * base_term * base_term;
    let ratio_mod = (base_pow4 + 1.0) * 0.5;

    let def_mod = if ratio < 1.0 { 1.0 / ratio_mod } else { ratio_mod };
    let atk_health_ratio = if attacker_no_wound_penalty {
        1.0
    } else {
        1.0 - ((100 - attacker_hp).max(0) as f32) / 300.0
    };
    let dmg_to_def = (30.0 * def_mod * atk_health_ratio).round() as i32;

    if is_ranged_attack {
        return (dmg_to_def.max(1), 0);
    }

    let atk_mod = if ratio > 1.0 { 1.0 / ratio_mod } else { ratio_mod };
    let def_health_ratio = if defender_no_wound_penalty {
        1.0
    } else {
        1.0 - ((100 - defender_hp).max(0) as f32) / 300.0
    };
    let dmg_to_atk = (30.0 * atk_mod * def_health_ratio).round() as i32;
    (dmg_to_def.max(1), dmg_to_atk.max(1))
}

#[inline]
fn tiles_connected_by_river(world: &NativeWorld, a: usize, b: usize) -> bool {
    let rmask = world.river_mask[a];
    if rmask == 0 {
        return false;
    }
    let base = a * 6;
    for d in 0..6 {
        if world.neighbors[base + d] == b as i32 {
            return ((rmask >> d) & 1) != 0;
        }
    }
    false
}

/// Evaluate one eligible combat unit purely read-only (Phase 1 — runs in parallel via Rayon).
fn evaluate_unit_plan(
    world: &NativeWorld,
    civ_idx: usize,
    u: &TurboUnitInput,
) -> UnitPlanCandidates {
    let mut plan = UnitPlanCandidates {
        attacks: Vec::new(),
        moves: Vec::new(),
        prefer_heal_here: false,
        fallback_kotlin: false,
    };

    if u.start_tile >= world.num_tiles || u.hp <= 0 || u.profile.current_move_f32 <= 0.05 {
        return plan;
    }

    let is_melee = (u.combat_flags & TCF_IS_MELEE) != 0;
    let is_ranged = (u.combat_flags & TCF_IS_RANGED) != 0;
    let is_siege = (u.combat_flags & TCF_IS_SIEGE) != 0;
    let is_water = (u.profile.flags & UP_IS_WATER) != 0;
    let is_land = (u.profile.flags & UP_IS_LAND) != 0;
    let is_embarked = (u.combat_flags & TCF_IS_EMBARKED) != 0;
    let must_setup = (u.combat_flags & TCF_MUST_SETUP) != 0;
    let is_setup = (u.combat_flags & TCF_IS_SETUP) != 0;
    let can_capture_cities = is_melee && (u.combat_flags & TCF_CANNOT_CAPTURE_CITIES) == 0;

    // 1. Compute exact single-turn reachable tiles via Unciv's exact fixed-point rules (lock-free RO)
    let raw_reach = compute_distance_within_turn_ro(world, &u.profile, u.start_tile, false);
    let num_reach = raw_reach.len() / 3;
    if num_reach == 0 {
        return plan;
    }

    // Collect valid tiles this unit can stop/attack from
    let mut from_tiles: Vec<i32> = Vec::with_capacity(num_reach);
    let mut from_move_bits: Vec<i32> = Vec::with_capacity(num_reach);
    let mut stoppable_tiles: Vec<usize> = Vec::with_capacity(num_reach);

    for i in 0..num_reach {
        let tid = raw_reach[i * 3];
        if tid < 0 || (tid as usize) >= world.num_tiles {
            continue;
        }
        let tu = tid as usize;
        let spent = f32::from_bits(raw_reach[i * 3 + 2] as u32);
        let move_left = (u.profile.current_move_f32 - spent).max(0.0);
        let can_stop = tu == u.start_tile || can_move_to_assuming_pass_through(world, &u.profile, tu);
        if !can_stop {
            continue;
        }
        stoppable_tiles.push(tu);

        // Must-setup siege units need >= 1.0 movement left after moving unless already set up at start_tile
        let can_attack_here = if must_setup && !is_setup && tu == u.start_tile {
            move_left >= 1.05
        } else if must_setup && tu != u.start_tile {
            move_left >= 1.05
        } else {
            move_left > 0.05 || (tu == u.start_tile && u.profile.current_move_f32 > 0.05)
        };
        if can_attack_here {
            // Land melee units should not attack from water unless amphibious
            let must_land = is_melee
                && is_land
                && (u.combat_flags & TCF_ATTACK_ACROSS_COAST) == 0
                && (world.terrain_flags[tu] & TF_WATER) != 0;
            if !must_land && world.terrain_damage[tu] <= 0 {
                from_tiles.push(tid);
                from_move_bits.push(move_left.to_bits() as i32);
            }
        }
    }

    // 2. If unit is critically wounded (<50 HP) and near enemy, prioritize retreat/heal!
    let cur_threat = world.threat_dist[u.start_tile];
    let cur_in_danger = ((world.threat_danger_bits[u.start_tile >> 6] >> (u.start_tile & 63)) & 1) != 0;
    if u.hp < 50 && cur_threat <= 3 {
        let mut best_retreat: Option<(usize, i32)> = None;
        for &cand in &stoppable_tiles {
            if cand == u.start_tile || world.terrain_damage[cand] > 0 {
                continue;
            }
            let td = world.threat_dist[cand];
            if td > cur_threat {
                let fdist = query_flow_dist(world, FLOW_FRIENDLY_CITY, cand);
                let safe_bonus = if ((world.threat_danger_bits[cand >> 6] >> (cand & 63)) & 1) == 0 {
                    40
                } else {
                    0
                };
                let score = (td as i32) * 30 + safe_bonus - (fdist.min(100) as i32);
                if best_retreat.map_or(true, |(_, bs)| score > bs) {
                    best_retreat = Some((cand, score));
                }
            }
        }
        if let Some((rt, sc)) = best_retreat {
            plan.moves.push((rt as i32, 500 + sc));
        } else {
            plan.prefer_heal_here = true;
        }
        return plan;
    }

    // Healthy-ish wounded unit (<85 HP) safe from enemies -> heal in place
    if u.hp < 85 && !cur_in_danger && cur_threat > 3 && world.terrain_damage[u.start_tile] <= 0 {
        plan.prefer_heal_here = true;
        return plan;
    }

    // 3. Evaluate all attackable enemy units & cities using exact LOS + Civ V damage formula!
    if !world.threat_enemy_tiles.is_empty() && !from_tiles.is_empty() && !(is_embarked && is_melee) {
        let indirect = (u.combat_flags & TCF_INDIRECT_FIRE) != 0;
        let eff_range = if is_ranged { u.range.max(1) } else { 1 };
        let pairs = find_attackable_pairs(
            world,
            civ_idx,
            &from_tiles,
            &from_move_bits,
            eff_range,
            is_melee && !is_ranged,
            indirect,
            &world.threat_enemy_tiles,
        );

        let no_wound = (u.combat_flags & TCF_NO_WOUNDED_PENALTY) != 0;
        let atk_across_river = (u.combat_flags & TCF_ATTACK_ACROSS_RIVER) != 0;
        let base_atk_str = if is_ranged { u.ranged_str } else { u.attack_str };

        let n_pairs = pairs.len() / 3;
        for pi in 0..n_pairs {
            let ft = pairs[pi * 3] as usize;
            let tt = pairs[pi * 3 + 1] as usize;
            let is_city = (world.tile_flags[tt] & TILE_CITY_CENTER) != 0
                && world.owner_civ[tt] >= 0
                && world.is_at_war(civ_idx, world.owner_civ[tt] as usize);
            let has_enemy_mil = world.mil_unit_civ[tt] >= 0
                && world.is_at_war(civ_idx, world.mil_unit_civ[tt] as usize);

            if !is_city && !has_enemy_mil {
                continue;
            }

            // Modifiers: flanking (melee), river crossing, terrain defense, siege vs city
            let mut atk_mod_pct = 0i32;
            if is_melee && !is_ranged {
                if !atk_across_river && tiles_connected_by_river(world, ft, tt) {
                    atk_mod_pct -= 20;
                }
                // Count adjacent friendly melee units for flanking (+10% each)
                let base_t = tt * 6;
                let mut flankers = 0i32;
                for d in 0..6 {
                    let nb = world.neighbors[base_t + d];
                    if nb >= 0 && nb as usize != ft && nb as usize != u.start_tile {
                        let nu = nb as usize;
                        if world.mil_unit_civ[nu] == civ_idx as i16
                            && (world.mil_unit_flags[nu] & UF_RANGED) == 0
                        {
                            flankers += 1;
                        }
                    }
                }
                atk_mod_pct += flankers * 10;
            }
            if is_city && is_siege {
                atk_mod_pct += 100; // Siege units excel vs cities
            }

            let eff_atk_str = (base_atk_str * (1.0 + atk_mod_pct as f32 / 100.0)).max(1.0);

            let (def_str, def_hp, def_no_wound) = if is_city {
                let cs = world.city_strength[tt].max(4) as f32;
                let chp = world.city_hp[tt] as i32;
                (cs, chp.max(1), true)
            } else {
                let us = world.mil_unit_strength[tt].max(4) as f32;
                let tdef = if (world.mil_unit_flags[tt] & UF_EMBARKED) != 0 {
                    0
                } else {
                    world.tile_def_bonus_pct[tt] as i32
                };
                let eff_def = (us * (1.0 + tdef as f32 / 100.0)).max(1.0);
                let uhp = world.mil_unit_hp[tt].max(1) as i32;
                (eff_def, uhp, false)
            };

            let (dmg_to_def, dmg_to_atk) = civ5_expected_damage(
                eff_atk_str,
                def_str,
                u.hp,
                def_hp,
                no_wound,
                def_no_wound,
                is_ranged,
            );

            // Never take suicidal attacks where attacker is likely to die!
            if !is_ranged && dmg_to_atk >= u.hp {
                continue;
            }

            let mut score = if is_city {
                if def_hp == 1 {
                    if can_capture_cities {
                        10_000
                    } else {
                        0 // Ranged units shouldn't waste fire on a 1-HP city
                    }
                } else if can_capture_cities && dmg_to_def >= def_hp {
                    10_000
                } else {
                    let mut s = if is_ranged { 115 } else { 95 };
                    if is_siege {
                        s += 45;
                    }
                    s += dmg_to_def - dmg_to_atk;
                    if !is_ranged && u.hp - dmg_to_atk * 2 <= 0 {
                        s -= 85;
                    }
                    s
                }
            } else {
                // Unit vs Unit attack value (matching Unciv's BattleHelper priorities)
                let mut s = dmg_to_def * 2 - dmg_to_atk;
                if dmg_to_def >= def_hp {
                    s += 65; // Strong kill-shot priority!
                }
                if (world.mil_unit_flags[tt] & UF_RANGED) != 0 {
                    s += 15; // Prioritize eliminating ranged/siege threats
                }
                if !is_ranged && ft != u.start_tile {
                    // Prefer attacking from higher defense terrain
                    s += (world.tile_def_bonus_pct[ft] as i32) / 5;
                }
                s
            };

            // Slight preference for attacking from current tile to preserve positioning/movement
            if ft == u.start_tile {
                score += 4;
            }

            if score > 30 {
                plan.attacks.push(CandidateAttack {
                    from_tile: ft as i32,
                    target_tile: tt as i32,
                    score,
                    est_dmg_to_def: dmg_to_def,
                    is_city,
                    is_melee_capture_kill: is_city && can_capture_cities && dmg_to_def >= def_hp,
                });
            }
        }

        if !plan.attacks.is_empty() {
            plan.attacks.sort_unstable_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| a.target_tile.cmp(&b.target_tile))
                    .then_with(|| a.from_tile.cmp(&b.from_tile))
            });
            plan.attacks.truncate(4);
        }
    }

    // 4. Strategic Movement via Precomputed O(V) Theater Flow-Fields!
    // Priority order: (a) Defend sieged city -> (b) Advance toward priority enemy city ->
    // (c) Advance toward close enemy -> (d) Garrison undefended own city -> (e) Explore frontier
    let primary_field = if !is_water && query_flow_dist(world, FLOW_SIEGED_CITY, u.start_tile) < 30 {
        FLOW_SIEGED_CITY
    } else if is_water {
        FLOW_ENEMY_CITY_WATER
    } else {
        FLOW_ENEMY_CITY_LAND
    };

    let cur_primary = query_flow_dist(world, primary_field, u.start_tile);
    let mut best_moves: Vec<(i32, i32)> = Vec::new();
    let min_flow_dist = if is_ranged { 4 } else { 2 };

    if cur_primary < FLOW_UNREACHED && (cur_primary > min_flow_dist || is_embarked) {
        // Pick reachable stoppable tiles that minimize flow-field distance!
        for &cand in &stoppable_tiles {
            if cand == u.start_tile || world.terrain_damage[cand] > 0 {
                continue;
            }
            // Don't move a ranged unit straight next to an enemy unless safe
            let cd = query_flow_dist(world, primary_field, cand);
            if cd < cur_primary {
                let gain = (cur_primary - cd) as i32;
                let mut mscore = 200 + gain * 20;
                if !is_water && !is_ranged {
                    mscore += (world.tile_def_bonus_pct[cand] as i32) / 5;
                }
                if is_ranged && world.threat_dist[cand] <= 1 {
                    mscore -= 35; // Keep ranged units 2 hexes back from enemy frontline
                }
                best_moves.push((cand as i32, mscore));
            }
        }
    }

    // If no flow improvement found, check direct threat gradient for close enemies (<= 5 hexes)
    if best_moves.is_empty() && cur_threat > 1 && cur_threat <= 6 {
        for &cand in &stoppable_tiles {
            if cand == u.start_tile || world.terrain_damage[cand] > 0 {
                continue;
            }
            let td = world.threat_dist[cand];
            let target_band = if is_ranged { 2u8 } else { 1u8 };
            if td < cur_threat && td >= target_band {
                let mscore = 140 + ((cur_threat - td) as i32) * 18;
                best_moves.push((cand as i32, mscore));
            }
        }
    }

    // If still idle (e.g. peacetime), try garrisoning an empty own city nearby or exploring frontier!
    if best_moves.is_empty() && !is_water {
        // Already garrisoned in own city during peacetime -> stay fortified!
        let on_own_city = (world.tile_flags[u.start_tile] & TILE_CITY_CENTER) != 0
            && world.owner_civ[u.start_tile] == civ_idx as i16;
        if on_own_city && cur_threat > 5 {
            plan.prefer_heal_here = true;
        } else {
            // Try stepping into a reachable ungarrisoned own city
            for &cand in &stoppable_tiles {
                if cand != u.start_tile
                    && (world.tile_flags[cand] & TILE_CITY_CENTER) != 0
                    && world.owner_civ[cand] == civ_idx as i16
                    && world.mil_unit_civ[cand] == NO_CIV
                {
                    best_moves.push((cand as i32, 120));
                }
            }
        }
    }

    if best_moves.is_empty()
        && !plan.prefer_heal_here
        && (u.combat_flags & TCF_IS_CITY_STATE) == 0
    {
        // Explore safe frontier using O(V) explore_frontier flow-field!
        let cur_exp = query_flow_dist(world, FLOW_EXPLORE_FRONTIER, u.start_tile);
        if cur_exp < FLOW_UNREACHED && cur_exp > 0 {
            for &cand in &stoppable_tiles {
                if cand == u.start_tile || world.terrain_damage[cand] > 0 {
                    continue;
                }
                let ed = query_flow_dist(world, FLOW_EXPLORE_FRONTIER, cand);
                if ed < cur_exp && ((world.threat_danger_bits[cand >> 6] >> (cand & 63)) & 1) == 0 {
                    best_moves.push((cand as i32, 80 + ((cur_exp - ed) as i32) * 12));
                }
            }
        } else {
            // Fallback single-step check if flow next exists and is reachable
            let nxt = query_flow_next_step(world, primary_field, u.start_tile);
            if nxt >= 0
                && nxt as usize != u.start_tile
                && stoppable_tiles.contains(&(nxt as usize))
                && world.terrain_damage[nxt as usize] <= 0
                && can_pass_through(world, &u.profile, nxt as usize)
            {
                best_moves.push((nxt, 70));
            }
        }
    }

    if !best_moves.is_empty() {
        best_moves.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        best_moves.truncate(3);
        plan.moves = best_moves;
    }

    plan
}

/// 2-Phase Deterministic Multi-Threaded Combat Unit Planner for one AI Civilization.
///
/// Input: `units` slice sorted in Unciv's priority order.
/// Output: flat `Vec<i32>` of length `units.len() * 3`:
///   `[action_code, move_tile_idx, attack_tile_idx]` for each unit `i`.
pub fn plan_civ_combat_units(
    world: &mut NativeWorld,
    civ_idx: usize,
    units: &[TurboUnitInput],
) -> Vec<i32> {
    let mut out = vec![ACTION_STAY_IDLE; units.len() * 3];
    if units.is_empty() || civ_idx >= world.num_civs || world.num_tiles == 0 {
        return out;
    }

    // Ensure ZoC bitsets are warm BEFORE entering lock-free parallel Phase 1!
    let needs_zoc = units
        .iter()
        .any(|u| (u.profile.flags & UP_CONSIDER_ZOC) != 0 && (u.profile.flags & UP_IGNORES_ZOC) == 0);
    if needs_zoc {
        world.ensure_zoc(civ_idx);
    }

    // Phase 1: Parallel read-only unit planning across bounded Rayon worker threads!
    let w_ref: &NativeWorld = world;
    let candidates: Vec<UnitPlanCandidates> = if units.len() >= 8 {
        with_rayon_pool(|| {
            units
                .par_iter()
                .map(|u| evaluate_unit_plan(w_ref, civ_idx, u))
                .collect()
        })
    } else {
        units
            .iter()
            .map(|u| evaluate_unit_plan(w_ref, civ_idx, u))
            .collect()
    };

    // Phase 2: Deterministic sequential reservation in priority order (prevents collisions & overkill!)
    let n = world.num_tiles;
    let mut reserved_mil = vec![false; n];
    for u in units {
        if u.start_tile < n {
            reserved_mil[u.start_tile] = true;
        }
    }
    let mut proj_enemy_hp: Vec<i32> = world.mil_unit_hp.iter().map(|&h| h as i32).collect();
    let mut proj_city_hp: Vec<i32> = world.city_hp.iter().map(|&h| h as i32).collect();

    for (i, (u, cand)) in units.iter().zip(candidates.iter()).enumerate() {
        let base = i * 3;
        if cand.fallback_kotlin {
            out[base] = ACTION_FALLBACK_KOTLIN;
            out[base + 1] = -1;
            out[base + 2] = -1;
            continue;
        }

        // Try best conflict-free attack first
        let mut chosen_attack: Option<CandidateAttack> = None;
        for &atk in &cand.attacks {
            let ft = atk.from_tile as usize;
            let tt = atk.target_tile as usize;
            if ft != u.start_tile && reserved_mil[ft] {
                continue;
            }
            if atk.is_city {
                if proj_city_hp[tt] <= 0 {
                    continue; // Already projected captured!
                }
                if proj_city_hp[tt] == 1 && !atk.is_melee_capture_kill {
                    continue; // Leave 1-HP city for melee capture
                }
            } else if proj_enemy_hp[tt] <= 0 {
                continue; // Already projected killed by higher-priority unit!
            }
            chosen_attack = Some(atk);
            break;
        }

        if let Some(atk) = chosen_attack {
            let ft = atk.from_tile as usize;
            let tt = atk.target_tile as usize;
            let is_melee = (u.combat_flags & TCF_IS_MELEE) != 0 && (u.combat_flags & TCF_IS_RANGED) == 0;
            if ft != u.start_tile {
                reserved_mil[u.start_tile] = false;
                reserved_mil[ft] = true;
            }
            if atk.is_city {
                if atk.is_melee_capture_kill {
                    proj_city_hp[tt] = 0;
                    reserved_mil[ft] = false;
                    reserved_mil[tt] = true;
                } else {
                    proj_city_hp[tt] = (proj_city_hp[tt] - atk.est_dmg_to_def).max(1);
                }
            } else {
                proj_enemy_hp[tt] -= atk.est_dmg_to_def;
                if is_melee && proj_enemy_hp[tt] <= 0 {
                    reserved_mil[ft] = false;
                    reserved_mil[tt] = true;
                }
            }
            out[base] = ACTION_ATTACK;
            out[base + 1] = atk.from_tile;
            out[base + 2] = atk.target_tile;
            continue;
        }

        // Try best conflict-free strategic move
        let mut chosen_move: Option<i32> = None;
        for &(dest, _) in &cand.moves {
            let du = dest as usize;
            if du < n && !reserved_mil[du] {
                chosen_move = Some(dest);
                break;
            }
        }

        if let Some(dest) = chosen_move {
            reserved_mil[u.start_tile] = false;
            reserved_mil[dest as usize] = true;
            out[base] = ACTION_MOVE;
            out[base + 1] = dest;
            out[base + 2] = -1;
        } else if cand.prefer_heal_here && (u.combat_flags & TCF_CAN_FORTIFY) != 0 {
            out[base] = ACTION_FORTIFY_HEAL;
            out[base + 1] = u.start_tile as i32;
            out[base + 2] = -1;
        } else {
            out[base] = ACTION_STAY_IDLE;
            out[base + 1] = -1;
            out[base + 2] = -1;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::build_threat_field;
    use crate::flowfield::build_civ_flow_fields;
    use crate::pathing::tests::make_hex_world;
    use crate::pathing::fpm_from_f32;

    #[test]
    fn civ5_damage_matches_unciv_formula() {
        // Equal strength (ratio = 1.0, full HP) -> 30 dmg each
        let (d_def, d_atk) = civ5_expected_damage(20.0, 20.0, 100, 100, false, false, false);
        assert_eq!(d_def, 30);
        assert_eq!(d_atk, 30);

        // Ranged attack takes 0 counter-damage
        let (r_def, r_atk) = civ5_expected_damage(20.0, 20.0, 100, 100, false, false, true);
        assert_eq!(r_def, 30);
        assert_eq!(r_atk, 0);

        // 2:1 strength advantage: ((2+3)/4)^4 = 2.4414 -> ratio_mod = 1.7207 -> 52 dealt, 17 taken
        let (hi_def, hi_atk) = civ5_expected_damage(40.0, 20.0, 100, 100, false, false, false);
        assert_eq!(hi_def, 52);
        assert_eq!(hi_atk, 17);
    }

    #[test]
    fn turbo_ai_plans_attacks_and_prevents_overkill_and_collisions() {
        let (mut w, map) = make_hex_world(4, 2);
        for t in 0..w.num_tiles {
            w.set_explored_bit(0, t, true);
            w.set_visible_bit(0, t, true);
        }
        w.diplo_matrix[0 * 2 + 1] = DIPLO_AT_WAR | DIPLO_CAN_PASS;
        w.diplo_matrix[1 * 2 + 0] = DIPLO_AT_WAR | DIPLO_CAN_PASS;

        // Weak enemy unit with 15 HP at (1, 0), enemy city at (3, 0)
        let enemy_t = map[&(1, 0)];
        let enemy_c = map[&(3, 0)];
        w.mil_unit_civ[enemy_t] = 1;
        w.mil_unit_hp[enemy_t] = 15;
        w.mil_unit_strength[enemy_t] = 10;
        w.tile_flags[enemy_c] |= TILE_CITY_CENTER;
        w.owner_civ[enemy_c] = 1;
        w.city_hp[enemy_c] = 200;
        w.city_max_hp[enemy_c] = 200;
        w.city_strength[enemy_c] = 18;

        // Place 2 friendly warriors at (0, 0) and (0, 1)
        let u0_t = map[&(0, 0)];
        let u1_t = map[&(0, 1)];
        w.mil_unit_civ[u0_t] = 0;
        w.mil_unit_hp[u0_t] = 100;
        w.mil_unit_strength[u0_t] = 20;
        w.mil_unit_civ[u1_t] = 0;
        w.mil_unit_hp[u1_t] = 100;
        w.mil_unit_strength[u1_t] = 20;

        build_threat_field(&mut w, 0, &[]);
        build_civ_flow_fields(&mut w, 0, &[enemy_c as i32], &[], &[]);

        let make_unit = |tile: usize| TurboUnitInput {
            start_tile: tile,
            hp: 100,
            attack_str: 20.0,
            ranged_str: 0.0,
            range: 1,
            combat_flags: TCF_IS_MELEE | TCF_CAN_FORTIFY,
            profile: UnitProfile {
                civ_idx: 0,
                flags: UP_IS_LAND | UP_CONSIDER_ZOC,
                current_move_f32: 2.0,
                current_move_fpm: fpm_from_f32(2.0),
                full_move_fpm: fpm_from_f32(2.0),
                cost_to_embark_fpm: -1,
                cost_to_disembark_fpm: -1,
                enemy_penalty_fpm: 0,
                dm_feature_mask: 0,
                dm_base_mask: 0,
                dm_hill_mask: 0,
            },
        };

        let units = vec![make_unit(u0_t), make_unit(u1_t)];
        let res = plan_civ_combat_units(&mut w, 0, &units);
        // Unit 0 should kill enemy_t; Unit 1 must NOT also attack already-dead enemy_t and should advance via flow-field!
        assert_eq!(res[0], ACTION_ATTACK);
        assert_eq!(res[2], enemy_t as i32);
        assert_eq!(res[3], ACTION_MOVE);
        assert_ne!(res[4], enemy_t as i32);
    }
}
