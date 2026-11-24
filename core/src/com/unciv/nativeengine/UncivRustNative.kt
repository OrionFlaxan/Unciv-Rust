package com.unciv.nativeengine

import yairm210.purity.annotations.Pure
import yairm210.purity.annotations.Readonly

/**
 * Low-level JNI declarations matching `rust/unciv-native/src/jni_bridge.rs`.
 *
 * All functions operate exclusively on primitive arrays and scalar bitfields (zero callbacks
 * into Kotlin/JVM). Callers should go through [NativeEngine] and [NativeWorldMirror] rather
 * than invoking these external methods directly.
 */
object UncivRustNative {

    // Terrain static flags (matches world.rs TF_*)
    const val TF_LAND = 1 shl 0
    const val TF_WATER = 1 shl 1
    const val TF_OCEAN = 1 shl 2
    const val TF_IMPASSABLE = 1 shl 3
    const val TF_HILL = 1 shl 4
    const val TF_ROUGH = 1 shl 5
    const val TF_ICE = 1 shl 6
    const val TF_FOREST_OR_JUNGLE = 1 shl 7
    const val TF_ADJ_RIVER = 1 shl 8
    const val TF_ADJ_COAST = 1 shl 9
    const val TF_NATURAL_WONDER = 1 shl 10

    // Dynamic tile flags (matches world.rs TILE_*)
    const val TILE_ROAD_NONE = 0
    const val TILE_ROAD_ROAD = 1
    const val TILE_ROAD_RAILROAD = 2
    const val TILE_ROAD_PILLAGED = 1 shl 2
    const val TILE_CITY_CENTER = 1 shl 3
    const val TILE_JUST_CONQUERED = 1 shl 4
    const val TILE_ENCAMPMENT = 1 shl 5
    const val TILE_RUIN = 1 shl 6
    const val TILE_CITY_BLOCKADED = 1 shl 7
    const val TILE_HAS_HARBOR = 1 shl 8

    // Military unit flags on tile (matches world.rs UF_*)
    const val UF_WATER = 1 shl 0
    const val UF_EMBARKED = 1 shl 1
    const val UF_RANGED = 1 shl 2
    const val UF_INVISIBLE = 1 shl 3
    const val UF_RANGE_SHIFT = 4

    // Diplomacy matrix flags (matches world.rs DIPLO_*)
    const val DIPLO_AT_WAR = 1 shl 0
    const val DIPLO_CAN_PASS = 1 shl 1
    const val DIPLO_OPEN_BORDERS = 1 shl 2
    const val DIPLO_FRIENDLY_TERRITORY = 1 shl 3
    const val DIPLO_KNOWS = 1 shl 4
    const val DIPLO_REL_SHIFT = 5
    const val DIPLO_REL_FAVORABLE = 5

    // Per-civ flags (matches world.rs CF_*)
    const val CF_ROADS_ACROSS_RIVERS = 1 shl 0
    const val CF_FOREST_AS_ROAD = 1 shl 1
    const val CF_IGNORE_HILL = 1 shl 2
    const val CF_CAN_EMBARK = 1 shl 3
    const val CF_EMBARK_OCEAN = 1 shl 4
    const val CF_ALL_OCEAN = 1 shl 5
    const val CF_PASS_IMPASSABLE = 1 shl 6
    const val CF_HAS_ENEMY_PENALTY = 1 shl 7
    const val CF_IS_CITY_STATE = 1 shl 8
    const val CF_IS_BARBARIAN = 1 shl 9
    const val CF_SPECTATOR_VISION = 1 shl 10

    // UnitProfile flags (matches pathing.rs UP_*)
    const val UP_IS_LAND = 1 shl 0
    const val UP_IS_WATER = 1 shl 1
    const val UP_IS_CIVILIAN = 1 shl 2
    const val UP_CANNOT_MOVE = 1 shl 3
    const val UP_ALL_TILES_COST_1 = 1 shl 4
    const val UP_IGNORES_TERRAIN_COST = 1 shl 5
    const val UP_IGNORES_ZOC = 1 shl 6
    const val UP_ROUGH_TERRAIN_PENALTY = 1 shl 7
    const val UP_CAN_MOVE_ON_WATER = 1 shl 8
    const val UP_CAN_PASS_IMPASSABLE = 1 shl 9
    const val UP_CAN_ENTER_ICE = 1 shl 10
    const val UP_CANNOT_EMBARK = 1 shl 11
    const val UP_CANNOT_ENTER_OCEAN = 1 shl 12
    const val UP_SPECIFIC_ALLOW_OCEAN = 1 shl 13
    const val UP_CAN_ENTER_FOREIGN = 1 shl 14
    const val UP_CAN_ENTER_CITY_STATES = 1 shl 15
    const val UP_AVOID_ENEMY_TERRITORY = 1 shl 16
    const val UP_NO_TERRAIN_MOVEMENT_UNIQUES = 1 shl 17
    const val UP_NO_BASE_OR_HILL_DM = 1 shl 18
    const val UP_CONSIDER_ZOC = 1 shl 19
    const val UP_AVOID_DAMAGING_TERRAIN = 1 shl 20
    const val UP_ALLOW_UNPASSABLE_DEST = 1 shl 21
    const val UP_ASSUME_UNEXPLORED_PASSABLE = 1 shl 22

    // Capital connection mediums (matches connectivity.rs MED_*)
    const val MED_START = 1 shl 0
    const val MED_ROAD = 1 shl 1
    const val MED_RAILROAD = 1 shl 2
    const val MED_HARBOR_FROM_ROAD = 1 shl 3
    const val MED_HARBOR_FROM_RAILROAD = 1 shl 4

    // Capital city flags (matches connectivity.rs CITY_*)
    const val CITY_IS_OWN = 1 shl 0
    const val CITY_HAS_HARBOR = 1 shl 1
    const val CITY_IS_BLOCKADED = 1 shl 2

    // Flow-field IDs (matches world.rs FLOW_*)
    const val FLOW_ENEMY_CITY_LAND = 0
    const val FLOW_ENEMY_CITY_WATER = 1
    const val FLOW_FRIENDLY_CITY = 2
    const val FLOW_SIEGED_CITY = 3
    const val FLOW_EXPLORE_FRONTIER = 4
    const val FLOW_UNREACHED = 65535

    // Turbo AI output action codes (matches turbo_ai.rs ACTION_*)
    const val ACTION_FALLBACK_KOTLIN = 0
    const val ACTION_ATTACK = 1
    const val ACTION_MOVE = 2
    const val ACTION_FORTIFY_HEAL = 3
    const val ACTION_STAY_IDLE = 4

    // Turbo AI unit combat flags (matches turbo_ai.rs TCF_*)
    const val TCF_IS_MELEE = 1 shl 0
    const val TCF_IS_RANGED = 1 shl 1
    const val TCF_IS_SIEGE = 1 shl 2
    const val TCF_INDIRECT_FIRE = 1 shl 3
    const val TCF_NO_WOUNDED_PENALTY = 1 shl 4
    const val TCF_CAN_MOVE_AFTER_ATTACK = 1 shl 5
    const val TCF_CANNOT_CAPTURE_CITIES = 1 shl 6
    const val TCF_ATTACK_ACROSS_RIVER = 1 shl 7
    const val TCF_ATTACK_ACROSS_COAST = 1 shl 8
    const val TCF_MUST_SETUP = 1 shl 9
    const val TCF_IS_SETUP = 1 shl 10
    const val TCF_CAN_FORTIFY = 1 shl 11
    const val TCF_IS_EMBARKED = 1 shl 12
    const val TCF_IS_CITY_STATE = 1 shl 13

    const val NO_TERRAIN_ID: Byte = -1 // 255 unsigned

    // 1. Lifecycle & World Sync
    @JvmStatic external fun nativeCreateWorld(
        numTiles: Int,
        numCivs: Int,
        worldWrap: Boolean,
        wrapRadius: Int,
        mapWidth: Int
    ): Long

    @JvmStatic external fun nativeDestroyWorld(handle: Long)

    @JvmStatic external fun nativeSyncStaticTopology(
        handle: Long,
        posX: ShortArray,
        posY: ShortArray,
        neighbors: IntArray,
        riverMask: ByteArray,
        terrainFlags: IntArray,
        baseMoveCost: ByteArray,
        terrainDamage: ShortArray,
        tileHeight: ByteArray,
        unitHeight: ByteArray,
        continent: ShortArray,
        baseTerrainId: ByteArray,
        hillTerrainId: ByteArray,
        lastTerrainId: ByteArray,
        featureTerrainMask: LongArray
    )

    @JvmStatic external fun nativeSyncDynamicState(
        handle: Long,
        numCivs: Int,
        ownerCiv: ShortArray,
        tileFlags: ShortArray,
        milUnitCiv: ShortArray,
        milUnitFlags: ByteArray,
        civUnitCiv: ShortArray,
        airUnitCiv: ShortArray,
        cityBombardRange: ByteArray,
        diploMatrix: ByteArray,
        civFlags: IntArray,
        civRoadSpeedFpm: ShortArray,
        civRoadSpeedF32: FloatArray,
        civPassableImpassableMask: LongArray
    )

    @JvmStatic external fun nativeSyncExploredBits(handle: Long, civIdx: Int, bits: LongArray)
    @JvmStatic external fun nativeSyncVisibleBits(handle: Long, civIdx: Int, bits: LongArray)

    @JvmStatic external fun nativePatchTileUnits(
        handle: Long,
        tileIdx: Int,
        milCiv: Short,
        milFlags: Byte,
        civCiv: Short,
        airCiv: Short
    )

    @JvmStatic external fun nativePatchTileDynamic(
        handle: Long,
        tileIdx: Int,
        ownerCiv: Short,
        tileFlags: Short,
        bombardRange: Byte
    )

    @JvmStatic external fun nativePatchTileTerrain(
        handle: Long,
        tileIdx: Int,
        terrainFlags: Int,
        baseMoveCost: Byte,
        terrainDamage: Short,
        tileHeight: Byte,
        unitHeight: Byte,
        baseTerrainId: Byte,
        hillTerrainId: Byte,
        lastTerrainId: Byte,
        featureTerrainMask: Long
    )

    @JvmStatic external fun nativePatchTileExplored(
        handle: Long,
        civIdx: Int,
        tileIdx: Int,
        explored: Boolean
    )

    @JvmStatic external fun nativePatchDiplo(handle: Long, civA: Int, civB: Int, diploByte: Byte)

    @JvmStatic external fun nativePatchCivFlags(
        handle: Long,
        civIdx: Int,
        civFlags: Int,
        roadSpeedFpm: Short,
        roadSpeedF32: Float,
        passableImpassableMask: Long
    )

    @JvmStatic external fun nativeMarkVisionDirty(handle: Long, civIdx: Int)

    // 2. Pathfinding & Movement
    @Readonly @JvmStatic external fun nativeComputeDistanceWithinTurn(
        handle: Long,
        civIdx: Int,
        profileFlags: Int,
        currentMoveF32: Float,
        currentMoveFpm: Int,
        fullMoveFpm: Int,
        costEmbarkFpm: Int,
        costDisembarkFpm: Int,
        enemyPenaltyFpm: Int,
        dmFeatureMask: Long,
        dmBaseMask: Long,
        dmHillMask: Long,
        startTile: Int,
        includeUnpassableBoundary: Boolean
    ): IntArray

    @Readonly @JvmStatic external fun nativeFindShortestPath(
        handle: Long,
        civIdx: Int,
        profileFlags: Int,
        currentMoveF32: Float,
        currentMoveFpm: Int,
        fullMoveFpm: Int,
        costEmbarkFpm: Int,
        costDisembarkFpm: Int,
        enemyPenaltyFpm: Int,
        dmFeatureMask: Long,
        dmBaseMask: Long,
        dmHillMask: Long,
        startTile: Int,
        destTile: Int,
        maxTurns: Int
    ): IntArray

    @Readonly @JvmStatic external fun nativeBatchFirstReachable(
        handle: Long,
        civIdx: Int,
        profileFlags: Int,
        currentMoveF32: Float,
        currentMoveFpm: Int,
        fullMoveFpm: Int,
        costEmbarkFpm: Int,
        costDisembarkFpm: Int,
        enemyPenaltyFpm: Int,
        dmFeatureMask: Long,
        dmBaseMask: Long,
        dmHillMask: Long,
        startTile: Int,
        candidates: IntArray,
        maxTurns: Int
    ): IntArray

    // 3. Vision & Line-of-Sight
    @Readonly @JvmStatic external fun nativeComputeViewableTiles(
        handle: Long,
        originIdx: Int,
        sightDistance: Int,
        forAttack: Boolean
    ): IntArray

    @JvmStatic external fun nativeRebuildCivVision(
        handle: Long,
        civIdx: Int,
        staticTiles: IntArray,
        unitTilesFlat: IntArray
    ): IntArray

    @JvmStatic external fun nativeApplyUnitVisionDelta(
        handle: Long,
        civIdx: Int,
        oldTiles: IntArray,
        newTiles: IntArray
    ): IntArray

    // 4. AI Threat Field, Attack Discovery & Exploration
    @JvmStatic external fun nativeBuildThreatField(
        handle: Long,
        civIdx: Int,
        extraDetectedEnemyTiles: IntArray
    )

    @Readonly @JvmStatic external fun nativeGetDistanceToClosestEnemy(
        handle: Long,
        tileIdx: Int,
        maxDist: Int,
        takeLargerValues: Boolean
    ): Int

    @Readonly @JvmStatic external fun nativeGetEnemyTilesInDistance(
        handle: Long,
        tileIdx: Int,
        maxDist: Int
    ): IntArray

    @Readonly @JvmStatic external fun nativeFindAttackablePairs(
        handle: Long,
        civIdx: Int,
        fromTiles: IntArray,
        fromMoveLeftBits: IntArray,
        attackRange: Int,
        isMelee: Boolean,
        indirectFire: Boolean,
        candidateEnemyTiles: IntArray
    ): IntArray

    @Readonly @JvmStatic external fun nativeFilterAndScoreExploreTiles(
        handle: Long,
        civIdx: Int,
        profileFlags: Int,
        costEmbarkFpm: Int,
        costDisembarkFpm: Int,
        dmFeatureMask: Long,
        dmBaseMask: Long,
        dmHillMask: Long,
        candidates: IntArray,
        visRange: Int
    ): IntArray

    // 5. Connectivity
    @Readonly @JvmStatic external fun nativeFindCapitalConnections(
        handle: Long,
        civIdx: Int,
        cityTiles: IntArray,
        cityFlags: IntArray,
        capitalSlot: Int,
        roadResearched: Boolean,
        railroadResearched: Boolean
    ): IntArray

    @Readonly @JvmStatic external fun nativeFindRoadPath(
        handle: Long,
        civIdx: Int,
        startTile: Int,
        targets: IntArray,
        validRoadBits: LongArray,
        maxHalfCost: Int
    ): IntArray

    @Readonly @JvmStatic external fun nativeEvalAttackPaths(
        handle: Long,
        civIdx: Int,
        targetCivIdx: Int,
        ourCityTiles: IntArray,
        enemyCityTiles: IntArray,
        capitalTile: Int,
        allEnemyCityTiles: IntArray
    ): Float

    // 6. Stateless Map Generation & Compression
    @Pure @JvmStatic external fun nativePerlinBatch(
        xs: DoubleArray,
        ys: DoubleArray,
        zs: DoubleArray,
        nOctaves: Int,
        persistence: Double,
        lacunarity: Double,
        scale: Double,
        ridged: Boolean
    ): DoubleArray

    @JvmStatic external fun nativeAssignContinents(
        neighbors: IntArray,
        passableLand: ByteArray,
        outContinents: IntArray
    ): IntArray

    @Pure @JvmStatic external fun nativeGzipBase64Encode(input: ByteArray, level: Int): ByteArray
    @Pure @JvmStatic external fun nativeGzipBase64Decode(input: ByteArray): ByteArray?

    // 7. v0.2.0 — Fast Topology Clone, O(V) Flow-Fields, Batch Reach & Turbo AI
    @Readonly @JvmStatic external fun nativeCloneWorldTopology(srcHandle: Long): Long

    @JvmStatic external fun nativeSyncCombatSnapshot(
        handle: Long,
        milUnitHp: ByteArray,
        milUnitStrength: ShortArray,
        cityHp: ShortArray,
        cityMaxHp: ShortArray,
        cityStrength: ShortArray,
        tileDefBonusPct: ShortArray
    )

    @JvmStatic external fun nativePatchTileCombat(
        handle: Long,
        tileIdx: Int,
        milHp: Byte,
        milStr: Short,
        cityHp: Short,
        cityMaxHp: Short,
        cityStr: Short,
        defBonusPct: Short
    )

    @JvmStatic external fun nativePrecomputeCivFlowFields(
        handle: Long,
        civIdx: Int,
        enemyCityTiles: IntArray,
        siegedCityTiles: IntArray,
        ownCityTiles: IntArray
    )

    @Readonly @JvmStatic external fun nativeQueryFlowNextStep(
        handle: Long,
        fieldKind: Int,
        fromTile: Int
    ): Int

    @Readonly @JvmStatic external fun nativeQueryFlowDist(
        handle: Long,
        fieldKind: Int,
        fromTile: Int
    ): Int

    @Readonly @JvmStatic external fun nativeGetDangerousTilesInRadius(
        handle: Long,
        civIdx: Int,
        centerTile: Int,
        radius: Int
    ): IntArray

    @Readonly @JvmStatic external fun nativeHasEnoughSettlerRoom(
        handle: Long,
        cityTiles: IntArray,
        minCount: Int
    ): Boolean

    @Readonly @JvmStatic external fun nativeBatchCanReach(
        handle: Long,
        civIdx: Int,
        startTile: Int,
        flags: Int,
        currentMoveF32: Float,
        currentMoveFpm: Int,
        fullMoveFpm: Int,
        costToEmbarkFpm: Int,
        costToDisembarkFpm: Int,
        enemyPenaltyFpm: Int,
        dmFeatureMask: Long,
        dmBaseMask: Long,
        dmHillMask: Long,
        candidates: IntArray,
        maxTurns: Int
    ): ByteArray

    @Readonly @JvmStatic external fun nativePlanCivCombatUnits(
        handle: Long,
        civIdx: Int,
        startTiles: IntArray,
        hps: IntArray,
        attackStrs: FloatArray,
        rangedStrs: FloatArray,
        ranges: IntArray,
        combatFlags: IntArray,
        profileFlags: IntArray,
        currentMoveF32s: FloatArray,
        currentMoveFpms: IntArray,
        fullMoveFpms: IntArray,
        embarkFpms: IntArray,
        disembarkFpms: IntArray,
        enemyPenaltyFpms: IntArray,
        dmFeatureMasks: LongArray,
        dmBaseMasks: LongArray,
        dmHillMasks: LongArray
    ): IntArray
}
