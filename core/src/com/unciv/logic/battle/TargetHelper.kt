package com.unciv.logic.battle

import com.unciv.Constants
import com.unciv.logic.automation.Timers.Companion.timeThis
import com.unciv.logic.city.City
import com.unciv.logic.map.mapunit.MapUnit
import com.unciv.logic.map.mapunit.movement.PathsToTilesWithinTurn
import com.unciv.logic.map.tile.Tile
import com.unciv.models.ruleset.unique.GameContext
import com.unciv.models.ruleset.unique.UniqueType
import yairm210.purity.annotations.Readonly

object TargetHelper {
    @Readonly
    fun getAttackableEnemies(
        unit: MapUnit,
        unitDistanceToTiles: PathsToTilesWithinTurn,
        tilesToCheck: List<Tile>? = null,
        stayOnTile: Boolean = false
    ): ArrayList<AttackableTile> = timeThis("getAttackableEnemies") {
        val rangeOfAttack = unit.getRange()
        val attackableTiles = ArrayList<AttackableTile>()

        val unitMustBeSetUp = unit.hasUnique(UniqueType.MustSetUp)
        val tilesToAttackFrom = if (stayOnTile || unit.baseUnit.isAirUnit())
            sequenceOf(Pair(unit.currentTile, unit.currentMovement))
        else getTilesToAttackFromWhenUnitMoves(unitDistanceToTiles, unitMustBeSetUp, unit)

        // Fast bounding-disk pre-filter + native Rust single-pass pair discovery for non-nuke / non-air-sweep units.
        // Eliminates redundant LOS calls (37x+ for long-range units on roads) and skips LOS entirely when no enemy is nearby.
        if (!unit.isNuclearWeapon() && !unit.isPreparingAirSweep()) {
            val fromList = ArrayList<Pair<Tile, Float>>()
            val isMelee = unit.baseUnit.isMelee()
            val isEscortingMelee = isMelee && unit.isEscorting()
            val escortUnit = if (isEscortingMelee) unit.getOtherEscortUnit() else null
            val unitTile = unit.currentTile
            var maxReachAerial = 0
            for (pair in tilesToAttackFrom) {
                if (escortUnit != null) {
                    if (!escortUnit.movement.canReachInCurrentTurn(pair.first)
                        || escortUnit.currentMovement - escortUnit.movement.getDistanceToTiles()[pair.first]!!.totalMovement <= 0f)
                        continue
                }
                fromList.add(pair)
                val d = unitTile.aerialDistanceTo(pair.first)
                if (d > maxReachAerial) maxReachAerial = d
            }
            if (fromList.isEmpty()) return attackableTiles

            val effectiveRange = if (isMelee) 1 else rangeOfAttack
            val searchRadius = maxReachAerial + effectiveRange
            val candidateEnemies = ArrayList<Tile>(8)
            val civ = unit.civ
            val visibleSet = tilesToCheck?.toHashSet()
            unitTile.tileMap.forEachTileInDistance(unitTile.position, searchRadius) { t ->
                // Cheap pointer pre-check before full tileContainsAttackableEnemy
                if (!t.isCityCenter() && t.militaryUnit == null && t.civilianUnit == null) return@forEachTileInDistance
                val inView = if (visibleSet != null) t in visibleSet else t in civ.viewableTiles
                if (inView && tileContainsAttackableEnemy(unit, t, tilesToCheck)) {
                    candidateEnemies.add(t)
                }
            }
            if (candidateEnemies.isEmpty()) return attackableTiles

            val indirectFire = unit.baseUnit.isAirUnit() || unit.hasUnique(UniqueType.IndirectFire, checkCivInfoUniques = true)
            val mirror = unitTile.tileMap.getOrCreateNativeWorld()
            if (mirror != null) {
                val fromIdx = IntArray(fromList.size) { fromList[it].first.zeroBasedIndex }
                val fromMoveBits = IntArray(fromList.size) { java.lang.Float.floatToRawIntBits(fromList[it].second) }
                val enemyIdx = IntArray(candidateEnemies.size) { candidateEnemies[it].zeroBasedIndex }
                val pairs = mirror.findAttackablePairs(
                    civ, fromIdx, fromMoveBits, effectiveRange, isMelee, indirectFire, enemyIdx
                )
                val tileList = unitTile.tileMap.tileList
                var i = 0
                val len = pairs.size
                while (i + 2 < len) {
                    val fromT = tileList[pairs[i]]
                    val targetT = tileList[pairs[i + 1]]
                    val moveLeft = java.lang.Float.intBitsToFloat(pairs[i + 2])
                    attackableTiles.add(AttackableTile(fromT, targetT, moveLeft, Battle.getMapCombatantOfTile(targetT)))
                    i += 3
                }
                return attackableTiles
            }

            // Pure-Kotlin fallback using the already-filtered candidateEnemies list
            val enemySet = candidateEnemies.toHashSet()
            for ((reachableTile, movementLeft) in fromList) {
                val tilesInAttackRange =
                    if (isMelee) reachableTile.neighbors
                    else if (indirectFire) reachableTile.getTilesInDistance(rangeOfAttack)
                    else reachableTile.tileMap.getViewableTiles(reachableTile.position, rangeOfAttack, true).asSequence()
                for (tile in tilesInAttackRange) {
                    if (tile != reachableTile && tile in enemySet) {
                        attackableTiles += AttackableTile(reachableTile, tile, movementLeft, Battle.getMapCombatantOfTile(tile))
                    }
                }
            }
            return attackableTiles
        }

        val tilesWithEnemies: HashSet<Tile> = HashSet()
        val tilesWithoutEnemies: HashSet<Tile> = HashSet()
        for ((reachableTile, movementLeft) in tilesToAttackFrom) {  // tiles we'll still have energy after we reach there
            // If we are a melee unit that is escorting, we only want to be able to attack from this
            // tile if the escorted unit can also move into the tile we are attacking if we kill the enemy unit.
            if (unit.baseUnit.isMelee() && unit.isEscorting()) {
                val escortingUnit = unit.getOtherEscortUnit()!!
                if (!escortingUnit.movement.canReachInCurrentTurn(reachableTile)
                    || escortingUnit.currentMovement - escortingUnit.movement.getDistanceToTiles()[reachableTile]!!.totalMovement <= 0f) 
                    continue
            }

            // Nukes ignore line of sight when checking range, and don't require a visible enemy on the target tile
            val tilesInAttackRange =
                if (unit.baseUnit.isMelee()) reachableTile.neighbors
                else if (unit.baseUnit.isAirUnit() || unit.isNuclearWeapon() || unit.hasUnique(UniqueType.IndirectFire, checkCivInfoUniques = true))
                    reachableTile.getTilesInDistance(rangeOfAttack)
                else reachableTile.tileMap.getViewableTiles(reachableTile.position, rangeOfAttack, true).asSequence()

            for (tile in tilesInAttackRange) {
                when {
                    // Since military units can technically enter tiles with enemy civilians,
                    // some try to move to to the tile and then attack the unit it contains, which is silly
                    tile == reachableTile -> continue

                    unit.isNuclearWeapon() -> {
                        if (Nuke.mayUseNuke(MapUnitCombatant(unit), tile))
                            attackableTiles += AttackableTile(
                                reachableTile, tile, movementLeft,
                                Battle.getMapCombatantOfTile(tile)
                            )
                    }

                    tile in tilesWithEnemies -> attackableTiles += AttackableTile(
                        reachableTile,
                        tile,
                        movementLeft,
                        Battle.getMapCombatantOfTile(tile)
                    )
                    tile in tilesWithoutEnemies -> continue // avoid checking the same empty tile multiple times
                    tileContainsAttackableEnemy(unit, tile, tilesToCheck) || unit.isPreparingAirSweep() -> {
                        tilesWithEnemies += tile
                        attackableTiles += AttackableTile(
                            reachableTile, tile, movementLeft,
                            Battle.getMapCombatantOfTile(tile)
                        )
                    }
                    else -> tilesWithoutEnemies += tile
                }
            }
        }
        return attackableTiles
    }

    @Readonly
    private fun getTilesToAttackFromWhenUnitMoves(unitDistanceToTiles: PathsToTilesWithinTurn, unitMustBeSetUp: Boolean, unit: MapUnit) =
        unitDistanceToTiles.asSequence()
            .sortedWith {a,b -> a.value.totalMovement.compareTo(b.value.totalMovement) }
            .map { (tile, distance) ->
                val movementPointsToExpendAfterMovement = if (unitMustBeSetUp) 1 else 0
                val movementPointsToExpendHere =
                    if (unitMustBeSetUp && !unit.isSetUpForSiege()) 1 else 0
                val movementPointsToExpendBeforeAttack =
                    if (tile == unit.currentTile) movementPointsToExpendHere else movementPointsToExpendAfterMovement
                val movementLeft =
                    unit.currentMovement - distance.totalMovement - movementPointsToExpendBeforeAttack
                Pair(tile, movementLeft)
            }
            // still got leftover movement points after all that, to attack
            .filter { it.second > Constants.minimumMovementEpsilon }
            .filter {
                it.first == unit.getTile() || unit.movement.canMoveTo(it.first)
            }

    @Readonly
    private fun tileContainsAttackableEnemy(unit: MapUnit, tile: Tile, tilesToCheck: List<Tile>?): Boolean {
        if (tile !in (tilesToCheck ?: unit.civ.viewableTiles) || !containsAttackableEnemy(tile, MapUnitCombatant(unit)) )
            return false
        val mapCombatant = Battle.getMapCombatantOfTile(tile)

        return (!unit.baseUnit.isMelee() || mapCombatant !is MapUnitCombatant || !mapCombatant.unit.isCivilian() || unit.movement.canPassThrough(tile))
    }

    @Readonly
    fun containsAttackableEnemy(tile: Tile, combatant: ICombatant): Boolean {
        if (combatant is MapUnitCombatant && combatant.unit.isEmbarked() && !combatant.hasUnique(UniqueType.AttackOnSea)) {
            // Can't attack water units while embarked, only land
            if (tile.isWater || combatant.isRanged())
                return false
        }

        val tileCombatant = Battle.getMapCombatantOfTile(tile) ?: return false
        if (tileCombatant.getCivInfo() == combatant.getCivInfo()) return false
        // If the user automates units, one may capture the city before the user had a chance to decide what to do with it,
        //  and then the next unit should not attack that city
        if (tileCombatant is CityCombatant && tileCombatant.city.hasJustBeenConquered) return false
        if (!combatant.getCivInfo().isAtWarWith(tileCombatant.getCivInfo())) return false

        if (combatant is MapUnitCombatant && combatant.isLandUnit() && combatant.isMelee() && tile.isWater &&
            !combatant.getCivInfo().tech.unitsCanEmbark && !combatant.unit.cache.canMoveOnWater
        )
            return false

        
        if (combatant is MapUnitCombatant) {
            val gameContext = GameContext(
                unit = combatant.unit, tile = tile, 
                ourCombatant = combatant, theirCombatant = tileCombatant, combatAction = CombatAction.Attack)

            if (combatant.hasUnique(UniqueType.CannotAttack, gameContext))
                return false

            if (combatant.unit.getMatchingUniques(UniqueType.CanOnlyAttackUnits, gameContext).run {
                    any() && none { tileCombatant.matchesFilter(it.params[0]) }
                }
            )
                return false

            if (combatant.unit.getMatchingUniques(UniqueType.CanOnlyAttackTiles, gameContext).run {
                    any() && none { tile.matchesFilter(it.params[0]) }
                }
            )
                return false
        }

        // Only units with the right unique can view submarines (or other invisible units) from more then one tile away.
        // Garrisoned invisible units can be attacked by anyone, as else the city will be in invincible.
        if (!tile.isCityCenter())
            return tileCombatant.isVisibleTo(combatant.getCivInfo())

        return true
    }

    /** Get a list of visible tiles which have something attackable */
    @Readonly
    fun getBombardableTiles(city: City): Sequence<Tile> =
            city.getCenterTile().getTilesInDistance(city.getBombardRange())
                    .filter { it.isVisible(city.civ) && containsAttackableEnemy(it, CityCombatant(city)) }

}
