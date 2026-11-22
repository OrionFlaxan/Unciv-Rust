package com.unciv.nativeengine

import com.unciv.Constants
import com.unciv.UncivGame
import com.unciv.logic.GameInfo
import com.unciv.logic.city.City
import com.unciv.logic.civilization.Civilization
import com.unciv.logic.civilization.diplomacy.DiplomaticStatus
import com.unciv.logic.civilization.diplomacy.RelationshipLevel
import com.unciv.logic.map.HexCoord
import com.unciv.logic.map.MapShape
import com.unciv.logic.map.MapPathing
import com.unciv.logic.map.TileMap
import com.unciv.logic.map.mapunit.MapUnit
import com.unciv.logic.map.mapunit.MapUnitCache
import com.unciv.logic.map.mapunit.movement.PathsToTilesWithinTurn
import com.unciv.logic.map.mapunit.movement.UnitMovement
import com.unciv.logic.map.pathingmap.FixedPointMovement
import com.unciv.logic.map.tile.RoadStatus
import com.unciv.logic.map.tile.Tile
import com.unciv.models.ruleset.Ruleset
import com.unciv.models.ruleset.unique.UniqueType
import com.unciv.utils.DebugUtils
import yairm210.purity.annotations.Readonly
import java.lang.ref.PhantomReference
import java.lang.ref.ReferenceQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * Stateful per-[TileMap] bridge managing a Rust `NativeWorld` handle, incremental O(1) state
 * patches, and Java-8-compatible automatic native memory reclamation via [PhantomReference].
 */
class NativeWorldMirror private constructor(
    val tileMap: TileMap,
    private val handleRef: AtomicLong,
    val numTiles: Int,
    val bitsetWords: Int,
    private val terrainIdMap: HashMap<String, Int>
) {
    /** Returned by [applyUnitVisionDelta] for O(ring) incremental civ vision updates. */
    class VisionDelta(
        val removedVisible: List<Tile>,
        val addedVisible: List<Tile>,
        val newlyExplored: List<Tile>
    )

    @Volatile var topologyDirty: Boolean = true
    @Volatile var dynamicDirty: Boolean = true
    @Volatile var diploDirty: Boolean = true
    @Volatile private var syncedNumCivs: Int = 0
    @Volatile private var syncedBaseNumCivs: Int = 0
    private var syncedCivs: List<Civilization> = emptyList()
    @Volatile private var threatCivIdx: Int = -1
    @Volatile var threatDirty: Boolean = true

    // Cached per-civ scalars to catch direct field mutations (e.g. in tests or mods) in O(1)
    private var cachedCivFlags = IntArray(0)
    private var cachedRoadSpeedFpm = ShortArray(0)
    private var cachedPassableMask = LongArray(0)

    // Cached enemy movement penalty info (Great Wall etc.)
    private var anyCivHasEnemyPenalty = false
    private var enemyPenaltyCivs: List<Civilization> = emptyList()
    private var anyInvisibleMilitaryUnit = false

    val handle: Long get() = handleRef.get()

    fun destroy() {
        val h = handleRef.getAndSet(0L)
        if (h != 0L) {
            UncivRustNative.nativeDestroyWorld(h)
        }
    }

    // =========================================================================
    // Synchronization & Incremental Patches
    // =========================================================================

    @Readonly @Suppress("purity")
    fun ensureTopologySynced() {
        val h = handle
        if (h == 0L || !topologyDirty) return
        syncStaticTopology()
    }

    @Readonly @Suppress("purity")
    private fun isCivRegistered(civ: Civilization): Boolean {
        val idx = civ.civIndex
        return idx >= 0 && idx < syncedCivs.size && syncedCivs[idx] === civ
    }

    @Readonly @Suppress("purity")
    fun ensureSynced(gameInfo: GameInfo, extraCiv: Civilization? = null) {
        val h = handle
        if (h == 0L) return
        if (topologyDirty) {
            syncStaticTopology()
        }
        val numCivs = gameInfo.civilizations.size
        if (dynamicDirty || numCivs != syncedBaseNumCivs || (extraCiv != null && !isCivRegistered(extraCiv))) {
            syncDynamicState(gameInfo, extraCiv)
        } else if (diploDirty) {
            syncDiploAndCivs(gameInfo)
        }
    }

    fun syncStaticTopology() {
        val h = handle
        if (h == 0L) return
        val tiles = tileMap.tileList
        val n = tiles.size
        if (n != numTiles) return

        val posX = ShortArray(n)
        val posY = ShortArray(n)
        val neighbors = IntArray(n * 6) { -1 }
        val riverMask = ByteArray(n)
        val terrainFlags = IntArray(n)
        val baseMoveCost = ByteArray(n)
        val terrainDamage = ShortArray(n)
        val tileHeight = ByteArray(n)
        val unitHeight = ByteArray(n)
        val continent = ShortArray(n)
        val baseTerrainId = ByteArray(n) { UncivRustNative.NO_TERRAIN_ID }
        val hillTerrainId = ByteArray(n) { UncivRustNative.NO_TERRAIN_ID }
        val lastTerrainId = ByteArray(n) { UncivRustNative.NO_TERRAIN_ID }
        val featureTerrainMask = LongArray(n)

        for (i in 0 until n) {
            val t = tiles[i]
            posX[i] = t.position.x.toShort()
            posY[i] = t.position.y.toShort()

            var rmask = 0
            val base = i * 6
            for (d in 0 until 6) {
                val clock = (d + 1) * 2
                val nb = tileMap.getClockPositionNeighborTile(t, clock)
                if (nb != null) {
                    neighbors[base + d] = nb.zeroBasedIndex
                    if (t.isAdjacentToRiver() && nb.isAdjacentToRiver() && t.isConnectedByRiver(nb)) {
                        rmask = rmask or (1 shl d)
                    }
                }
            }
            riverMask[i] = rmask.toByte()

            terrainFlags[i] = computeTerrainFlags(t)
            baseMoveCost[i] = t.lastTerrain.movementCost.coerceIn(0, 127).toByte()
            var dmg = 0
            for (tr in t.allTerrains) dmg += tr.damagePerTurn
            terrainDamage[i] = dmg.coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
            tileHeight[i] = t.tileHeight.coerceIn(-128, 127).toByte()
            unitHeight[i] = t.unitHeight.coerceIn(-128, 127).toByte()
            continent[i] = t.getContinent().coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()

            baseTerrainId[i] = terrainIdByte(t.baseTerrain)
            hillTerrainId[i] = t.getHillTerrain()?.let { terrainIdByte(it.name) } ?: UncivRustNative.NO_TERRAIN_ID
            lastTerrainId[i] = terrainIdByte(t.lastTerrain.name)
            featureTerrainMask[i] = computeFeatureMask(t)
        }

        UncivRustNative.nativeSyncStaticTopology(
            h, posX, posY, neighbors, riverMask, terrainFlags, baseMoveCost,
            terrainDamage, tileHeight, unitHeight, continent,
            baseTerrainId, hillTerrainId, lastTerrainId, featureTerrainMask
        )
        topologyDirty = false
    }

    fun syncDynamicState(gameInfo: GameInfo, extraCiv: Civilization? = null) {
        val h = handle
        if (h == 0L) return
        val tiles = tileMap.tileList
        val n = tiles.size
        if (n != numTiles) return

        val baseCivs = gameInfo.civilizations
        val civs = ArrayList<Civilization>(baseCivs.size + 2)
        for (i in 0 until baseCivs.size) {
            val c = baseCivs[i]
            c.civIndex = i
            civs.add(c)
        }

        fun registerCiv(c: Civilization?): Short {
            if (c == null) return -1
            val idx = c.civIndex
            if (idx >= 0 && idx < civs.size && civs[idx] === c) return idx.toShort()
            val newIdx = civs.size
            c.civIndex = newIdx
            civs.add(c)
            return newIdx.toShort()
        }
        if (extraCiv != null) registerCiv(extraCiv)

        val ownerCiv = ShortArray(n) { -1 }
        val tileFlags = ShortArray(n)
        val milUnitCiv = ShortArray(n) { -1 }
        val milUnitFlags = ByteArray(n)
        val civUnitCiv = ShortArray(n) { -1 }
        val airUnitCiv = ShortArray(n) { -1 }
        val cityBombardRange = ByteArray(n)
        var hasInvisible = false

        for (i in 0 until n) {
            val t = tiles[i]
            ownerCiv[i] = registerCiv(t.getOwner())
            tileFlags[i] = computeTileFlags(t)
            cityBombardRange[i] = if (t.isCityCenter()) {
                t.getCity()!!.getBombardRange().coerceIn(0, 127).toByte()
            } else 0

            val mu = t.militaryUnit
            if (mu != null) {
                milUnitCiv[i] = registerCiv(mu.civ)
                val mf = computeMilUnitFlags(mu)
                milUnitFlags[i] = mf
                if ((mf.toInt() and UncivRustNative.UF_INVISIBLE) != 0) hasInvisible = true
            }
            val cu = t.civilianUnit
            if (cu != null) {
                civUnitCiv[i] = registerCiv(cu.civ)
            }
            if (t.airUnits.isNotEmpty()) {
                airUnitCiv[i] = registerCiv(t.airUnits[0].civ)
            }
        }
        anyInvisibleMilitaryUnit = hasInvisible

        val numCivs = civs.size
        val diploMatrix = buildDiploMatrix(civs)
        val civFlags = IntArray(numCivs)
        val civRoadFpm = ShortArray(numCivs)
        val civRoadF32 = FloatArray(numCivs)
        val civPassMask = LongArray(numCivs)
        val penaltyCivs = ArrayList<Civilization>(2)

        for (i in 0 until numCivs) {
            val c = civs[i]
            civFlags[i] = computeCivFlags(c)
            val spd = c.tech.movementSpeedOnRoads
            civRoadF32[i] = spd
            civRoadFpm[i] = FixedPointMovement.fpmFromMovement(spd).bits.toShort()
            civPassMask[i] = computePassableImpassablesMask(c)
            if (c.hasActiveEnemyMovementPenalty) penaltyCivs.add(c)
        }
        anyCivHasEnemyPenalty = penaltyCivs.isNotEmpty()
        enemyPenaltyCivs = penaltyCivs

        cachedCivFlags = civFlags
        cachedRoadSpeedFpm = civRoadFpm
        cachedPassableMask = civPassMask

        UncivRustNative.nativeSyncDynamicState(
            h, numCivs, ownerCiv, tileFlags, milUnitCiv, milUnitFlags,
            civUnitCiv, airUnitCiv, cityBombardRange,
            diploMatrix, civFlags, civRoadFpm, civRoadF32, civPassMask
        )

        // Sync explored & visible bitsets for every civ
        for (i in 0 until numCivs) {
            val c = civs[i]
            c.ensureExploredBitsInitialized(tileMap)
            UncivRustNative.nativeSyncExploredBits(h, i, c.exploredTileBits)
            if (c.viewableTiles.isNotEmpty()) {
                val visBits = LongArray(bitsetWords)
                for (vt in c.viewableTiles) {
                    val idx = vt.zeroBasedIndex
                    visBits[idx ushr 6] = visBits[idx ushr 6] or (1L shl (idx and 63))
                }
                UncivRustNative.nativeSyncVisibleBits(h, i, visBits)
            }
        }

        syncedBaseNumCivs = baseCivs.size
        syncedNumCivs = numCivs
        syncedCivs = civs
        dynamicDirty = false
        diploDirty = false
        threatDirty = true
    }

    private fun syncDiploAndCivs(gameInfo: GameInfo) {
        val h = handle
        if (h == 0L) return
        if (gameInfo.civilizations.size != syncedBaseNumCivs) {
            syncDynamicState(gameInfo)
            return
        }
        val civs = syncedCivs
        val numCivs = civs.size
        val penaltyCivs = ArrayList<Civilization>(2)
        for (a in 0 until numCivs) {
            val civA = civs[a]
            val cf = computeCivFlags(civA)
            val spd = civA.tech.movementSpeedOnRoads
            val fpm = FixedPointMovement.fpmFromMovement(spd).bits.toShort()
            val pm = computePassableImpassablesMask(civA)
            if (cf != cachedCivFlags[a] || fpm != cachedRoadSpeedFpm[a] || pm != cachedPassableMask[a]) {
                cachedCivFlags[a] = cf
                cachedRoadSpeedFpm[a] = fpm
                cachedPassableMask[a] = pm
                UncivRustNative.nativePatchCivFlags(h, a, cf, fpm, spd, pm)
            }
            if (civA.hasActiveEnemyMovementPenalty) penaltyCivs.add(civA)
            for (b in 0 until numCivs) {
                UncivRustNative.nativePatchDiplo(h, a, b, computeDiploByte(civA, civs[b]))
            }
        }
        anyCivHasEnemyPenalty = penaltyCivs.isNotEmpty()
        enemyPenaltyCivs = penaltyCivs
        diploDirty = false
        threatDirty = true
    }

    /** O(1) scalar check catching direct mutations on `civ.tech` / `civ.passableImpassables` (e.g. in tests). */
    @Readonly @Suppress("purity")
    private fun ensureCivFlagsCurrent(civ: Civilization) {
        val h = handle
        val idx = civ.civIndex
        if (h == 0L || idx < 0 || idx >= cachedCivFlags.size) return
        val cf = computeCivFlags(civ)
        val spd = civ.tech.movementSpeedOnRoads
        val fpm = FixedPointMovement.fpmFromMovement(spd).bits.toShort()
        val pm = computePassableImpassablesMask(civ)
        if (cf != cachedCivFlags[idx] || fpm != cachedRoadSpeedFpm[idx] || pm != cachedPassableMask[idx]) {
            cachedCivFlags[idx] = cf
            cachedRoadSpeedFpm[idx] = fpm
            cachedPassableMask[idx] = pm
            UncivRustNative.nativePatchCivFlags(h, idx, cf, fpm, spd, pm)
        }
    }

    fun onTileUnitsChanged(tile: Tile) {
        val h = handle
        if (h == 0L || dynamicDirty) return
        val idx = tile.zeroBasedIndex
        if (idx < 0 || idx >= numTiles) return
        val mu = tile.militaryUnit
        val cu = tile.civilianUnit
        val au = if (tile.airUnits.isNotEmpty()) tile.airUnits[0] else null
        if ((mu != null && !isCivRegistered(mu.civ))
            || (cu != null && !isCivRegistered(cu.civ))
            || (au != null && !isCivRegistered(au.civ))
        ) {
            dynamicDirty = true
            return
        }
        val milCiv = (mu?.civ?.civIndex ?: -1).toShort()
        val milFlags = if (mu != null) computeMilUnitFlags(mu) else 0
        if ((milFlags.toInt() and UncivRustNative.UF_INVISIBLE) != 0) anyInvisibleMilitaryUnit = true
        val civCiv = (cu?.civ?.civIndex ?: -1).toShort()
        val airCiv = (au?.civ?.civIndex ?: -1).toShort()
        UncivRustNative.nativePatchTileUnits(h, idx, milCiv, milFlags, civCiv, airCiv)
        threatDirty = true
    }

    fun onTileDynamicChanged(tile: Tile) {
        val h = handle
        if (h == 0L || dynamicDirty) return
        val idx = tile.zeroBasedIndex
        if (idx < 0 || idx >= numTiles) return
        val ownerObj = tile.getOwner()
        if (ownerObj != null && !isCivRegistered(ownerObj)) {
            dynamicDirty = true
            return
        }
        val owner = (ownerObj?.civIndex ?: -1).toShort()
        val flags = computeTileFlags(tile)
        val br = if (tile.isCityCenter()) tile.getCity()!!.getBombardRange().coerceIn(0, 127).toByte() else 0
        UncivRustNative.nativePatchTileDynamic(h, idx, owner, flags, br)
        threatDirty = true
    }

    fun onTileTerrainChanged(tile: Tile) {
        val h = handle
        if (h == 0L || topologyDirty) return
        val idx = tile.zeroBasedIndex
        if (idx < 0 || idx >= numTiles) return
        var dmg = 0
        for (tr in tile.allTerrains) dmg += tr.damagePerTurn
        UncivRustNative.nativePatchTileTerrain(
            h, idx,
            computeTerrainFlags(tile),
            tile.lastTerrain.movementCost.coerceIn(0, 127).toByte(),
            dmg.coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort(),
            tile.tileHeight.coerceIn(-128, 127).toByte(),
            tile.unitHeight.coerceIn(-128, 127).toByte(),
            terrainIdByte(tile.baseTerrain),
            tile.getHillTerrain()?.let { terrainIdByte(it.name) } ?: UncivRustNative.NO_TERRAIN_ID,
            terrainIdByte(tile.lastTerrain.name),
            computeFeatureMask(tile)
        )
    }

    fun onTileExploredChanged(civ: Civilization, tile: Tile, explored: Boolean) {
        val h = handle
        val cIdx = civ.civIndex
        if (h == 0L || dynamicDirty || cIdx < 0 || cIdx >= syncedNumCivs) return
        UncivRustNative.nativePatchTileExplored(h, cIdx, tile.zeroBasedIndex, explored)
    }

    fun onDiplomacyChanged(civA: Civilization, civB: Civilization) {
        val h = handle
        if (h == 0L || dynamicDirty) return
        val a = civA.civIndex
        val b = civB.civIndex
        if (a < 0 || b < 0 || a >= syncedNumCivs || b >= syncedNumCivs) {
            diploDirty = true
            return
        }
        UncivRustNative.nativePatchDiplo(h, a, b, computeDiploByte(civA, civB))
        UncivRustNative.nativePatchDiplo(h, b, a, computeDiploByte(civB, civA))
        threatDirty = true
    }

    fun markVisionDirty(civ: Civilization? = null) {
        val h = handle
        if (h == 0L) return
        val idx = civ?.civIndex ?: -1
        if (idx >= syncedNumCivs) return
        UncivRustNative.nativeMarkVisionDirty(h, idx)
        if (civ == null || idx == threatCivIdx) threatDirty = true
    }

    // =========================================================================
    // Unit Eligibility & Profile Encoding
    // =========================================================================

    @Readonly @Suppress("purity")
    fun canUseNativePathing(unit: MapUnit, includeEscortUnit: Boolean = true): Boolean {
        if (!NativeEngine.isAvailable || handle == 0L) return false
        if (unit.baseUnit.isAirUnit() || unit.isPreparingParadrop()) return false
        if (includeEscortUnit && unit.isEscorting()) return false
        val cache = unit.cache
        if (cache.cannotMove) return false
        if (!cache.noFilteredDoubleMovementUniques) return false
        if (cache.hasConditionalDoubleMovement) return false
        // Reject if any double-movement terrain is outside the 64-bit mask (huge mod)
        if (cache.doubleMovementInTerrain.isNotEmpty()) {
            for (name in cache.doubleMovementInTerrain.keys) {
                if ((terrainIdMap[name] ?: 999) >= 64) return false
            }
        }
        // Loaded air-unit carriers require CarrierSlotMatcher
        if (cache.isCarrierUnique && unit.hasTile()) {
            val airUnits = unit.getTile().airUnits
            for (i in 0 until airUnits.size) {
                val a = airUnits[i]
                if (a.isTransported && a.owner == unit.owner) return false
            }
        }
        // If 2+ warring civs have differing enemy movement penalties for this unit, fall back
        if (anyCivHasEnemyPenalty && resolveEnemyPenaltyFpm(unit) < 0) return false
        return true
    }

    /** Returns uniform enemy movement penalty in FPM (>= 0), or -1 if warring civs disagree. */
    @Readonly
    private fun resolveEnemyPenaltyFpm(unit: MapUnit): Int {
        if (!anyCivHasEnemyPenalty) return 0
        val civ = unit.civ
        var foundPenalty = -1
        for (i in 0 until enemyPenaltyCivs.size) {
            val enemy = enemyPenaltyCivs[i]
            if (enemy == civ || !civ.isAtWarWith(enemy)) continue
            val uniques = enemy.enemyMovementPenaltyUniques
            var sum = 0
            if (uniques != null) {
                for (u in uniques) {
                    if (u.type == UniqueType.EnemyUnitsSpendExtraMovement && unit.matchesFilter(u.params[0])) {
                        sum += u.params[1].toInt()
                    }
                }
            }
            val fpm = sum * 30
            if (foundPenalty == -1) {
                foundPenalty = fpm
            } else if (foundPenalty != fpm) {
                return -1 // Divergent penalties across multiple warring civs -> Kotlin fallback
            }
        }
        return if (foundPenalty < 0) 0 else foundPenalty
    }

    @Readonly
    private fun encodeProfileFlags(
        unit: MapUnit,
        considerZoneOfControl: Boolean,
        avoidDamagingTerrain: Boolean,
        useAStarSetting: Boolean
    ): Int {
        val c = unit.cache
        val civ = unit.civ
        var f = 0
        if (unit.baseUnit.isLandUnit) f = f or UncivRustNative.UP_IS_LAND
        if (unit.baseUnit.isWaterUnit) f = f or UncivRustNative.UP_IS_WATER
        if (unit.isCivilian()) f = f or UncivRustNative.UP_IS_CIVILIAN
        if (c.cannotMove) f = f or UncivRustNative.UP_CANNOT_MOVE
        if (c.allTilesCosts1) f = f or UncivRustNative.UP_ALL_TILES_COST_1
        if (c.ignoresTerrainCost) f = f or UncivRustNative.UP_IGNORES_TERRAIN_COST
        if (c.ignoresZoneOfControl) f = f or UncivRustNative.UP_IGNORES_ZOC
        if (c.roughTerrainPenalty) f = f or UncivRustNative.UP_ROUGH_TERRAIN_PENALTY
        if (c.canMoveOnWater) f = f or UncivRustNative.UP_CAN_MOVE_ON_WATER
        if (c.canPassThroughImpassableTiles) f = f or UncivRustNative.UP_CAN_PASS_IMPASSABLE
        if (c.canEnterIceTiles) f = f or UncivRustNative.UP_CAN_ENTER_ICE
        if (c.cannotEmbark) f = f or UncivRustNative.UP_CANNOT_EMBARK
        if (c.cannotEnterOceanTiles) f = f or UncivRustNative.UP_CANNOT_ENTER_OCEAN
        if (civ.tech.specificUnitsCanEnterOcean &&
            civ.getMatchingUniques(UniqueType.UnitsMayEnterOcean).any { unit.matchesFilter(it.params[0]) }
        ) {
            f = f or UncivRustNative.UP_SPECIFIC_ALLOW_OCEAN
        }
        if (c.canEnterForeignTerrain) f = f or UncivRustNative.UP_CAN_ENTER_FOREIGN
        if (c.canEnterCityStates) f = f or UncivRustNative.UP_CAN_ENTER_CITY_STATES
        if (unit.isCivilian() && unit.isAutomated()) f = f or UncivRustNative.UP_AVOID_ENEMY_TERRITORY
        if (c.noTerrainMovementUniques) f = f or UncivRustNative.UP_NO_TERRAIN_MOVEMENT_UNIQUES
        if (c.noBaseTerrainOrHillDoubleMovementUniques) f = f or UncivRustNative.UP_NO_BASE_OR_HILL_DM
        if (considerZoneOfControl) f = f or UncivRustNative.UP_CONSIDER_ZOC
        if (avoidDamagingTerrain) f = f or UncivRustNative.UP_AVOID_DAMAGING_TERRAIN
        if (!useAStarSetting) {
            f = f or UncivRustNative.UP_ALLOW_UNPASSABLE_DEST
            f = f or UncivRustNative.UP_ASSUME_UNEXPLORED_PASSABLE
        }
        return f
    }

    @Readonly
    private fun encodeDoubleMovementMasks(unit: MapUnit): Triple<Long, Long, Long> {
        val dm = unit.cache.doubleMovementInTerrain
        if (dm.isEmpty()) return Triple(0L, 0L, 0L)
        var feat = 0L
        var base = 0L
        var hill = 0L
        for ((name, info) in dm) {
            val id = terrainIdMap[name] ?: continue
            if (id < 0 || id >= 64) continue
            val bit = 1L shl id
            when (info.terrainTarget) {
                MapUnitCache.DoubleMovementTerrainTarget.Feature -> feat = feat or bit
                MapUnitCache.DoubleMovementTerrainTarget.Base -> base = base or bit
                MapUnitCache.DoubleMovementTerrainTarget.Hill -> hill = hill or bit
                MapUnitCache.DoubleMovementTerrainTarget.Filter -> {}
            }
        }
        return Triple(feat, base, hill)
    }

    // =========================================================================
    // 1. Native Pathfinding & Movement Queries
    // =========================================================================

    @Readonly @Suppress("purity")
    fun computeDistanceWithinTurn(
        unit: MapUnit,
        position: HexCoord,
        usableMovement: Float,
        considerZoneOfControl: Boolean,
        includeUnpassableBoundary: Boolean
    ): PathsToTilesWithinTurn {
        val gameInfo = unit.civ.gameInfo
        ensureSynced(gameInfo)
        ensureCivFlagsCurrent(unit.civ)

        val useAStar = UncivGame.Current.settings.useAStarPathfinding
        val startTile = if (position == unit.currentTile.position) unit.currentTile else tileMap[position]
        val flags = encodeProfileFlags(unit, considerZoneOfControl, false, useAStar)
        val (dmFeat, dmBase, dmHill) = encodeDoubleMovementMasks(unit)
        val curFpm = FixedPointMovement.fpmFromMovement(usableMovement).bits
        val fullFpm = FixedPointMovement.fpmFromMovement(unit.getMaxMovement()).bits
        val embFpm = unit.cache.costToEmbark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val disFpm = unit.cache.costToDisembark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val penFpm = resolveEnemyPenaltyFpm(unit)

        val triples = UncivRustNative.nativeComputeDistanceWithinTurn(
            handle, unit.civ.civIndex, flags,
            usableMovement, curFpm, fullFpm,
            embFpm, disFpm, penFpm,
            dmFeat, dmBase, dmHill,
            startTile.zeroBasedIndex, includeUnpassableBoundary
        )

        val tileList = tileMap.tileList
        val result = PathsToTilesWithinTurn()
        var i = 0
        val len = triples.size
        while (i + 2 < len) {
            val t = tileList[triples[i]]
            val p = tileList[triples[i + 1]]
            val dist = java.lang.Float.intBitsToFloat(triples[i + 2])
            result[t] = UnitMovement.ParentTileAndTotalMovement(t, p, dist)
            i += 3
        }
        return result
    }

    @Readonly @Suppress("purity")
    fun findShortestPath(
        unit: MapUnit,
        destination: Tile,
        avoidDamagingTerrain: Boolean,
        maxTurns: Int = 62
    ): List<Tile> {
        val gameInfo = unit.civ.gameInfo
        ensureSynced(gameInfo)
        ensureCivFlagsCurrent(unit.civ)

        val useAStar = UncivGame.Current.settings.useAStarPathfinding
        val flags = encodeProfileFlags(unit, true, avoidDamagingTerrain, useAStar)
        val (dmFeat, dmBase, dmHill) = encodeDoubleMovementMasks(unit)
        val curMove = unit.currentMovement
        val curFpm = FixedPointMovement.fpmFromMovement(curMove).bits
        val fullFpm = FixedPointMovement.fpmFromMovement(unit.getMaxMovement()).bits
        val embFpm = unit.cache.costToEmbark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val disFpm = unit.cache.costToDisembark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val penFpm = resolveEnemyPenaltyFpm(unit)

        val indices = UncivRustNative.nativeFindShortestPath(
            handle, unit.civ.civIndex, flags,
            curMove, curFpm, fullFpm,
            embFpm, disFpm, penFpm,
            dmFeat, dmBase, dmHill,
            unit.currentTile.zeroBasedIndex, destination.zeroBasedIndex, maxTurns
        )
        if (indices.isEmpty()) return emptyList()
        val tileList = tileMap.tileList
        val out = ArrayList<Tile>(indices.size)
        for (idx in indices) out.add(tileList[idx])
        return out
    }

    /**
     * Single-expansion batch reachability: given priority-ordered [candidates], returns
     * `Pair(firstReachableIndex, shortestPathToIt)` or `Pair(-1, emptyList())`.
     */
    @Readonly @Suppress("purity")
    fun batchFirstReachable(
        unit: MapUnit,
        candidates: List<Tile>,
        avoidDamagingTerrain: Boolean = false,
        maxTurns: Int = 62
    ): Pair<Int, List<Tile>> {
        if (candidates.isEmpty()) return -1 to emptyList()
        val gameInfo = unit.civ.gameInfo
        ensureSynced(gameInfo)
        ensureCivFlagsCurrent(unit.civ)

        val useAStar = UncivGame.Current.settings.useAStarPathfinding
        val flags = encodeProfileFlags(unit, true, avoidDamagingTerrain, useAStar)
        val (dmFeat, dmBase, dmHill) = encodeDoubleMovementMasks(unit)
        val curMove = unit.currentMovement
        val curFpm = FixedPointMovement.fpmFromMovement(curMove).bits
        val fullFpm = FixedPointMovement.fpmFromMovement(unit.getMaxMovement()).bits
        val embFpm = unit.cache.costToEmbark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val disFpm = unit.cache.costToDisembark?.let { FixedPointMovement.fpmFromMovement(it).bits } ?: -1
        val penFpm = resolveEnemyPenaltyFpm(unit)

        val candIdx = IntArray(candidates.size) { candidates[it].zeroBasedIndex }
        val packed = UncivRustNative.nativeBatchFirstReachable(
            handle, unit.civ.civIndex, flags,
            curMove, curFpm, fullFpm,
            embFpm, disFpm, penFpm,
            dmFeat, dmBase, dmHill,
            unit.currentTile.zeroBasedIndex, candIdx, maxTurns
        )
        if (packed.isEmpty() || packed[0] < 0) return -1 to emptyList()
        val bestIdx = packed[0]
        val tileList = tileMap.tileList
        val path = ArrayList<Tile>(packed.size - 1)
        for (i in 1 until packed.size) path.add(tileList[packed[i]])
        return bestIdx to path
    }

    // =========================================================================
    // 2. Native O(R^2) Line-of-Sight & Incremental Civ Vision
    // =========================================================================

    @Readonly @Suppress("purity")
    fun computeViewableTiles(origin: Tile, sightDistance: Int, forAttack: Boolean): List<Tile> {
        ensureTopologySynced()
        val indices = UncivRustNative.nativeComputeViewableTiles(
            handle, origin.zeroBasedIndex, sightDistance, forAttack
        )
        val tileList = tileMap.tileList
        val out = ArrayList<Tile>(indices.size)
        for (idx in indices) out.add(tileList[idx])
        return out
    }

    fun rebuildCivVision(
        civ: Civilization,
        staticTiles: Collection<Tile>,
        unitTilesFlat: IntArray
    ): List<Tile> {
        val h = handle
        if (h == 0L) return emptyList()
        ensureSynced(civ.gameInfo)
        val staticArr = IntArray(staticTiles.size)
        var k = 0
        for (t in staticTiles) staticArr[k++] = t.zeroBasedIndex
        val newlyIdx = UncivRustNative.nativeRebuildCivVision(h, civ.civIndex, staticArr, unitTilesFlat)
        if (civ.civIndex == threatCivIdx) threatDirty = true
        if (newlyIdx.isEmpty()) return emptyList()
        val tileList = tileMap.tileList
        val out = ArrayList<Tile>(newlyIdx.size)
        for (idx in newlyIdx) out.add(tileList[idx])
        return out
    }

    fun applyUnitVisionDelta(
        civ: Civilization,
        oldTiles: Collection<Tile>,
        newTiles: Collection<Tile>
    ): VisionDelta? {
        val h = handle
        if (h == 0L || dynamicDirty || civ.civIndex < 0 || civ.civIndex >= syncedNumCivs) return null
        val oldArr = IntArray(oldTiles.size)
        var i = 0
        for (t in oldTiles) oldArr[i++] = t.zeroBasedIndex
        val newArr = IntArray(newTiles.size)
        i = 0
        for (t in newTiles) newArr[i++] = t.zeroBasedIndex

        val packed = UncivRustNative.nativeApplyUnitVisionDelta(h, civ.civIndex, oldArr, newArr)
        if (packed.size < 3) return null // Not synced in Rust yet
        val remLen = packed[0]
        val addLen = packed[1]
        val expLen = packed[2]
        if ((remLen > 0 || addLen > 0) && civ.civIndex == threatCivIdx) threatDirty = true
        val tileList = tileMap.tileList
        var off = 3
        val removed = if (remLen == 0) emptyList() else ArrayList<Tile>(remLen).also { list ->
            for (j in 0 until remLen) list.add(tileList[packed[off++]])
        }
        val added = if (addLen == 0) emptyList() else ArrayList<Tile>(addLen).also { list ->
            for (j in 0 until addLen) list.add(tileList[packed[off++]])
        }
        val explored = if (expLen == 0) emptyList() else ArrayList<Tile>(expLen).also { list ->
            for (j in 0 until expLen) list.add(tileList[packed[off++]])
        }
        return VisionDelta(removed, added, explored)
    }

    // =========================================================================
    // 3. Multi-Source Threat Field & Single-Pass Attack Pair Discovery
    // =========================================================================

    @Readonly @Suppress("purity")
    fun ensureThreatField(civ: Civilization) {
        val h = handle
        if (h == 0L) return
        ensureSynced(civ.gameInfo)
        val cIdx = civ.civIndex
        if (!threatDirty && threatCivIdx == cIdx) return

        // Only check invisible units if any exist on the map (almost always empty!)
        val extraDetected = if (anyInvisibleMilitaryUnit) {
            val list = ArrayList<Int>(4)
            for (tile in civ.viewableTiles) {
                val mu = tile.militaryUnit ?: continue
                if (mu.civ != civ && civ.isAtWarWith(mu.civ) && mu.hasUnique(UniqueType.Invisible) && mu.isVisibleTo(civ)) {
                    list.add(tile.zeroBasedIndex)
                }
            }
            IntArray(list.size) { list[it] }
        } else {
            EMPTY_INT_ARRAY
        }

        UncivRustNative.nativeBuildThreatField(h, cIdx, extraDetected)
        threatCivIdx = cIdx
        threatDirty = false
    }

    @Readonly @Suppress("purity")
    fun getDistanceToClosestEnemy(civ: Civilization, tile: Tile, maxDist: Int, takeLargerValues: Boolean): Int {
        ensureThreatField(civ)
        return UncivRustNative.nativeGetDistanceToClosestEnemy(
            handle, tile.zeroBasedIndex, maxDist, takeLargerValues
        )
    }

    @Readonly @Suppress("purity")
    fun getEnemyTilesInDistance(civ: Civilization, tile: Tile, maxDist: Int): List<Tile> {
        ensureThreatField(civ)
        val indices = UncivRustNative.nativeGetEnemyTilesInDistance(handle, tile.zeroBasedIndex, maxDist)
        if (indices.isEmpty()) return emptyList()
        val tileList = tileMap.tileList
        val out = ArrayList<Tile>(indices.size)
        for (idx in indices) out.add(tileList[idx])
        return out
    }

    @Readonly @Suppress("purity")
    fun findAttackablePairs(
        civ: Civilization,
        fromTiles: IntArray,
        fromMoveLeftBits: IntArray,
        attackRange: Int,
        isMelee: Boolean,
        indirectFire: Boolean,
        candidateEnemies: IntArray
    ): IntArray {
        ensureTopologySynced()
        return UncivRustNative.nativeFindAttackablePairs(
            handle, civ.civIndex, fromTiles, fromMoveLeftBits,
            attackRange, isMelee, indirectFire, candidateEnemies
        )
    }

    // =========================================================================
    // 4. Connectivity: Capital Connections, Roads & Attack Paths
    // =========================================================================

    @Readonly @Suppress("purity")
    fun findCapitalConnections(
        civ: Civilization,
        openBordersCities: List<City>,
        capitalSlot: Int,
        roadResearched: Boolean,
        railroadResearched: Boolean
    ): IntArray {
        ensureSynced(civ.gameInfo)
        val count = openBordersCities.size
        val cityTiles = IntArray(count)
        val cityFlags = IntArray(count)
        for (i in 0 until count) {
            val c = openBordersCities[i]
            cityTiles[i] = c.getCenterTile().zeroBasedIndex
            var f = 0
            if (c.civ == civ) {
                f = f or UncivRustNative.CITY_IS_OWN
                if (c.containsBuildingUnique(UniqueType.ConnectTradeRoutes)) {
                    f = f or UncivRustNative.CITY_HAS_HARBOR
                    if (c.isBlockaded()) f = f or UncivRustNative.CITY_IS_BLOCKADED
                }
            }
            cityFlags[i] = f
        }
        return UncivRustNative.nativeFindCapitalConnections(
            handle, civ.civIndex, cityTiles, cityFlags,
            capitalSlot, roadResearched, railroadResearched
        )
    }

    @Readonly @Suppress("purity")
    fun findRoadPath(
        civ: Civilization,
        startTile: Tile,
        targets: Collection<Tile>,
        maxHalfCost: Int = Int.MAX_VALUE / 2
    ): List<Tile>? {
        ensureSynced(civ.gameInfo)
        val targetArr = IntArray(targets.size)
        var ti = 0
        for (t in targets) targetArr[ti++] = t.zeroBasedIndex

        // Build validRoadBits using fast explored-bitset filter (skips unexplored & water immediately)
        val tiles = tileMap.tileList
        val validBits = LongArray(bitsetWords)
        val exploredBits = civ.exploredTileBits
        val hasExploredBits = exploredBits.size >= bitsetWords
        val spectator = DebugUtils.VISIBLE_MAP || civ.isSpectator
        for (i in 0 until numTiles) {
            if (!spectator && hasExploredBits && ((exploredBits[i ushr 6] ushr (i and 63)) and 1L) == 0L) {
                continue
            }
            val tile = tiles[i]
            if (tile.isWater || tile.isImpassible()) continue
            if (MapPathing.isValidRoadPathTile(civ, tile)) {
                validBits[i ushr 6] = validBits[i ushr 6] or (1L shl (i and 63))
            }
        }
        val pathIdx = UncivRustNative.nativeFindRoadPath(
            handle, civ.civIndex, startTile.zeroBasedIndex, targetArr, validBits, maxHalfCost
        )
        if (pathIdx.isEmpty()) return null
        val out = ArrayList<Tile>(pathIdx.size)
        for (idx in pathIdx) out.add(tiles[idx])
        return out
    }

    @Readonly @Suppress("purity")
    fun evalAttackPaths(
        civ: Civilization,
        targetCiv: Civilization,
        targetCitiesWithOurCity: List<Pair<City, City>>,
        capitalTile: Tile,
        allEnemyCities: List<City>
    ): Float {
        ensureSynced(civ.gameInfo)
        val pairCount = targetCitiesWithOurCity.size
        val ourTiles = IntArray(pairCount) { targetCitiesWithOurCity[it].first.getCenterTile().zeroBasedIndex }
        val enemyTiles = IntArray(pairCount) { targetCitiesWithOurCity[it].second.getCenterTile().zeroBasedIndex }
        val allEnemies = IntArray(allEnemyCities.size) { allEnemyCities[it].getCenterTile().zeroBasedIndex }
        return UncivRustNative.nativeEvalAttackPaths(
            handle, civ.civIndex, targetCiv.civIndex,
            ourTiles, enemyTiles, capitalTile.zeroBasedIndex, allEnemies
        )
    }

    // =========================================================================
    // Encoding Helpers
    // =========================================================================

    private fun terrainIdByte(name: String): Byte {
        val id = terrainIdMap[name] ?: return UncivRustNative.NO_TERRAIN_ID
        return if (id in 0..63) id.toByte() else UncivRustNative.NO_TERRAIN_ID
    }

    private fun computeTerrainFlags(t: Tile): Int {
        var f = 0
        if (t.isLand) f = f or UncivRustNative.TF_LAND
        if (t.isWater) f = f or UncivRustNative.TF_WATER
        if (t.isOcean) f = f or UncivRustNative.TF_OCEAN
        if (t.isImpassible()) f = f or UncivRustNative.TF_IMPASSABLE
        if (t.isHill()) f = f or UncivRustNative.TF_HILL
        if (t.isRoughTerrain()) f = f or UncivRustNative.TF_ROUGH
        if (t.terrainFeatureObjects.any { it.isIce }) f = f or UncivRustNative.TF_ICE
        if (t.terrainFeatures.contains(Constants.forest) || t.terrainFeatures.contains(Constants.jungle)) {
            f = f or UncivRustNative.TF_FOREST_OR_JUNGLE
        }
        if (t.isAdjacentToRiver()) f = f or UncivRustNative.TF_ADJ_RIVER
        if (t.isNaturalWonder()) f = f or UncivRustNative.TF_NATURAL_WONDER
        return f
    }

    private fun computeFeatureMask(t: Tile): Long {
        var mask = 0L
        val feats = t.terrainFeatures
        for (i in 0 until feats.size) {
            val id = terrainIdMap[feats[i]] ?: continue
            if (id in 0..63) mask = mask or (1L shl id)
        }
        return mask
    }

    private fun computeTileFlags(t: Tile): Short {
        var f = when (t.roadStatus) {
            RoadStatus.None -> UncivRustNative.TILE_ROAD_NONE
            RoadStatus.Road -> UncivRustNative.TILE_ROAD_ROAD
            RoadStatus.Railroad -> UncivRustNative.TILE_ROAD_RAILROAD
        }
        if (t.roadIsPillaged) f = f or UncivRustNative.TILE_ROAD_PILLAGED
        if (t.isCityCenter()) {
            f = f or UncivRustNative.TILE_CITY_CENTER
            val city = t.getCity()
            if (city != null && city.hasJustBeenConquered) {
                f = f or UncivRustNative.TILE_JUST_CONQUERED
            }
        }
        if (t.isBarbarianEncampment()) f = f or UncivRustNative.TILE_ENCAMPMENT
        return f.toShort()
    }

    private fun computeMilUnitFlags(u: MapUnit): Byte {
        var f = 0
        if (u.baseUnit.isWaterUnit) f = f or UncivRustNative.UF_WATER
        if (u.hasTile() && u.isEmbarked()) f = f or UncivRustNative.UF_EMBARKED
        if (u.baseUnit.isRanged()) f = f or UncivRustNative.UF_RANGED
        if (u.hasUnique(UniqueType.Invisible)) f = f or UncivRustNative.UF_INVISIBLE
        val range = u.getRange().coerceIn(0, 15)
        f = f or (range shl UncivRustNative.UF_RANGE_SHIFT)
        return f.toByte()
    }

    private fun computeDiploByte(civA: Civilization, civB: Civilization): Byte {
        if (civA == civB) {
            return (UncivRustNative.DIPLO_CAN_PASS or
                UncivRustNative.DIPLO_OPEN_BORDERS or
                UncivRustNative.DIPLO_FRIENDLY_TERRITORY or
                UncivRustNative.DIPLO_KNOWS or
                (UncivRustNative.DIPLO_REL_FAVORABLE shl UncivRustNative.DIPLO_REL_SHIFT)).toByte()
        }
        var b = 0
        if (civA.isAtWarWith(civB)) b = b or UncivRustNative.DIPLO_AT_WAR
        if (civA.diplomacyFunctions.canPassThroughTiles(civB)) b = b or UncivRustNative.DIPLO_CAN_PASS

        val dmAB = civA.getDiplomacyManager(civB)
        if (!civA.isBarbarian && !civB.isBarbarian && dmAB != null) {
            if ((civB.isCityState && dmAB.diplomaticStatus != DiplomaticStatus.War) || dmAB.hasOpenBorders) {
                b = b or UncivRustNative.DIPLO_OPEN_BORDERS
            }
        }
        if (civA.knows(civB)) {
            b = b or UncivRustNative.DIPLO_KNOWS
            val dmBA = civB.getDiplomacyManager(civA)
            if (dmBA != null && dmBA.isConsideredFriendlyTerritory()) {
                b = b or UncivRustNative.DIPLO_FRIENDLY_TERRITORY
            }
            val relOrd = dmBA?.relationshipIgnoreAfraid()?.ordinal ?: RelationshipLevel.Favorable.ordinal
            b = b or ((relOrd and 0x7) shl UncivRustNative.DIPLO_REL_SHIFT)
        } else {
            b = b or (RelationshipLevel.Favorable.ordinal shl UncivRustNative.DIPLO_REL_SHIFT)
        }
        return b.toByte()
    }

    private fun buildDiploMatrix(civs: List<Civilization>): ByteArray {
        val nc = civs.size
        val mat = ByteArray(nc * nc)
        for (a in 0 until nc) {
            val civA = civs[a]
            val row = a * nc
            for (b in 0 until nc) {
                mat[row + b] = computeDiploByte(civA, civs[b])
            }
        }
        return mat
    }

    private fun computeCivFlags(c: Civilization): Int {
        var f = 0
        if (c.tech.roadsConnectAcrossRivers) f = f or UncivRustNative.CF_ROADS_ACROSS_RIVERS
        if (c.nation.forestsAndJunglesAreRoads) f = f or UncivRustNative.CF_FOREST_AS_ROAD
        if (c.nation.ignoreHillMovementCost) f = f or UncivRustNative.CF_IGNORE_HILL
        if (c.tech.unitsCanEmbark) f = f or UncivRustNative.CF_CAN_EMBARK
        if (c.tech.embarkedUnitsCanEnterOcean) f = f or UncivRustNative.CF_EMBARK_OCEAN
        if (c.tech.allUnitsCanEnterOcean) f = f or UncivRustNative.CF_ALL_OCEAN
        if (c.passThroughImpassableUnlocked) f = f or UncivRustNative.CF_PASS_IMPASSABLE
        if (c.hasActiveEnemyMovementPenalty) f = f or UncivRustNative.CF_HAS_ENEMY_PENALTY
        if (c.isCityState) f = f or UncivRustNative.CF_IS_CITY_STATE
        if (c.isBarbarian) f = f or UncivRustNative.CF_IS_BARBARIAN
        if (DebugUtils.VISIBLE_MAP || c.isSpectator) f = f or UncivRustNative.CF_SPECTATOR_VISION
        return f
    }

    private fun computePassableImpassablesMask(c: Civilization): Long {
        if (!c.passThroughImpassableUnlocked || c.passableImpassables.isEmpty()) return 0L
        var mask = 0L
        for (name in c.passableImpassables) {
            val id = terrainIdMap[name] ?: continue
            if (id in 0..63) mask = mask or (1L shl id)
        }
        return mask
    }

    companion object {
        private val EMPTY_INT_ARRAY = IntArray(0)

        // Java 8 compatible automatic native handle cleanup via PhantomReference + ReferenceQueue
        private val refQueue = ReferenceQueue<NativeWorldMirror>()
        private val liveHandles = ConcurrentHashMap<PhantomReference<NativeWorldMirror>, AtomicLong>()

        private fun drainDeadHandles() {
            while (true) {
                val ref = refQueue.poll() ?: break
                val hRef = liveHandles.remove(ref) ?: continue
                val h = hRef.getAndSet(0L)
                if (h != 0L) {
                    UncivRustNative.nativeDestroyWorld(h)
                }
            }
        }

        /** Creates a new [NativeWorldMirror] for [tileMap], or returns `null` if native engine is unavailable. */
        fun createOrNull(tileMap: TileMap, ruleset: Ruleset): NativeWorldMirror? {
            if (!NativeEngine.isAvailable) return null
            drainDeadHandles()
            val numTiles = tileMap.tileList.size
            if (numTiles <= 0) return null
            val numCivs = if (tileMap.hasGameInfo()) tileMap.gameInfo.civilizations.size else 0
            val params = tileMap.mapParameters
            val wrapRadius = if (params.shape == MapShape.rectangular) {
                params.mapSize.width / 2
            } else {
                params.mapSize.radius
            }
            val h = UncivRustNative.nativeCreateWorld(
                numTiles, numCivs, params.worldWrap, wrapRadius, tileMap.width
            )
            if (h == 0L) return null

            val terrainMap = HashMap<String, Int>(ruleset.terrains.size * 2)
            var nextId = 0
            for (name in ruleset.terrains.keys) {
                terrainMap[name] = nextId++
            }
            val handleRef = AtomicLong(h)
            val mirror = NativeWorldMirror(
                tileMap = tileMap,
                handleRef = handleRef,
                numTiles = numTiles,
                bitsetWords = (numTiles + 63) ushr 6,
                terrainIdMap = terrainMap
            )
            val phantom = PhantomReference(mirror, refQueue)
            liveHandles[phantom] = handleRef
            return mirror
        }
    }
}
