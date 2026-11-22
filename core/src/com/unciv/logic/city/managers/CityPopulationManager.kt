package com.unciv.logic.city.managers

import com.unciv.logic.IsPartOfGameInfoSerialization
import com.unciv.logic.automation.Automation
import com.unciv.logic.automation.Timers.Companion.timeThis
import com.unciv.logic.city.City
import com.unciv.logic.civilization.NotificationCategory
import com.unciv.logic.civilization.NotificationIcon
import com.unciv.logic.map.tile.Tile
import com.unciv.models.Counter
import com.unciv.models.ruleset.unique.UniqueType
import com.unciv.models.stats.Stats
import com.unciv.ui.components.extensions.toPercent
import com.unciv.utils.withItem
import com.unciv.utils.withoutItem
import yairm210.purity.annotations.Readonly
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.pow

class CityPopulationManager : IsPartOfGameInfoSerialization {
    @Transient
    lateinit var city: City

    var population = 1
        private set
    var foodStored = 0

    val specialistAllocations = Counter<String>()

    @Readonly fun getNewSpecialists() = specialistAllocations 


    //region pure functions
    fun clone(): CityPopulationManager {
        val toReturn = CityPopulationManager()
        toReturn.specialistAllocations.add(specialistAllocations)
        toReturn.population = population
        toReturn.foodStored = foodStored
        return toReturn
    }

    @Readonly fun getNumberOfSpecialists() = getNewSpecialists().values.sum()

    @Readonly
    fun getFreePopulation(): Int {
        val workingPopulation = city.getWorkedTiles().count()
        return population - workingPopulation - getNumberOfSpecialists()
    }
    
    @Readonly
    fun getFoodToNextPopulation(): Int {
        // civ v math, civilization.wikia
        var foodRequired = 15 + 8 * (population - 1) + floor((population - 1).toDouble().pow(1.5))

        foodRequired *= city.civ.gameInfo.speed.modifier

        if (city.civ.isCityState)
            foodRequired *= 1.5f
        if (!city.civ.isHuman())
            foodRequired *= city.civ.gameInfo.getDifficulty().aiCityGrowthModifier
        return foodRequired.toInt()
    }

    /** Take null to mean infinity. */
    @Readonly
    fun getNumTurnsToStarvation(): Int? {
        if (!city.isStarving()) return null
        return foodStored / -city.foodForNextTurn() + 1
    }


    /** Take null to mean infinity. */
    @Readonly
    fun getNumTurnsToNewPopulation(): Int? {
        if (!city.isGrowing()) return null
        val roundedFoodPerTurn = city.foodForNextTurn().toFloat()
        val remainingFood = getFoodToNextPopulation() - foodStored
        var turnsToGrowth = ceil(remainingFood / roundedFoodPerTurn).toInt()
        if (turnsToGrowth < 1) turnsToGrowth = 1
        return turnsToGrowth
    }


    //endregion

    /** Implements [UniqueParameterType.PopulationFilter][com.unciv.models.ruleset.unique.UniqueParameterType.PopulationFilter] */
    @Readonly
    fun getPopulationFilterAmount(filter: String): Int {
        return when (filter) {
            "Specialists" -> getNumberOfSpecialists()
            "Population" -> population
            "Followers of the Majority Religion", "Followers of this Religion" -> city.religion.getFollowersOfMajorityReligion()
            "Unemployed" -> getFreePopulation()
            else -> specialistAllocations[filter]
        }
    }


    fun nextTurn(food: Int) {
        foodStored += food
        if (food < 0)
            city.civ.addNotification("[${city.name}] is starving!",
                city.location, NotificationCategory.Cities, NotificationIcon.Growth, NotificationIcon.Death)
        if (foodStored < 0) {        // starvation!
            if (population > 1) addPopulation(-1)
            foodStored = 0
        }
        val foodNeededToGrow = getFoodToNextPopulation()
        if (foodStored < foodNeededToGrow) return

        // What if the stores are already over foodNeededToGrow but NullifiesGrowth is in effect?
        // We could simply test food==0 - but this way NullifiesStat(food) will still allow growth:
        if (city.getMatchingUniques(UniqueType.NullifiesGrowth).any())
            return

        // Hard block growth when using Avoid Growth, cap stored food
        if (city.avoidGrowth) {
            foodStored = foodNeededToGrow
            return
        }

        // growth!
        foodStored -= foodNeededToGrow
        var percentOfFoodCarriedOver = 0
        city.forEachMatchingUnique(UniqueType.CarryOverFood) {
            if (city.matchesFilter(it.params[1]))
                percentOfFoodCarriedOver += it.params[0].toInt()
        }
        percentOfFoodCarriedOver = percentOfFoodCarriedOver.coerceAtMost(95)  // Try to avoid runaway food gain in mods, just in case
        foodStored += (foodNeededToGrow * percentOfFoodCarriedOver / 100f).toInt()
        addPopulation(1)
        city.shouldReassignPopulation = true
        city.civ.addNotification("[${city.name}] has grown!", city.location,
            NotificationCategory.Cities, NotificationIcon.Growth)
    }

    fun addPopulation(count: Int) {
        val changedAmount = count.coerceAtLeast(1 - population)
        population += changedAmount
        val freePopulation = getFreePopulation()
        if (freePopulation < 0) {
            unassignExtraPopulation()
            city.cityStats.update()
        } else {
            autoAssignPopulation()
        }

        if (city.civ.gameInfo.isReligionEnabled())
            city.religion.updatePressureOnPopulationChange(changedAmount)
    }

    fun setPopulation(count: Int) {
        addPopulation(-population + count)
    }

    /** Only assigns free population */
    internal fun autoAssignPopulation(): Unit = timeThis("CityPopulationManager.autoAssignPopulation") {
        val freePopulation = getFreePopulation()
        if (freePopulation <= 0) {
            city.cityStats.update()
            return
        }
        city.cityStats.update(updateCivStats = false)  // calculate current stats with current assignments; final update() updates civ stats

        val cityStats = city.cityStats.currentCityStats
        city.currentGPPBonus = city.getGreatPersonPercentageBonus()  // pre-calculate for use in Automation.rankSpecialist
        var specialistFoodBonus = 2f  // See CityStats.calcFoodEaten()
        specialistFoodBonus = 2f - specialistFoodBonus

        val rankCtx = Automation.createCityWorkRankContext(city)

        val candidateTiles = ArrayList<Tile>(36)
        val candidateStats = ArrayList<Stats>(36)
        for (t in city.getWorkableTiles()) {
            if (!t.isBlockaded() && !t.providesYield()) {
                candidateTiles.add(t)
                candidateStats.add(t.stats.getTileStats(city, city.civ))
            }
        }
        val tileCount = candidateTiles.size
        val tileWorked = BooleanArray(tileCount)

        val maxSpecialistsMap = if (city.manualSpecialists) null else getMaxSpecialists()
        val specNames: Array<String>?
        val specMax: IntArray?
        val specStats: Array<Stats>?
        val specGpp: FloatArray?
        if (maxSpecialistsMap != null && maxSpecialistsMap.isNotEmpty()) {
            val n = maxSpecialistsMap.size
            val rulesetSpecs = city.getRuleset().specialists
            val gppScale = (100 + city.currentGPPBonus) / 100f
            specNames = Array(n) { "" }
            specMax = IntArray(n)
            specStats = Array(n) { Stats.ZERO }
            specGpp = FloatArray(n)
            var idx = 0
            for ((k, v) in maxSpecialistsMap) {
                specNames[idx] = k
                specMax[idx] = v
                specStats[idx] = city.cityStats.getStatsOfSpecialist(k)
                val info = rulesetSpecs[k]
                specGpp[idx] = if (info != null) info.greatPersonPoints.sumValues().toFloat() * gppScale else 0f
                idx++
            }
        } else {
            specNames = null
            specMax = null
            specStats = null
            specGpp = null
        }

        repeat(freePopulation) {
            val surplusFood = cityStats.food
            // Evaluate unworked tiles without allocating Maps, Pairs, Stats, or CityStats
            var bestTileIdx = -1
            var bestTileRank = Float.NEGATIVE_INFINITY
            var bestLon = Int.MIN_VALUE
            var bestLat = Int.MIN_VALUE
            for (i in 0 until tileCount) {
                if (tileWorked[i]) continue
                val t = candidateTiles[i]
                val rank = Automation.rankStatsForCityWorkFast(candidateStats[i], rankCtx, surplusFood, false)
                if (bestTileIdx < 0 ||
                    rank > bestTileRank ||
                    (rank == bestTileRank && (t.longitude > bestLon || (t.longitude == bestLon && t.latitude > bestLat)))
                ) {
                    bestTileIdx = i
                    bestTileRank = rank
                    bestLon = t.longitude
                    bestLat = t.latitude
                }
            }
            val valueBestTile = if (bestTileIdx >= 0) bestTileRank else 0f

            var bestJob: String? = null
            var valueBestSpecialist = 0f
            if (specNames != null && specMax != null && specStats != null && specGpp != null) {
                var bestSpecRank = Float.NEGATIVE_INFINITY
                for (i in specNames.indices) {
                    val name = specNames[i]
                    if (specialistAllocations[name] >= specMax[i]) continue
                    val rank = Automation.rankStatsForCityWorkFast(specStats[i], rankCtx, surplusFood, true) + specGpp[i]
                    if (bestJob == null || rank > bestSpecRank) {
                        bestJob = name
                        bestSpecRank = rank
                    }
                }
                if (bestJob != null) valueBestSpecialist = bestSpecRank
            }

            // Assign population
            if (valueBestTile > valueBestSpecialist) {
                if (bestTileIdx >= 0) {
                    tileWorked[bestTileIdx] = true
                    city.workTile(candidateTiles[bestTileIdx])
                    cityStats.food += candidateStats[bestTileIdx].food
                }
            } else if (bestJob != null) {
                specialistAllocations.add(bestJob, 1)
                cityStats.food += specialistFoodBonus
            }
        }
        city.cityStats.update()
    }

    fun unassignExtraPopulation() {
        for (tile in city.getWorkedTiles().toList()) {
            if (tile.getOwner() != city.civ || tile.getWorkingCity() != city
                    || tile.aerialDistanceTo(city.getCenterTile()) > city.getWorkRange())
                city.stopWorkingTile(tile)
        }

        // unassign specialists that cannot be (e.g. the city was captured and one of the specialist buildings was destroyed)
        for ((specialistName, maxAmount) in getMaxSpecialists())
            if (specialistAllocations[specialistName] > maxAmount)
                specialistAllocations[specialistName] = maxAmount

        

        while (getFreePopulation() < 0) {
            //evaluate tiles
            val worstWorkedTile: Tile? = city.getWorkedTiles()
                    .minByOrNull {
                        Automation.rankTileForCityWork(it, city) +
                            (if (it.isLocked()) 10 else 0)
                    }
            val valueWorstTile = if (worstWorkedTile == null) 0f
            else Automation.rankTileForCityWork(worstWorkedTile, city)

            //evaluate specialists
            val worstAutoJob: String? = if (city.manualSpecialists) null else specialistAllocations.keys
                    .minByOrNull { Automation.rankSpecialist(it, city) }
            var valueWorstSpecialist = 0f
            if (worstAutoJob != null)
                valueWorstSpecialist = Automation.rankSpecialist(worstAutoJob, city)


            // un-assign population
            when {
                worstAutoJob != null && worstWorkedTile != null -> {
                    // choose between removing a specialist and removing a tile
                    if (valueWorstTile < valueWorstSpecialist) {
                        city.stopWorkingTile(worstWorkedTile)
                    }
                    else
                        specialistAllocations.add(worstAutoJob, -1)
                }
                worstAutoJob != null -> specialistAllocations.add(worstAutoJob, -1)
                worstWorkedTile != null -> city.stopWorkingTile(worstWorkedTile)
                else -> {
                    // It happens when "city.manualSpecialists == true"
                    //  and population goes below the number of specialists, e.g. city is razing.
                    // Let's give a chance to do the work automatically at least.
                    val worstJob = specialistAllocations.keys.minByOrNull {
                        Automation.rankSpecialist(it, city) }
                        ?: break // sorry, we can do nothing about that
                    specialistAllocations.add(worstJob, -1)
                }
            }
        }
    }

    @Readonly
    fun getMaxSpecialists(): Counter<String> {
        val counter = Counter<String>()
        for (building in city.cityConstructions.getBuiltBuildings())
            counter.add(building.newSpecialists())
        return counter
    }
}
