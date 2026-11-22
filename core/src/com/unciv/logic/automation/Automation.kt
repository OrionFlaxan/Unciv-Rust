package com.unciv.logic.automation

import com.unciv.view.TileView
import com.unciv.view.GameView
import com.unciv.view.CivView
import com.unciv.view.ForeignCivView
import com.unciv.logic.automation.Timers.Companion.timeThis
import com.unciv.logic.city.City
import com.unciv.logic.city.CityFocus
import com.unciv.logic.city.CityStats
import com.unciv.logic.civilization.Civilization
import com.unciv.logic.map.BFS
import com.unciv.logic.map.TileMap
import com.unciv.logic.map.tile.Tile
import com.unciv.models.ruleset.Building
import com.unciv.models.ruleset.INonPerpetualConstruction
import com.unciv.models.ruleset.IConstruction
import com.unciv.models.ruleset.PerpetualConstruction
import com.unciv.models.ruleset.tile.ResourceType
import com.unciv.models.ruleset.tile.TileImprovement
import com.unciv.models.ruleset.unique.GameContext
import com.unciv.models.ruleset.unique.Unique
import com.unciv.models.ruleset.unique.UniqueType
import com.unciv.models.ruleset.unit.BaseUnit
import com.unciv.models.stats.Stat
import com.unciv.models.stats.Stats
import com.unciv.ui.screens.victoryscreen.RankingType
import yairm210.purity.annotations.LocalState
import yairm210.purity.annotations.Readonly
import kotlin.math.min


object Automation {

    @Readonly
    fun rankTileForCityWork(tile: Tile, city: City): Float {
        val stats = tile.stats.getTileStats(city, city.civ)
        return rankStatsForCityWork(stats, city, false)
    }

    @Readonly
    fun rankSpecialist(specialist: String, city: City): Float {
        val stats = city.cityStats.getStatsOfSpecialist(specialist)
        var rank = rankStatsForCityWork(stats, city, true)
        // derive GPP score
        var gpp = 0f
        if (city.getRuleset().specialists.containsKey(specialist)) { // To solve problems in total remake mods
            val specialistInfo = city.getRuleset().specialists[specialist]!!
            gpp = specialistInfo.greatPersonPoints.sumValues().toFloat()
        }
        gpp = gpp * (100 + city.currentGPPBonus) / 100
        rank += gpp // GPP weight
        return rank
    }

    // More complicated logic to properly weigh Food vs other Stats (esp Production)
    @Readonly
    private fun getFoodModWeight(city: City, surplusFood: Float): Float {
        val speed = city.civ.gameInfo.speed.modifier
        // Zero out Growth if close to Unhappiness limit
        if (city.civ.getHappiness() < -8)
            return 0f
        if (city.civ.isAI()) {
            // Value 1 Growth at less than 2 Production, but above 1 Production + 1 Gold
            return 1.5f
        }
        // Human weights. May be different since AI Happiness is always "easier"
        // Only apply these for Default to not interfere with Focus weights
        if (city.getCityFocus() == CityFocus.NoFocus) {
            if (city.population.population < 5)
                return 2f
            if (surplusFood > city.population.getFoodToNextPopulation() / (10 * speed))
                return 0.75f // get Growth just under Production
        }

        return 1f
    }

    /** Precomputed city-level invariants hoisted out of `CityPopulationManager.autoAssignPopulation`. */
    internal class CityWorkRankContext(
        val cityAIFocus: CityFocus,
        val cityStatsObj: CityStats,
        val allTechsAreResearched: Boolean,
        val currentConstruction: IConstruction,
        val isPerpetual: Boolean,
        val avoidGrowth: Boolean,
        val isSmallCity: Boolean,
        val civHappiness: Int,
        val civGoldNegative: Boolean,
        val isAI: Boolean,
        val popCount: Int,
        val foodThresholdForHumanWeight: Float,
        val growthNullifyingUnique: Boolean,
        val growthBonusMultiplier: Float,
        val specialistHappinessDelta: Float
    ) {
        @Readonly
        fun foodModWeight(surplusFood: Float): Float {
            if (civHappiness < -8) return 0f
            if (isAI) return 1.5f
            if (cityAIFocus == CityFocus.NoFocus) {
                if (popCount < 5) return 2f
                if (surplusFood > foodThresholdForHumanWeight) return 0.75f
            }
            return 1f
        }
    }

    @Readonly
    internal fun createCityWorkRankContext(city: City): CityWorkRankContext {
        val cityAIFocus = city.getCityFocus()
        val civInfo = city.civ
        val currentConstruction = city.cityConstructions.getCurrentConstruction()
        val civHappiness = civInfo.getHappiness()
        val isAI = civInfo.isAI()
        val popCount = city.population.population
        val speed = civInfo.gameInfo.speed.modifier
        val foodThreshold =
            if (!isAI && cityAIFocus == CityFocus.NoFocus && popCount >= 5)
                city.population.getFoodToNextPopulation() / (10f * speed)
            else Float.POSITIVE_INFINITY

        var nullifies = false
        city.forEachMatchingUnique(UniqueType.NullifiesGrowth, city.state) { _: Unique ->
            nullifies = true
        }

        var growthMult = 1f
        if (!nullifies) {
            city.forEachMatchingUnique(UniqueType.GrowthPercentBonus, city.state) { unique: Unique ->
                if (city.matchesFilter(unique.params[1])) {
                    growthMult += unique.params[0].toFloat() / 100f
                }
            }
            if (city.isWeLoveTheKingDayActive() && civHappiness >= 0) {
                growthMult += 0.25f
            }
        }

        var specHapDelta = 0f
        city.forEachMatchingUnique(UniqueType.UnhappinessFromPopulationTypePercentageChange, city.state) { unique: Unique ->
            if (unique.params[1] == "Specialists" && city.matchesFilter(unique.params[2])) {
                specHapDelta += unique.params[0].toFloat() / 100f
            }
        }

        return CityWorkRankContext(
            cityAIFocus = cityAIFocus,
            cityStatsObj = city.cityStats,
            allTechsAreResearched = civInfo.tech.allTechsAreResearched(),
            currentConstruction = currentConstruction,
            isPerpetual = currentConstruction is PerpetualConstruction,
            avoidGrowth = city.avoidGrowth,
            isSmallCity = popCount < 10,
            civHappiness = civHappiness,
            civGoldNegative = civInfo.stats.statsForNextTurn.gold < 0f,
            isAI = isAI,
            popCount = popCount,
            foodThresholdForHumanWeight = foodThreshold,
            growthNullifyingUnique = nullifies,
            growthBonusMultiplier = growthMult,
            specialistHappinessDelta = specHapDelta
        )
    }

    @Readonly
    internal fun rankStatsForCityWorkFast(
        stats: Stats,
        ctx: CityWorkRankContext,
        surplusFood: Float,
        areWeRankingSpecialist: Boolean
    ): Float {
        var food = stats.food
        var prod = stats.production
        var gold = stats.gold
        var sci = stats.science
        var cult = stats.culture
        var hap = stats.happiness
        var faith = stats.faith

        if (areWeRankingSpecialist) {
            hap -= ctx.specialistHappinessDelta
            if (sci == 3f || sci >= 5f) sci *= 1.3f
        }

        if (ctx.cityStatsObj.canConvertFoodToProduction(surplusFood, ctx.currentConstruction)) {
            prod += ctx.cityStatsObj.getProductionFromExcessiveFood(surplusFood + food) -
                ctx.cityStatsObj.getProductionFromExcessiveFood(surplusFood)
            food = 0f
        }

        var feedFood = 0f
        if (surplusFood < 0f) feedFood = min(food, -surplusFood).coerceAtLeast(0f)
        var growthFood = food - feedFood
        if (ctx.avoidGrowth) growthFood = 0f

        // Exact Stats.applyRankingWeights()
        val foodBaseWeight = 14f
        prod *= 12.01f
        gold *= 6f
        sci *= 9.01f
        cult *= 8f
        hap *= 10f
        faith *= 7f

        var weightedFood = feedFood * (foodBaseWeight * 8f)
        if (!ctx.growthNullifyingUnique) {
            val newGrowthFood = (growthFood * ctx.growthBonusMultiplier).coerceAtLeast(0f)
            weightedFood += newGrowthFood * foodBaseWeight * ctx.foodModWeight(surplusFood)
        }

        if (ctx.isSmallCity) sci /= 2f
        if (ctx.civGoldNegative) gold *= 2f
        if (ctx.civHappiness < 5) hap *= 2f
        if (ctx.allTechsAreResearched) sci = 0f
        if (ctx.isPerpetual) prod /= 6f

        val focus = ctx.cityAIFocus
        for (stat in focus.statValuesForFocus) {
            val mult = focus.getStatMultiplier(stat)
            when (stat) {
                Stat.Food -> if (weightedFood != 0f) weightedFood *= mult
                Stat.Production -> if (prod != 0f) prod *= mult
                Stat.Gold -> if (gold != 0f) gold *= mult
                Stat.Science -> if (sci != 0f) sci *= mult
                Stat.Culture -> if (cult != 0f) cult *= mult
                Stat.Happiness -> if (hap != 0f) hap *= mult
                Stat.Faith -> if (faith != 0f) faith *= mult
            }
        }

        return weightedFood + prod + gold + sci + cult + hap + faith
    }

    @Readonly
    fun rankStatsForCityWork(stats: Stats, city: City, areWeRankingSpecialist: Boolean): Float
        = timeThis("Automation.rankStatsForCityWork") {
        val ctx = createCityWorkRankContext(city)
        rankStatsForCityWorkFast(stats, ctx, city.cityStats.currentCityStats.food, areWeRankingSpecialist)
    }

    fun tryTrainMilitaryUnit(city: City) {
        if (city.isPuppet) return
        if ((city.cityConstructions.getCurrentConstruction() as? BaseUnit)?.isMilitary == true)
            return // already training a military unit
        val chosenUnit = chooseMilitaryUnit(city, city.civ.gameInfo.ruleset.units.values.asSequence())
        if (chosenUnit != null)
            city.cityConstructions.setCurrentConstruction(chosenUnit.name)
    }

    @Readonly
    fun chooseMilitaryUnit(city: City, availableUnits: Sequence<BaseUnit>): BaseUnit? {
        val rng = city.state.stateBasedRandom("Automation.chooseMilitaryUnit")
        val currentChoice = city.cityConstructions.getCurrentConstruction()
        if (currentChoice is BaseUnit && !currentChoice.isCivilian()) return currentChoice

        // if not coastal, removeShips == true so don't even consider ships
        var removeShips = true
        var isMissingNavalUnitsForCityDefence = false

        @Readonly fun isNavalMeleeUnit(unit: BaseUnit) = unit.isMelee() && unit.type.isWaterUnit()
        if (city.isCoastal()) {
            // in the future this could be simplified by assigning every distinct non-lake body of
            // water their own ID like a continent ID
            val findWaterConnectedCitiesAndEnemies =
                    BFS(city.getCenterTile()) { it.isWater && !it.isImpassible() || it.isCityCenter() }
            findWaterConnectedCitiesAndEnemies.stepToEnd()

            val numberOfOurConnectedCities = findWaterConnectedCitiesAndEnemies.getReachedTiles()
                .count { it.isCityCenter() && it.getOwner() == city.civ }
            val numberOfOurNavalMeleeUnits = findWaterConnectedCitiesAndEnemies.getReachedTiles()
                .sumOf { it.getUnits().count { it.civ == city.civ && isNavalMeleeUnit(it.baseUnit) } }
                
            isMissingNavalUnitsForCityDefence = numberOfOurConnectedCities > numberOfOurNavalMeleeUnits

            removeShips = findWaterConnectedCitiesAndEnemies.getReachedTiles().none {
                        (it.isCityCenter() && it.getOwner() != city.civ)
                                || (it.militaryUnit != null && it.militaryUnit!!.civ != city.civ)
                    } // there is absolutely no reason for you to make water units on this body of water.
        }

        val militaryUnits = availableUnits
            .filter { it.isMilitary }
            .filterNot { removeShips && it.isWaterUnit }
            .filter { allowSpendingResource(GameView(city.civ.gameInfo, city.civ).civView, it) }
            .filterNot {
                // units that can't attack may be very strong meatshields, but current AI
                // has trouble using them properly, and this conveniently filters out aircraft carriers
                // (they're kinda useless compared to investing in battleships or extended-range bombers,
                // and are obsoleted at Stealth. Excluding units based on their carrying slots being
                // not needed filters out nuclear submarines and missile cruisers, which is not correct)
                it.hasUnique(UniqueType.CannotAttack)
            }
            // Only now do we filter out the constructable units because that's a heavier check
            .filter { it.isBuildable(city.cityConstructions) }
            .toList().asSequence()

        val chosenUnit: BaseUnit
        if (!city.civ.isAtWar()
                && city.civ.cities.any { it.getCenterTile().militaryUnit == null }
                && militaryUnits.any { it.isRanged() } // this is for city defence so get a ranged unit if we can
        ) {
            chosenUnit = militaryUnits
                .filter { it.isRanged() }
                .maxByOrNull { it.cost }!!
        }
        else if (isMissingNavalUnitsForCityDefence && militaryUnits.any { isNavalMeleeUnit(it) }) {
            chosenUnit = militaryUnits
                .filter { isNavalMeleeUnit(it) }
                .maxBy { it.cost }
        }
        else { // randomize type of unit and take the strongest of its kind
            val bestUnitsForType = hashMapOf<String, BaseUnit>()
            for (unit in militaryUnits) {
                if (bestUnitsForType[unit.unitType] == null
                        || bestUnitsForType[unit.unitType]!!.getForceEvaluation() < unit.getForceEvaluation()) {
                    bestUnitsForType[unit.unitType] = unit
                }
            }
            // Check the maximum force evaluation for the shortlist so we can prune useless ones (ie scouts)
            val bestForce = bestUnitsForType.maxOfOrNull { it.value.getForceEvaluation() } ?: return null
            chosenUnit = bestUnitsForType.filterValues { it.uniqueTo != null || it.getForceEvaluation() > bestForce / 3 }.values.random(rng)
        }
        return chosenUnit
    }

    /** Determines whether [civInfo] should be allocating military to fending off barbarians */
    fun afraidOfBarbarians(civView: CivView): Boolean {
        val civInfo = civView.getCiv()
        if (civInfo.isCityState || civInfo.isBarbarian)
            return false

        if (civInfo.gameInfo.gameParameters.noBarbarians)
            return false // If there are no barbarians we are not afraid

        val speed = civInfo.gameInfo.speed
        if (civInfo.gameInfo.turns > 200 * speed.barbarianModifier)
            return false // Very late in the game we are not afraid

        var multiplier = if (civInfo.gameInfo.gameParameters.ragingBarbarians) 1.3f
        else 1f // We're slightly more afraid of raging barbs

        // Past the early game we are less afraid
        if (civInfo.gameInfo.turns > 120 * speed.barbarianModifier * multiplier)
            multiplier /= 2

        // If we have no cities or a lot of units we are not afraid
        if (civInfo.cities.isEmpty() || civInfo.units.getCivUnits().count() >= 4 * multiplier)
            return false

        // If we have vision of our entire starting continent (ish) we are not afraid
        civInfo.gameInfo.tileMap.assignContinents(TileMap.AssignContinentsMode.Ensure)
        val startingContinent = civInfo.getCapital()!!.getCenterTile().getContinent()
        val startingContinentSize = civInfo.gameInfo.tileMap.continentSizes[startingContinent]
        if (startingContinentSize != null && startingContinentSize < civInfo.viewableTiles.size * multiplier)
            return false

        // Otherwise we're afraid
        return true
    }

    /** Checks resource scarcity making a construction undesirable. */
    @Readonly
    fun allowAutomatedConstruction(
        civView: CivView,
        city: City,
        construction: INonPerpetualConstruction
    ): Boolean {
        return allowSpendingResource(civView, construction, city)
    }


    /** Determines whether the AI should be willing to spend strategic resources to build
     *  [construction] for [civInfo], assumes that we are actually able to do so. */
    @Readonly
    fun allowSpendingResource(civView: CivView, construction: INonPerpetualConstruction, cityInfo: City? = null): Boolean {
        val civInfo = civView.getCiv()
        // City states do whatever they want
        if (civInfo.isCityState)
            return true

        // Spaceships are always allowed
        if (construction.name in civInfo.gameInfo.spaceResources)
            return true

        val requiredResources = if (construction is BaseUnit)
            construction.getResourceRequirementsPerTurn(civInfo.state)
        else construction.getResourceRequirementsPerTurn(cityInfo?.state ?: civInfo.state)
        // Does it even require any resources?
        if (requiredResources.isEmpty())
            return true

        val civResources = civInfo.getCivResourcesByName()

        // Rule of thumb: reserve 2-3 for spaceship, then reserve half each for buildings and units
        // Assume that no buildings provide any resources
        for ((resource, amount) in requiredResources) {

            // Also count things under construction
            var futureForUnits = 0
            var futureForBuildings = 0

            for (city in civInfo.cities) {
                val otherConstruction = city.cityConstructions.getCurrentConstruction()
                if (otherConstruction is Building)
                    futureForBuildings += otherConstruction.getResourceRequirementsPerTurn(city.state)[resource]
                else
                    futureForUnits += otherConstruction.getResourceRequirementsPerTurn(civInfo.state)[resource]
            }

            // Make sure we have some for space
            if (resource in civInfo.gameInfo.spaceResources && civResources[resource]!! - amount - futureForBuildings - futureForUnits < 2) {
                return false
            }

            // Assume buildings remain useful
            val neededForBuilding = civInfo.cache.lastEraResourceUsedForBuilding[resource] != null
            // Don't care about old units
            val neededForUnits = civInfo.cache.lastEraResourceUsedForUnit[resource] != null
                    && civInfo.cache.lastEraResourceUsedForUnit[resource]!! >= civInfo.getEraNumber()

            // No need to save for both
            if (!neededForBuilding || !neededForUnits) {
                continue
            }

            val usedForUnits = civInfo.detailedCivResources.filter { it.resource.name == resource && it.origin == "Units" }.sumOf { -it.amount }
            val usedForBuildings = civInfo.detailedCivResources.filter { it.resource.name == resource && it.origin == "Buildings" }.sumOf { -it.amount }

            if (construction is Building) {
                // Will more than half the total resources be used for buildings after this construction?
                if (civResources[resource]!! + usedForUnits < usedForBuildings + amount + futureForBuildings) {
                    return false
                }
            } else {
                // Will more than half the total resources be used for units after this construction?
                if (civResources[resource]!! + usedForBuildings < usedForUnits + amount + futureForUnits) {
                    return false
                }
            }
        }
        return true
    }

    @Readonly
    fun threatAssessment(assessorView: CivView, assessedView: ForeignCivView): ThreatLevel {
        val assessor = assessorView.getCiv()
        val assessed = assessedView.getCiv()
        val powerLevelComparison =
            assessed.getStatForRanking(RankingType.Force) / assessor.getStatForRanking(RankingType.Force).toFloat()
        return when {
            powerLevelComparison > 2 -> ThreatLevel.VeryHigh
            powerLevelComparison > 1.5f -> ThreatLevel.High
            powerLevelComparison < 0.5f -> ThreatLevel.VeryLow
            powerLevelComparison < (1 / 1.5f) -> ThreatLevel.Low
            else -> ThreatLevel.Medium
        }
    }
    @Readonly
    private fun improvementIsRemovable(city: City, tile: Tile): Boolean {
        val gameContext = GameContext(city.civ, city, tile = tile)
        val improvement = tile.tileImprovement ?: return false
        return !improvement.hasUnique(UniqueType.AutomatedUnitsWillNotReplace, gameContext) &&
            !improvement.hasUnique(UniqueType.Irremovable, gameContext)
    }

    /** Support [UniqueType.CreatesOneImprovement] unique - find best tile for placement automation */
    @Readonly
    fun getTileForConstructionImprovement(city: City, improvement: TileImprovement): Tile? {
        val civ = city.civ
        val civView = GameView(civ.gameInfo, civ).civView
        return city.getTiles().filter {
            (it.tileImprovement == null || improvementIsRemovable(city, it))
                && it.improvementFunctions.canBuildImprovement(improvement, city.state)
        }.maxByOrNull { 
            // Needs to take into account future improvement layouts, and better placement of citadel-like improvements
            rankStatsValue(it.stats.getStatDiffForImprovement(improvement, civ, city, it.stats.getTileStats(city, civ)), civView) + (
                if (improvement.hasUnique(UniqueType.DefensiveBonus)) { 
                    it.aerialDistanceTo(city.getCenterTile()) + it.getDefensiveBonus(false) } 
                else 0).toFloat()
        }
    }

    // Ranks a tile for any purpose except the expansion algorithm of cities
    @Readonly
    internal fun rankTile(tile: TileView, civView: CivView): Float {
        val tileOwner = tile.getOwner()
        if (tileOwner != null && tileOwner != civView) return 0f // Already belongs to another civilization, useless to us
        val stats = tile.getTileStats(civView)
        var rank = rankStatsValue(stats, civView)
        if (tile.improvement == null) rank += 0.5f // improvement potential!
        val isPillaged = tile.improvementIsPillaged || tile.roadIsPillaged
        if (isPillaged) rank += 0.6f
        val resource = tile.getViewableResource(civView)
        if (resource != null) {
            if (resource.resourceType != ResourceType.Bonus) rank += 1f // for usage
            if (tile.improvement == null) rank += 1f // improvement potential - resources give lots when improved!
            if (isPillaged) rank += 1.1f // even better, repair is faster
        }
        return rank
    }

    @Readonly
    fun rankStatsValue(stats: Stats, civView: CivView): Float {
        val civInfo = civView.getCiv()
        var rank = 0.0f
        rank += stats.food * 1.2f //food get more value to keep city growing

        rank += if (civInfo.gold < 0 && civInfo.stats.statsForNextTurn.gold <= 0)
                    stats.gold //build more gold infrastructure if in serious gold problems
        // This could lead to oscilliatory behaviour however: gold problem -> build trade post -> no gold problem -> replace trade posts -> gold problem
                else
                    stats.gold / 3 // Gold is valued less than is the case for citizen assignment,
        //otherwise the AI would replace tiles with trade posts upon entering a golden age,
        //and replace the trade post again when the golden age ends.
        // We need a way to take golden age gold into account before the GA actually takes place
        rank += stats.happiness
        rank += stats.production
        rank += stats.science
        rank += stats.culture
        rank += stats.faith
        return rank
    }
}

enum class ThreatLevel {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh
}
