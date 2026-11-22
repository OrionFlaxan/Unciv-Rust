//! Structure-of-Arrays (SoA) native mirror of Unciv's `TileMap`, `Civilization` diplomacy,
//! unit occupancy, exploration/vision bitsets, precomputed ZoC masks, and multi-source threat field.
//!
//! All data is laid out in dense flat vectors indexed by `tile.zeroBasedIndex` (`0..num_tiles`)
//! and `civ.civIndex` (`0..num_civs`) so hot loops operate entirely in L1/L2 cache with zero
//! heap allocations and zero callbacks into the JVM.

// Terrain static flags (u32)
pub const TF_LAND: u32 = 1 << 0;
pub const TF_WATER: u32 = 1 << 1;
pub const TF_OCEAN: u32 = 1 << 2;
pub const TF_IMPASSABLE: u32 = 1 << 3;
pub const TF_HILL: u32 = 1 << 4;
pub const TF_ROUGH: u32 = 1 << 5;
pub const TF_ICE: u32 = 1 << 6;
pub const TF_FOREST_OR_JUNGLE: u32 = 1 << 7;
pub const TF_ADJ_RIVER: u32 = 1 << 8;
pub const TF_ADJ_COAST: u32 = 1 << 9;
pub const TF_NATURAL_WONDER: u32 = 1 << 10;

// Dynamic tile flags (u16)
pub const TILE_ROAD_MASK: u16 = 0b11; // 0=None, 1=Road, 2=Railroad
pub const TILE_ROAD_NONE: u16 = 0;
pub const TILE_ROAD_ROAD: u16 = 1;
pub const TILE_ROAD_RAILROAD: u16 = 2;
pub const TILE_ROAD_PILLAGED: u16 = 1 << 2;
pub const TILE_CITY_CENTER: u16 = 1 << 3;
pub const TILE_JUST_CONQUERED: u16 = 1 << 4;
pub const TILE_ENCAMPMENT: u16 = 1 << 5;
pub const TILE_RUIN: u16 = 1 << 6;
pub const TILE_CITY_BLOCKADED: u16 = 1 << 7;
pub const TILE_HAS_HARBOR: u16 = 1 << 8;

// Military unit flags on tile (u8)
pub const UF_WATER: u8 = 1 << 0;
pub const UF_EMBARKED: u8 = 1 << 1;
pub const UF_RANGED: u8 = 1 << 2;
pub const UF_INVISIBLE: u8 = 1 << 3;
pub const UF_RANGE_SHIFT: u8 = 4; // bits 4..7 = attack range (0..15)

// Diplomacy matrix flags per (civ_a, civ_b) (u8)
pub const DIPLO_AT_WAR: u8 = 1 << 0;
pub const DIPLO_CAN_PASS: u8 = 1 << 1;
pub const DIPLO_OPEN_BORDERS: u8 = 1 << 2;
pub const DIPLO_FRIENDLY_TERRITORY: u8 = 1 << 3;
pub const DIPLO_KNOWS: u8 = 1 << 4;
pub const DIPLO_REL_SHIFT: u8 = 5; // bits 5..7 = RelationshipLevel.ordinal (0..7)
pub const DIPLO_REL_FAVORABLE: u8 = 5; // RelationshipLevel.Favorable.ordinal

// Per-civ flags (u32)
pub const CF_ROADS_ACROSS_RIVERS: u32 = 1 << 0;
pub const CF_FOREST_AS_ROAD: u32 = 1 << 1;
pub const CF_IGNORE_HILL: u32 = 1 << 2;
pub const CF_CAN_EMBARK: u32 = 1 << 3;
pub const CF_EMBARK_OCEAN: u32 = 1 << 4;
pub const CF_ALL_OCEAN: u32 = 1 << 5;
pub const CF_PASS_IMPASSABLE: u32 = 1 << 6;
pub const CF_HAS_ENEMY_PENALTY: u32 = 1 << 7;
pub const CF_IS_CITY_STATE: u32 = 1 << 8;
pub const CF_IS_BARBARIAN: u32 = 1 << 9;
pub const CF_SPECTATOR_VISION: u32 = 1 << 10;

pub const NO_TERRAIN_ID: u8 = 255;
pub const NO_CIV: i16 = -1;
pub const THREAT_UNREACHED: u8 = 255;

/// Stateful native world mirror owned by a `TileMap`'s `NativeWorldMirror`.
pub struct NativeWorld {
    pub num_tiles: usize,
    pub num_civs: usize,
    pub bitset_words: usize,
    pub world_wrap: bool,
    pub wrap_radius: i32,
    pub map_width: i32,

    // Static / terrain-change topology
    pub pos_x: Vec<i16>,
    pub pos_y: Vec<i16>,
    /// `neighbors[tile * 6 + d]` where `d` in 0..6 corresponds to clock 2, 4, 6, 8, 10, 12; -1 if absent.
    pub neighbors: Vec<i32>,
    /// Bit `d` set iff edge from `tile` to `neighbors[tile * 6 + d]` crosses a river.
    pub river_mask: Vec<u8>,
    pub terrain_flags: Vec<u32>,
    /// `tile.lastTerrain.movementCost` as integer (usually 1 or 2; city center reduction is applied in cost calc).
    pub base_move_cost: Vec<u8>,
    /// `tile.allTerrains.sumOf { damagePerTurn }` (base terrain damage; 0 for almost all tiles).
    pub terrain_damage: Vec<i16>,
    pub tile_height: Vec<i8>,
    pub unit_height: Vec<i8>,
    pub continent: Vec<i16>,
    pub base_terrain_id: Vec<u8>,
    pub hill_terrain_id: Vec<u8>,
    pub last_terrain_id: Vec<u8>,
    pub feature_terrain_mask: Vec<u64>,

    // Dynamic turn/unit state
    pub owner_civ: Vec<i16>,
    pub tile_flags: Vec<u16>,
    pub mil_unit_civ: Vec<i16>,
    pub mil_unit_flags: Vec<u8>,
    pub civ_unit_civ: Vec<i16>,
    pub air_unit_civ: Vec<i16>,
    pub city_bombard_range: Vec<u8>,

    // Per-civ bitsets & vision refcounts
    pub explored_bits: Vec<u64>,
    pub visible_bits: Vec<u64>,
    pub vision_refcount: Vec<u16>,
    pub vision_synced: Vec<bool>,

    // Diplomacy & civ movement properties
    pub diplo_matrix: Vec<u8>,
    pub civ_flags: Vec<u32>,
    pub civ_road_speed_fpm: Vec<u16>,
    pub civ_road_speed_f32: Vec<f32>,
    pub civ_passable_impassable_mask: Vec<u64>,

    // Active-civ ZoC acceleration
    pub zoc_civ: i32,
    pub zoc_dirty: bool,
    pub zoc_land_bits: Vec<u64>,
    pub zoc_water_bits: Vec<u64>,

    // Active-civ multi-source threat field
    pub threat_civ: i32,
    pub threat_dirty: bool,
    pub threat_max_dist: u8,
    pub threat_dist: Vec<u8>,
    pub threat_enemy_tiles: Vec<i32>,
    pub threat_danger_bits: Vec<u64>,
}

impl NativeWorld {
    pub fn new(
        num_tiles: usize,
        num_civs: usize,
        world_wrap: bool,
        wrap_radius: i32,
        map_width: i32,
    ) -> Self {
        let bitset_words = (num_tiles + 63) / 64;
        Self {
            num_tiles,
            num_civs,
            bitset_words,
            world_wrap,
            wrap_radius,
            map_width,
            pos_x: vec![0; num_tiles],
            pos_y: vec![0; num_tiles],
            neighbors: vec![-1; num_tiles * 6],
            river_mask: vec![0; num_tiles],
            terrain_flags: vec![0; num_tiles],
            base_move_cost: vec![1; num_tiles],
            terrain_damage: vec![0; num_tiles],
            tile_height: vec![0; num_tiles],
            unit_height: vec![0; num_tiles],
            continent: vec![-1; num_tiles],
            base_terrain_id: vec![NO_TERRAIN_ID; num_tiles],
            hill_terrain_id: vec![NO_TERRAIN_ID; num_tiles],
            last_terrain_id: vec![NO_TERRAIN_ID; num_tiles],
            feature_terrain_mask: vec![0; num_tiles],

            owner_civ: vec![NO_CIV; num_tiles],
            tile_flags: vec![0; num_tiles],
            mil_unit_civ: vec![NO_CIV; num_tiles],
            mil_unit_flags: vec![0; num_tiles],
            civ_unit_civ: vec![NO_CIV; num_tiles],
            air_unit_civ: vec![NO_CIV; num_tiles],
            city_bombard_range: vec![0; num_tiles],

            explored_bits: vec![0; num_civs * bitset_words],
            visible_bits: vec![0; num_civs * bitset_words],
            vision_refcount: vec![0; num_civs * num_tiles],
            vision_synced: vec![false; num_civs],

            diplo_matrix: vec![DIPLO_REL_FAVORABLE << DIPLO_REL_SHIFT; num_civs * num_civs],
            civ_flags: vec![0; num_civs],
            civ_road_speed_fpm: vec![15; num_civs],
            civ_road_speed_f32: vec![0.5; num_civs],
            civ_passable_impassable_mask: vec![0; num_civs],

            zoc_civ: -1,
            zoc_dirty: true,
            zoc_land_bits: vec![0; bitset_words],
            zoc_water_bits: vec![0; bitset_words],

            threat_civ: -1,
            threat_dirty: true,
            threat_max_dist: 0,
            threat_dist: vec![THREAT_UNREACHED; num_tiles],
            threat_enemy_tiles: Vec::new(),
            threat_danger_bits: vec![0; bitset_words],
        }
    }

    pub fn resize_civs(&mut self, num_civs: usize) {
        if self.num_civs == num_civs {
            return;
        }
        self.num_civs = num_civs;
        self.explored_bits.resize(num_civs * self.bitset_words, 0);
        self.visible_bits.resize(num_civs * self.bitset_words, 0);
        self.vision_refcount.resize(num_civs * self.num_tiles, 0);
        self.vision_synced = vec![false; num_civs];
        self.diplo_matrix = vec![DIPLO_REL_FAVORABLE << DIPLO_REL_SHIFT; num_civs * num_civs];
        self.civ_flags.resize(num_civs, 0);
        self.civ_road_speed_fpm.resize(num_civs, 15);
        self.civ_road_speed_f32.resize(num_civs, 0.5);
        self.civ_passable_impassable_mask.resize(num_civs, 0);
        self.zoc_dirty = true;
        self.threat_dirty = true;
    }

    #[inline(always)]
    pub fn diplo(&self, civ_a: usize, civ_b: usize) -> u8 {
        self.diplo_matrix[civ_a * self.num_civs + civ_b]
    }

    #[inline(always)]
    pub fn is_at_war(&self, civ_a: usize, civ_b: usize) -> bool {
        if civ_a == civ_b {
            return false;
        }
        (self.diplo(civ_a, civ_b) & DIPLO_AT_WAR) != 0
    }

    #[inline(always)]
    pub fn can_pass_borders(&self, civ_a: usize, civ_b: usize) -> bool {
        if civ_a == civ_b {
            return true;
        }
        (self.diplo(civ_a, civ_b) & DIPLO_CAN_PASS) != 0
    }

    #[inline(always)]
    pub fn has_open_borders_conn(&self, civ_a: usize, civ_b: usize) -> bool {
        if civ_a == civ_b {
            return true;
        }
        (self.diplo(civ_a, civ_b) & DIPLO_OPEN_BORDERS) != 0
    }

    #[inline(always)]
    pub fn is_friendly_territory(&self, civ_a: usize, owner: i16) -> bool {
        if owner < 0 {
            return false;
        }
        let o = owner as usize;
        if o == civ_a {
            return true;
        }
        (self.diplo(civ_a, o) & DIPLO_FRIENDLY_TERRITORY) != 0
    }

    #[inline(always)]
    pub fn relationship_level(&self, civ_a: usize, owner: i16) -> u8 {
        if owner < 0 {
            return DIPLO_REL_FAVORABLE;
        }
        let o = owner as usize;
        if o == civ_a {
            return DIPLO_REL_FAVORABLE;
        }
        (self.diplo(civ_a, o) >> DIPLO_REL_SHIFT) & 0x7
    }

    #[inline(always)]
    pub fn is_explored(&self, civ: usize, tile: usize) -> bool {
        if (self.civ_flags[civ] & CF_SPECTATOR_VISION) != 0 {
            return true;
        }
        let w = tile >> 6;
        let b = tile & 63;
        ((self.explored_bits[civ * self.bitset_words + w] >> b) & 1) != 0
    }

    #[inline(always)]
    pub fn set_explored_bit(&mut self, civ: usize, tile: usize, val: bool) {
        let idx = civ * self.bitset_words + (tile >> 6);
        let mask = 1u64 << (tile & 63);
        if val {
            self.explored_bits[idx] |= mask;
        } else {
            self.explored_bits[idx] &= !mask;
        }
    }

    #[inline(always)]
    pub fn is_visible(&self, civ: usize, tile: usize) -> bool {
        if (self.civ_flags[civ] & CF_SPECTATOR_VISION) != 0 {
            return true;
        }
        let w = tile >> 6;
        let b = tile & 63;
        ((self.visible_bits[civ * self.bitset_words + w] >> b) & 1) != 0
    }

    #[inline(always)]
    pub fn set_visible_bit(&mut self, civ: usize, tile: usize, val: bool) {
        let idx = civ * self.bitset_words + (tile >> 6);
        let mask = 1u64 << (tile & 63);
        if val {
            self.visible_bits[idx] |= mask;
        } else {
            self.visible_bits[idx] &= !mask;
        }
    }

    /// Exact port of `Tile.aerialDistanceTo` including cylindrical world wrap.
    #[inline(always)]
    pub fn aerial_distance(&self, a: usize, b: usize) -> i32 {
        let x1 = self.pos_x[a] as i32;
        let y1 = self.pos_y[a] as i32;
        let x2 = self.pos_x[b] as i32;
        let y2 = self.pos_y[b] as i32;

        let dx = x1 - x2;
        let dy = y1 - y2;
        let dist = dx.abs().max(dy.abs()).max((dx - dy).abs());

        if !self.world_wrap || dist <= self.map_width / 2 {
            return dist;
        }

        // Unwrap b's position (exact match to TileMap.getUnwrappedPosition)
        let r = self.wrap_radius;
        let lx = x2 + r;
        let ly = y2 - r;
        let rx = x2 - r;
        let ry = y2 + r;
        let (ux, uy) = if rx * rx + ry * ry < lx * lx + ly * ly {
            (rx, ry)
        } else {
            (lx, ly)
        };
        let wdx = x1 - ux;
        let wdy = y1 - uy;
        wdx.abs().max(wdy.abs()).max((wdx - wdy).abs())
    }

    #[inline(always)]
    pub fn unpillaged_road(&self, tile: usize) -> u16 {
        let f = self.tile_flags[tile];
        if (f & TILE_ROAD_PILLAGED) != 0 {
            TILE_ROAD_NONE
        } else {
            f & TILE_ROAD_MASK
        }
    }

    #[inline(always)]
    pub fn forest_or_jungle_are_roads(&self, civ: usize, tile: usize) -> bool {
        (self.civ_flags[civ] & CF_FOREST_AS_ROAD) != 0
            && (self.terrain_flags[tile] & TF_FOREST_OR_JUNGLE) != 0
            && self.is_friendly_territory(civ, self.owner_civ[tile])
    }

    #[inline(always)]
    pub fn has_connection(&self, civ: usize, tile: usize) -> bool {
        self.unpillaged_road(tile) != TILE_ROAD_NONE || self.forest_or_jungle_are_roads(civ, tile)
    }

    #[inline(always)]
    pub fn has_road_or_rail_any(&self, civ: usize, tile: usize) -> bool {
        (self.tile_flags[tile] & TILE_ROAD_MASK) != TILE_ROAD_NONE
            || self.forest_or_jungle_are_roads(civ, tile)
    }

    #[inline(always)]
    pub fn can_civ_pass_through(&self, civ: usize, tile: usize) -> bool {
        let owner = self.owner_civ[tile];
        if owner < 0 || owner as usize == civ {
            return true;
        }
        let owner_u = owner as usize;
        let tf = self.tile_flags[tile];
        if (tf & TILE_CITY_CENTER) != 0
            && self.is_at_war(civ, owner_u)
            && (tf & TILE_JUST_CONQUERED) == 0
        {
            return false;
        }
        self.can_pass_borders(civ, owner_u)
    }

    /// Rebuilds precomputed ZoC bitsets for `civ_idx` in O(num_tiles).
    /// A tile has its ZoC bit set iff at least one of its 6 neighbors exerts ZoC against `civ_idx`.
    pub fn ensure_zoc(&mut self, civ_idx: usize) {
        if !self.zoc_dirty && self.zoc_civ == civ_idx as i32 {
            return;
        }
        self.zoc_land_bits.fill(0);
        self.zoc_water_bits.fill(0);
        let n = self.num_tiles;
        for t in 0..n {
            let tf = self.tile_flags[t];
            let mut exerts_land = false;
            let mut exerts_water = false;

            if (tf & TILE_CITY_CENTER) != 0 {
                let owner = self.owner_civ[t];
                if owner >= 0 && self.is_at_war(civ_idx, owner as usize) {
                    exerts_land = true;
                    exerts_water = true;
                }
            } else {
                let mc = self.mil_unit_civ[t];
                if mc >= 0 && self.is_at_war(civ_idx, mc as usize) {
                    let mf = self.mil_unit_flags[t];
                    if (mf & UF_WATER) != 0 {
                        exerts_land = true;
                        exerts_water = true;
                    } else if (mf & UF_EMBARKED) == 0 {
                        exerts_land = true;
                    }
                }
            }

            if !exerts_land && !exerts_water {
                continue;
            }
            let base = t * 6;
            for d in 0..6 {
                let nb = self.neighbors[base + d];
                if nb < 0 {
                    continue;
                }
                let u = nb as usize;
                let w = u >> 6;
                let mask = 1u64 << (u & 63);
                if exerts_land {
                    self.zoc_land_bits[w] |= mask;
                }
                if exerts_water {
                    self.zoc_water_bits[w] |= mask;
                }
            }
        }
        self.zoc_civ = civ_idx as i32;
        self.zoc_dirty = false;
    }

    /// Incremental patch when a tile's unit occupancy changes.
    #[inline]
    pub fn patch_tile_units(
        &mut self,
        tile: usize,
        mil_civ: i16,
        mil_flags: u8,
        civ_civ: i16,
        air_civ: i16,
    ) {
        if tile >= self.num_tiles {
            return;
        }
        if self.mil_unit_civ[tile] != mil_civ || self.mil_unit_flags[tile] != mil_flags {
            self.zoc_dirty = true;
            self.threat_dirty = true;
        }
        self.mil_unit_civ[tile] = mil_civ;
        self.mil_unit_flags[tile] = mil_flags;
        self.civ_unit_civ[tile] = civ_civ;
        self.air_unit_civ[tile] = air_civ;
    }

    /// Incremental patch when a tile's ownership, road, or city flags change.
    #[inline]
    pub fn patch_tile_dynamic(
        &mut self,
        tile: usize,
        owner: i16,
        flags: u16,
        bombard_range: u8,
    ) {
        if tile >= self.num_tiles {
            return;
        }
        let prev_city = (self.tile_flags[tile] & TILE_CITY_CENTER) != 0;
        let new_city = (flags & TILE_CITY_CENTER) != 0;
        if prev_city != new_city || (new_city && self.owner_civ[tile] != owner) {
            self.zoc_dirty = true;
            self.threat_dirty = true;
        }
        self.owner_civ[tile] = owner;
        self.tile_flags[tile] = flags;
        self.city_bombard_range[tile] = bombard_range;
    }

    /// Incremental patch when a tile's terrain changes (forest chop, fallout, etc.).
    #[inline]
    pub fn patch_tile_terrain(
        &mut self,
        tile: usize,
        terrain_flags: u32,
        base_move_cost: u8,
        terrain_damage: i16,
        tile_height: i8,
        unit_height: i8,
        base_terrain_id: u8,
        hill_terrain_id: u8,
        last_terrain_id: u8,
        feature_terrain_mask: u64,
    ) {
        if tile >= self.num_tiles {
            return;
        }
        self.terrain_flags[tile] = terrain_flags;
        self.base_move_cost[tile] = base_move_cost;
        self.terrain_damage[tile] = terrain_damage;
        self.tile_height[tile] = tile_height;
        self.unit_height[tile] = unit_height;
        self.base_terrain_id[tile] = base_terrain_id;
        self.hill_terrain_id[tile] = hill_terrain_id;
        self.last_terrain_id[tile] = last_terrain_id;
        self.feature_terrain_mask[tile] = feature_terrain_mask;
    }
}
