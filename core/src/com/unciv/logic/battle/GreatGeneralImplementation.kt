package com.unciv.logic.battle

import com.unciv.logic.map.mapunit.MapUnit
import com.unciv.models.ruleset.unique.GameContext
import com.unciv.models.ruleset.unique.Unique
import com.unciv.models.ruleset.unique.UniqueType
import yairm210.purity.annotations.Readonly


object GreatGeneralImplementation {

    private data class GeneralBonusData(val general: MapUnit, val radius: Int, val filter: String, val bonus: Int) {
        constructor(general: MapUnit, unique: Unique) : this(
            general,
            radius = unique.params[2].toIntOrNull() ?: 0,
            filter = unique.params[1],
            bonus = unique.params[0].toIntOrNull() ?: 0
        )
    }

    /**
     * Determine the "Great General" bonus for [ourUnitCombatant] by searching for units carrying the [UniqueType.StrengthBonusInRadius] in the vicinity.
     *
     * Used by [BattleDamage.getGeneralModifiers].
     *
     * @return A pair of unit's name and bonus (percentage) as Int (typically 15), or 0 if no applicable Great General equivalents found
     */
    @Readonly
    fun getGreatGeneralBonus(
        ourUnitCombatant: MapUnitCombatant,
        enemy: ICombatant,
        combatAction: CombatAction
    ): Pair<String, Int> {
        val unit = ourUnitCombatant.unit
        val civInfo = ourUnitCombatant.unit.civ
        val allGenerals = civInfo.units.getCivUnits()
            .filter { it.cache.hasStrengthBonusInRadiusUnique }
        if (allGenerals.none()) return Pair("", 0)

        val greatGeneral = allGenerals
            .flatMap { general ->
                general.getMatchingUniques(UniqueType.StrengthBonusInRadius,
                    GameContext(unit.civ, ourCombatant = ourUnitCombatant, theirCombatant = enemy, combatAction = combatAction))
                    .map { GeneralBonusData(general, it) }
            }.filter {
                // Support the border case when a mod unit has several
                // GreatGeneralAura uniques (e.g. +50% as radius 1, +25% at radius 2, +5% at radius 3)
                // The "Military" test is also supported deep down in unit.matchesFilter, a small
                // optimization for the most common case, as this function is only called for `MapUnitCombatant`s
                it.general.currentTile.aerialDistanceTo(unit.getTile()) <= it.radius
                        && (it.filter == "Military" || unit.matchesFilter(it.filter, state = it.general.cache.state))
            }
        val greatGeneralModifier = greatGeneral.maxByOrNull { it.bonus } ?: return Pair("",0)

        if (unit.hasUnique(UniqueType.GreatGeneralProvidesDoubleCombatBonus, checkCivInfoUniques = true)
            && greatGeneralModifier.general.isGreatPersonOfType("War")) // apply only on "true" generals
            return Pair(greatGeneralModifier.general.name, greatGeneralModifier.bonus * 2)
        return Pair(greatGeneralModifier.general.name, greatGeneralModifier.bonus)
    }

    /**
     * Zero-allocation numeric fast path equivalent to [getGreatGeneralBonus]`.second`,
     * used by [BattleDamage.getGeneralModifierSum] to avoid intermediate Sequence/Pair allocations.
     */
    @Readonly
    fun getGreatGeneralBonusInt(
        ourUnitCombatant: MapUnitCombatant,
        enemy: ICombatant,
        combatAction: CombatAction
    ): Int {
        val unit = ourUnitCombatant.unit
        val civInfo = unit.civ
        val unitTile = unit.getTile()
        var bestBonus = Int.MIN_VALUE
        var bestGeneral: MapUnit? = null
        var ctx: GameContext? = null

        for (general in civInfo.units.getCivUnits()) {
            if (!general.cache.hasStrengthBonusInRadiusUnique) continue
            val gameContext = ctx ?: GameContext(
                civInfo,
                ourCombatant = ourUnitCombatant,
                theirCombatant = enemy,
                combatAction = combatAction
            ).also { ctx = it }
            val dist = general.currentTile.aerialDistanceTo(unitTile)
            for (unique in general.getMatchingUniques(UniqueType.StrengthBonusInRadius, gameContext)) {
                val radius = unique.params[2].toIntOrNull() ?: 0
                if (dist > radius) continue
                val filter = unique.params[1]
                if (filter != "Military" && !unit.matchesFilter(filter, state = general.cache.state)) continue
                val bonus = unique.params[0].toIntOrNull() ?: 0
                if (bonus > bestBonus) {
                    bestBonus = bonus
                    bestGeneral = general
                }
            }
        }
        val chosen = bestGeneral ?: return 0
        if (unit.hasUnique(UniqueType.GreatGeneralProvidesDoubleCombatBonus, checkCivInfoUniques = true)
            && chosen.isGreatPersonOfType("War")
        ) return bestBonus * 2
        return bestBonus
    }
}
