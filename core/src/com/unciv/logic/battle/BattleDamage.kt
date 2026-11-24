package com.unciv.logic.battle

import com.unciv.logic.map.tile.Tile
import com.unciv.models.Counter
import com.unciv.models.ruleset.GlobalUniques
import com.unciv.models.ruleset.unique.GameContext
import com.unciv.models.ruleset.unique.Unique
import com.unciv.models.ruleset.unique.UniqueTarget
import com.unciv.models.ruleset.unique.UniqueType
import com.unciv.models.translations.tr
import com.unciv.ui.components.extensions.toPercent
import yairm210.purity.annotations.LocalState
import yairm210.purity.annotations.Pure
import yairm210.purity.annotations.Readonly
import yairm210.purity.annotations.ReturnsNewInstance
import kotlin.collections.set
import kotlin.math.max
import kotlin.math.pow
import kotlin.math.roundToInt
import kotlin.random.Random

object BattleDamage {

    @Readonly
    private fun getModifierStringFromUnique(unique: Unique): String {
        val source = when (unique.sourceObjectType) {
            UniqueTarget.Unit -> "Unit ability"
            UniqueTarget.Nation -> "National ability"
            UniqueTarget.Global -> GlobalUniques.getUniqueSourceDescription(unique)
            else -> "[${unique.sourceObjectName}] ([${unique.getSourceNameForUser()}])"
        }.tr()
        if (unique.modifiers.isEmpty()) return source

        val conditionalsText = unique.modifiers.joinToString { it.text.tr() }
        return "$source - $conditionalsText"
    }

    @Readonly @ReturnsNewInstance
    private fun getGeneralModifiers(combatant: ICombatant, enemy: ICombatant, combatAction: CombatAction, tileToAttackFrom: Tile): Counter<String> {
        val modifiers = Counter<String>()

        val conditionalState = getGameContext(combatAction, combatant, enemy)
        val civInfo = combatant.getCivInfo()

        if (combatant is MapUnitCombatant) {

            val unitUniqueModifiers = getUnitUniqueModifiers(combatant, enemy, conditionalState, tileToAttackFrom)
            modifiers.add(unitUniqueModifiers)

            val civResources = civInfo.getCivResourcesByName()
            for (resource in combatant.unit.getResourceRequirementsPerTurn().keys)
                if (civResources[resource]!! < 0 && !civInfo.isBarbarian)
                    modifiers["Missing resource"] = BattleConstants.MISSING_RESOURCES_MALUS

            val (greatGeneralName, greatGeneralBonus) = GreatGeneralImplementation.getGreatGeneralBonus(combatant, enemy, combatAction)
            if (greatGeneralBonus != 0)
                modifiers[greatGeneralName] = greatGeneralBonus

        } else if (combatant is CityCombatant) {
            combatant.city.forEachMatchingUnique(UniqueType.StrengthForCities, conditionalState) { unique ->
                modifiers.add(getModifierStringFromUnique(unique), unique.params[0].toInt())
            }
        }

        if (enemy.getCivInfo().isBarbarian) {
            modifiers["Difficulty"] =
                (civInfo.gameInfo.getDifficulty().barbarianBonus * 100).toInt()
        }

        return modifiers
    }

    @Readonly
    private fun getGameContext(
        combatAction: CombatAction,
        combatant: ICombatant,
        enemy: ICombatant,
    ): GameContext {
        val attackedTile =
            if (combatAction == CombatAction.Attack) enemy.getTile()
            else combatant.getTile()

        val conditionalState = GameContext(
            combatant.getCivInfo(),
            city = (combatant as? CityCombatant)?.city,
            ourCombatant = combatant,
            theirCombatant = enemy,
            attackedTile = attackedTile,
            combatAction = combatAction
        )
        return conditionalState
    }

    @Readonly
    private fun getUnitUniqueModifiers(combatant: MapUnitCombatant, enemy: ICombatant, conditionalState: GameContext,
                                       tileToAttackFrom: Tile): Counter<String> {
        val civInfo = combatant.getCivInfo()
        val modifiers = Counter<String>()

        combatant.unit.forEachMatchingUnique(UniqueType.Strength, conditionalState, checkCivInfoUniques = true) { unique ->
            modifiers.add(getModifierStringFromUnique(unique), unique.params[0].toInt())
        }

        // e.g., Mehal Sefari https://civilization.fandom.com/wiki/Mehal_Sefari_(Civ5)
        for (unique in combatant.getMatchingUniques(
            UniqueType.StrengthNearCapital, conditionalState, true
        )) {
            if (civInfo.cities.isEmpty() || civInfo.getCapital() == null) break
            val distance =
                combatant.getTile().aerialDistanceTo(civInfo.getCapital()!!.getCenterTile())
            // https://steamcommunity.com/sharedfiles/filedetails/?id=326411722#464287
            val effect = unique.params[0].toInt() - 3 * distance
            if (effect > 0)
                modifiers.add(getModifierStringFromUnique(unique), effect)
        }

        //https://www.carlsguides.com/strategy/civilization5/war/combatbonuses.php
        var adjacentUnits = combatant.getTile().neighbors.flatMap { it.getUnits() }
        if (enemy.getTile() !in combatant.getTile().neighbors && tileToAttackFrom in combatant.getTile().neighbors
            && enemy is MapUnitCombatant
        )
            adjacentUnits += sequenceOf(enemy.unit)

        // e.g., Maori Warrior - https://civilization.fandom.com/wiki/Maori_Warrior_(Civ5)
        val strengthMalus = adjacentUnits.filter { it.civ.isAtWarWith(combatant.getCivInfo()) }
            .flatMap { it.getMatchingUniques(UniqueType.StrengthForAdjacentEnemies) }
            .filter { combatant.matchesFilter(it.params[1]) && combatant.getTile().matchesFilter(it.params[2]) }
            .maxByOrNull { it.params[0] }
        if (strengthMalus != null) {
            modifiers.add("Adjacent enemy units", strengthMalus.params[0].toInt())
        }
        return modifiers
    }

    @Readonly
    fun getAttackModifiers(
        attacker: ICombatant,
        defender: ICombatant, tileToAttackFrom: Tile
    ): Counter<String> {
        val modifiers = getGeneralModifiers(attacker, defender, CombatAction.Attack, tileToAttackFrom)

        if (attacker is MapUnitCombatant) {

            val terrainAttackModifiers = getTerrainAttackModifiers(attacker, defender, tileToAttackFrom)
            modifiers.add(terrainAttackModifiers)

            // Air unit attacking with Air Sweep
            if (attacker.unit.isPreparingAirSweep())
                modifiers.add(getAirSweepAttackModifiers(attacker))

            if (attacker.isMelee()) {
                val numberOfOtherAttackersSurroundingDefender = defender.getTile().neighbors.count {
                    it.militaryUnit != null && it.militaryUnit != attacker.unit
                            && it.militaryUnit!!.civ == attacker.getCivInfo()
                            && MapUnitCombatant(it.militaryUnit!!).isMelee()
                }
                if (numberOfOtherAttackersSurroundingDefender > 0) {
                    var flankingBonus = BattleConstants.BASE_FLANKING_BONUS

                    // e.g., Discipline policy - https://civilization.fandom.com/wiki/Discipline_(Civ5)
                    attacker.unit.forEachMatchingUnique(UniqueType.FlankAttackBonus, checkCivInfoUniques = true,
                            gameContext = getGameContext(CombatAction.Attack, attacker, defender)) { unique ->
                        flankingBonus *= unique.params[0].toPercent()
                    }
                    modifiers["Flanking"] =
                        (flankingBonus * numberOfOtherAttackersSurroundingDefender).toInt()
                }
            }

        }

        return modifiers
    }

    @Readonly
    private fun getTerrainAttackModifiers(attacker: MapUnitCombatant, defender: ICombatant, tileToAttackFrom: Tile): Counter<String> {
        val modifiers = Counter<String>()
        if (attacker.unit.isEmbarked() && defender.getTile().isLand
            && !attacker.unit.hasUnique(UniqueType.AttackAcrossCoast)
        )
            modifiers["Landing"] = BattleConstants.LANDING_MALUS

        // Land Melee Unit attacking to Water
        if (attacker.unit.type.isLandUnit() && !attacker.getTile().isWater && attacker.isMelee() && defender.getTile().isWater
            && !attacker.unit.hasUnique(UniqueType.AttackAcrossCoast)
        )
            modifiers["Boarding"] = BattleConstants.BOARDING_MALUS

        // Melee Unit on water attacking to Land (not City) unit
        if (!attacker.unit.type.isAirUnit() && attacker.isMelee() && attacker.getTile().isWater && !defender.getTile().isWater
            && !attacker.unit.hasUnique(UniqueType.AttackAcrossCoast) && !defender.isCity()
        )
            modifiers["Landing"] = BattleConstants.LANDING_MALUS

        if (isMeleeAttackingAcrossRiverWithNoBridge(attacker, tileToAttackFrom, defender))
            modifiers["Across river"] = BattleConstants.ATTACKING_ACROSS_RIVER_MALUS
        return modifiers
    }

    @Readonly
    private fun isMeleeAttackingAcrossRiverWithNoBridge(attacker: MapUnitCombatant, tileToAttackFrom: Tile, defender: ICombatant) = (
        attacker.isMelee()
            &&
            (tileToAttackFrom.aerialDistanceTo(defender.getTile()) == 1
                && tileToAttackFrom.isConnectedByRiver(defender.getTile())
                && !attacker.unit.hasUnique(UniqueType.AttackAcrossRiver))
            &&
            (!tileToAttackFrom.hasConnection(attacker.getCivInfo()) // meaning, the tiles are not road-connected for this civ
                || !defender.getTile().hasConnection(attacker.getCivInfo())
                || !attacker.getCivInfo().tech.roadsConnectAcrossRivers)
        )

    @Readonly
    fun getAirSweepAttackModifiers(
        attacker: ICombatant
    ): Counter<String> {
        val modifiers = Counter<String>()

        if (attacker is MapUnitCombatant) {
            attacker.unit.forEachMatchingUnique(UniqueType.StrengthWhenAirsweep) { unique ->
                modifiers.add(getModifierStringFromUnique(unique), unique.params[0].toInt())
            }
        }

        return modifiers
    }

    @Readonly
    fun getDefenceModifiers(attacker: ICombatant, defender: ICombatant, tileToAttackFrom: Tile): Counter<String> {
        val modifiers = getGeneralModifiers(defender, attacker, CombatAction.Defend, tileToAttackFrom)
        val tile = defender.getTile()

        if (defender is MapUnitCombatant && !defender.unit.isEmbarked()) { // Embarked units get no terrain defensive bonuses

            val tileDefenceBonus = tile.getDefensiveBonus(unit = defender.unit)
            if (!defender.unit.hasUnique(UniqueType.NoDefensiveTerrainBonus, checkCivInfoUniques = true) && tileDefenceBonus > 0
                || !defender.unit.hasUnique(UniqueType.NoDefensiveTerrainPenalty, checkCivInfoUniques = true) && tileDefenceBonus < 0
            )
                modifiers["Tile"] = (tileDefenceBonus * 100).toInt()


            if (defender.unit.isFortified() || defender.unit.isGuarding())
                modifiers["Fortification"] = BattleConstants.FORTIFICATION_BONUS * defender.unit.getFortificationTurns()
        }

        return modifiers
    }

    @Readonly
    internal fun modifiersToFinalBonus(modifiers: Counter<String>): Float {
        var finalModifier = 1f
        for (modifierValue in modifiers.values) finalModifier += modifierValue / 100f
        return finalModifier
    }

    @Readonly
    private fun getHealthDependantDamageRatio(combatant: ICombatant): Float {
        return if (combatant !is MapUnitCombatant
            || combatant.unit.hasUnique(UniqueType.NoDamagePenaltyWoundedUnits, checkCivInfoUniques = true)
        ) 1f
        // Each 3 points of health reduces damage dealt by 1%
        else 1 - (100 - combatant.getHealth()) / BattleConstants.DAMAGE_REDUCTION_WOUNDED_UNIT_RATIO_PERCENTAGE
    }


    @Readonly
    private fun getGeneralModifierSum(
        combatant: ICombatant,
        enemy: ICombatant,
        combatAction: CombatAction,
        tileToAttackFrom: Tile
    ): Int {
        var sum = 0
        val conditionalState = getGameContext(combatAction, combatant, enemy)
        val civInfo = combatant.getCivInfo()

        if (combatant is MapUnitCombatant) {
            sum += getUnitUniqueModifierSum(combatant, enemy, conditionalState, tileToAttackFrom)

            if (!civInfo.isBarbarian) {
                val reqs = combatant.unit.getResourceRequirementsPerTurn()
                if (reqs.isNotEmpty()) {
                    val civResources = civInfo.getCivResourcesByName()
                    for (resource in reqs.keys) {
                        if (civResources[resource]!! < 0) {
                            sum += BattleConstants.MISSING_RESOURCES_MALUS
                            break // Matches Counter["Missing resource"] = ... single assignment
                        }
                    }
                }
            }

            sum += GreatGeneralImplementation.getGreatGeneralBonusInt(combatant, enemy, combatAction)
        } else if (combatant is CityCombatant) {
            combatant.city.forEachMatchingUnique(UniqueType.StrengthForCities, conditionalState) { unique ->
                sum += unique.params[0].toInt()
            }
        }

        if (enemy.getCivInfo().isBarbarian) {
            sum += (civInfo.gameInfo.getDifficulty().barbarianBonus * 100).toInt()
        }

        return sum
    }

    @Readonly
    private fun getUnitUniqueModifierSum(
        combatant: MapUnitCombatant,
        enemy: ICombatant,
        conditionalState: GameContext,
        tileToAttackFrom: Tile
    ): Int {
        val civInfo = combatant.getCivInfo()
        val unit = combatant.unit
        val combatTile = combatant.getTile()
        var sum = 0

        unit.forEachMatchingUnique(UniqueType.Strength, conditionalState, checkCivInfoUniques = true) { unique ->
            sum += unique.params[0].toInt()
        }

        val capitalTile = if (civInfo.cities.isNotEmpty()) civInfo.getCapital()?.getCenterTile() else null
        if (capitalTile != null) {
            val distance = combatTile.aerialDistanceTo(capitalTile)
            unit.forEachMatchingUnique(UniqueType.StrengthNearCapital, conditionalState, checkCivInfoUniques = true) { unique ->
                val effect = unique.params[0].toInt() - 3 * distance
                if (effect > 0) sum += effect
            }
        }

        @Readonly
        fun maxMalusParamFromUnit(adjUnit: com.unciv.logic.map.mapunit.MapUnit, currentBest: String?): String? {
            if (!adjUnit.civ.isAtWarWith(civInfo)) return currentBest
            var best = currentBest
            for (u in adjUnit.getMatchingUniques(UniqueType.StrengthForAdjacentEnemies)) {
                if (combatant.matchesFilter(u.params[1]) && combatTile.matchesFilter(u.params[2])) {
                    val p0 = u.params[0]
                    if (best == null || p0 > best) best = p0
                }
            }
            return best
        }

        var bestMalusParam: String? = null
        val enemyTile = enemy.getTile()
        var enemyIsNeighbor = false
        var attackFromIsNeighbor = false

        for (nb in combatTile.neighbors) {
            if (nb == enemyTile) enemyIsNeighbor = true
            if (nb == tileToAttackFrom) attackFromIsNeighbor = true
            for (u in nb.getUnits()) bestMalusParam = maxMalusParamFromUnit(u, bestMalusParam)
        }
        if (!enemyIsNeighbor && attackFromIsNeighbor && enemy is MapUnitCombatant) {
            bestMalusParam = maxMalusParamFromUnit(enemy.unit, bestMalusParam)
        }
        if (bestMalusParam != null) sum += bestMalusParam.toInt()

        return sum
    }

    @Readonly
    private fun getTerrainAttackModifierSum(
        attacker: MapUnitCombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile
    ): Int {
        var sum = 0
        val unit = attacker.unit
        val defenderTile = defender.getTile()
        val attackerTile = attacker.getTile()
        val noCoastPenalty = unit.hasUnique(UniqueType.AttackAcrossCoast)

        var hasLanding = false
        if (unit.isEmbarked() && defenderTile.isLand && !noCoastPenalty) {
            hasLanding = true
        }
        if (unit.type.isLandUnit() && !attackerTile.isWater && attacker.isMelee() && defenderTile.isWater && !noCoastPenalty) {
            sum += BattleConstants.BOARDING_MALUS
        }
        if (!unit.type.isAirUnit() && attacker.isMelee() && attackerTile.isWater && !defenderTile.isWater && !noCoastPenalty && !defender.isCity()) {
            hasLanding = true
        }
        if (hasLanding) sum += BattleConstants.LANDING_MALUS

        if (isMeleeAttackingAcrossRiverWithNoBridge(attacker, tileToAttackFrom, defender)) {
            sum += BattleConstants.ATTACKING_ACROSS_RIVER_MALUS
        }
        return sum
    }

    @Readonly
    fun getAttackModifierSum(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile
    ): Int {
        var sum = getGeneralModifierSum(attacker, defender, CombatAction.Attack, tileToAttackFrom)

        if (attacker is MapUnitCombatant) {
            sum += getTerrainAttackModifierSum(attacker, defender, tileToAttackFrom)

            if (attacker.unit.isPreparingAirSweep()) {
                attacker.unit.forEachMatchingUnique(UniqueType.StrengthWhenAirsweep) { unique ->
                    sum += unique.params[0].toInt()
                }
            }

            if (attacker.isMelee()) {
                val attackerCiv = attacker.getCivInfo()
                var flankers = 0
                for (nb in defender.getTile().neighbors) {
                    val mu = nb.militaryUnit
                    if (mu != null && mu != attacker.unit && mu.civ == attackerCiv && !mu.baseUnit.isRanged()) {
                        flankers++
                    }
                }
                if (flankers > 0) {
                    var flankingBonus = BattleConstants.BASE_FLANKING_BONUS
                    attacker.unit.forEachMatchingUnique(
                        UniqueType.FlankAttackBonus,
                        checkCivInfoUniques = true,
                        gameContext = getGameContext(CombatAction.Attack, attacker, defender)
                    ) { unique ->
                        flankingBonus *= unique.params[0].toPercent()
                    }
                    sum += (flankingBonus * flankers).toInt()
                }
            }
        }

        return sum
    }

    @Readonly
    fun getDefenceModifierSum(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile
    ): Int {
        var sum = getGeneralModifierSum(defender, attacker, CombatAction.Defend, tileToAttackFrom)

        if (defender is MapUnitCombatant && !defender.unit.isEmbarked()) {
            val tileDefenceBonus = defender.getTile().getDefensiveBonus(unit = defender.unit)
            if (!defender.unit.hasUnique(UniqueType.NoDefensiveTerrainBonus, checkCivInfoUniques = true) && tileDefenceBonus > 0
                || !defender.unit.hasUnique(UniqueType.NoDefensiveTerrainPenalty, checkCivInfoUniques = true) && tileDefenceBonus < 0
            ) {
                sum += (tileDefenceBonus * 100).toInt()
            }

            if (defender.unit.isFortified() || defender.unit.isGuarding()) {
                sum += BattleConstants.FORTIFICATION_BONUS * defender.unit.getFortificationTurns()
            }
        }

        return sum
    }

    /**
     * Includes attack modifiers (zero-allocation numeric fast path — exact parity with [getAttackModifiers])
     */
    @Readonly
    fun getAttackingStrength(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile
    ): Float {
        val attackModifier = 1f + getAttackModifierSum(attacker, defender, tileToAttackFrom) / 100f
        return max(1f, attacker.getAttackingStrength(defender) * attackModifier)
    }


    /**
     * Includes defence modifiers (zero-allocation numeric fast path — exact parity with [getDefenceModifiers])
     */
    @Readonly
    fun getDefendingStrength(attacker: ICombatant, defender: ICombatant, tileToAttackFrom: Tile): Float {
        val defenceModifier = 1f + getDefenceModifierSum(attacker, defender, tileToAttackFrom) / 100f
        return max(1f, defender.getDefendingStrength(attacker) * defenceModifier)
    }
    
    @Readonly
    fun getRandomness(combatant: ICombatant): Float = 
        Random(combatant.getCivInfo().gameInfo.turns
                * combatant.getTile().position.toVector2().hashCode().toLong()).nextFloat()

    @Readonly
    fun calculateDamageToAttacker(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile = defender.getTile(),
        /** Between 0 and 1. */
        randomnessFactor: Float = getRandomness(attacker)
    ): Int {
        if (attacker.isRanged() && !attacker.isAirUnit()) return 0
        if (defender.isCivilian()) return 0
        val ratio = getAttackingStrength(attacker, defender, tileToAttackFrom) / getDefendingStrength(
                attacker, defender, tileToAttackFrom)
        return (damageModifier(ratio, true, randomnessFactor) * getHealthDependantDamageRatio(defender)).roundToInt()
    }

    @Readonly
    fun calculateDamageToDefender(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile = defender.getTile(),
        /** Between 0 and 1.  Defaults to turn and location-based random to avoid save scumming */
        randomnessFactor: Float = getRandomness(defender)
        ,
    ): Int {
        if (defender.isCivilian()) return BattleConstants.DAMAGE_TO_CIVILIAN_UNIT
        val ratio = getAttackingStrength(attacker, defender, tileToAttackFrom) /
                getDefendingStrength(attacker, defender, tileToAttackFrom)
        return (damageModifier(ratio, false, randomnessFactor) * getHealthDependantDamageRatio(attacker)).roundToInt()
    }

    /**
     * Computes `(damageToAttacker, damageToDefender)` in a single pass, sharing the
     * strength ratio calculation between both sides.
     */
    @Readonly
    fun calculateBothDamages(
        attacker: ICombatant,
        defender: ICombatant,
        tileToAttackFrom: Tile = defender.getTile(),
        randomnessToAttacker: Float = getRandomness(attacker),
        randomnessToDefender: Float = getRandomness(defender)
    ): Pair<Int, Int> {
        if (defender.isCivilian()) return 0 to BattleConstants.DAMAGE_TO_CIVILIAN_UNIT
        val ratio = getAttackingStrength(attacker, defender, tileToAttackFrom) /
                getDefendingStrength(attacker, defender, tileToAttackFrom)
        val dmgToAtk = if (attacker.isRanged() && !attacker.isAirUnit()) 0
            else (damageModifier(ratio, true, randomnessToAttacker) * getHealthDependantDamageRatio(defender)).roundToInt()
        val dmgToDef = (damageModifier(ratio, false, randomnessToDefender) * getHealthDependantDamageRatio(attacker)).roundToInt()
        return dmgToAtk to dmgToDef
    }

    /** (max, min) bonus damage dealt to [defender] from an additional [UniqueType.ExtraRangedAttack] - `(0, 0)` if not applicable. */
    @Readonly
    fun getExtraRangedAttackBonusDamage(attacker: MapUnitCombatant, defender: ICombatant, tileToAttackFrom: Tile): Pair<Int, Int> {
        var maxExtra = 0
        var minExtra = 0
        for (fakeAttacker in Battle.getExtraRangedAttackFakeUnits(attacker)) {
            maxExtra += calculateDamageToDefender(fakeAttacker, defender, tileToAttackFrom, 1f)
            minExtra += calculateDamageToDefender(fakeAttacker, defender, tileToAttackFrom, 0f)
        }
        return maxExtra to minExtra
    }

    @Pure
    private fun damageModifier(
        attackerToDefenderRatio: Float,
        damageToAttacker: Boolean,
        /** Between 0 and 1. */
        randomnessFactor: Float,
    ): Float {
        // https://forums.civfanatics.com/threads/getting-the-combat-damage-math.646582/#post-15468029
        val strongerToWeakerRatio =
            attackerToDefenderRatio.pow(if (attackerToDefenderRatio < 1) -1 else 1)
        var ratioModifier = (((strongerToWeakerRatio + 3) / 4).pow(4) + 1) / 2
        if (damageToAttacker && attackerToDefenderRatio > 1 || !damageToAttacker && attackerToDefenderRatio < 1) // damage ratio from the weaker party is inverted
            ratioModifier = ratioModifier.pow(-1)
        val randomCenteredAround30 = 24 + 12 * randomnessFactor
        return randomCenteredAround30 * ratioModifier
    }
}
enum class CombatAction {
    Attack,
    Defend,
    Intercept,
}
