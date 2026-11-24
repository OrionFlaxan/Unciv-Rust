package com.unciv.logic.automation.civilization

import com.unciv.view.CityView
import com.unciv.view.CivView
import com.unciv.view.ForeignCityView
import com.unciv.view.RoadPathingCache
import com.unciv.view.TileView
import yairm210.purity.annotations.Cache
import yairm210.purity.annotations.Readonly

/**
 * Holds data that automation caches while a civ's moves are being automated.
 * Created in [NextTurnAutomation.automateCivMoves] and passed down to wherever it's needed - it is never stored on game state,
 * so it cannot go stale between turns.
 */
class AutomationCache {

    class ClosestEnemyTileData(
        /** The farthest radius in which we have checked tiles for enemies.
         * A value of 2 means all enemies at a radius of 2 are in tilesWithEnemies. */
        var distanceSearched: Int,
        /** Stores the location of the enemy tiles that we saw with the distance at which we saw them.
         * Tiles are sorted by distance in increasing order.
         * This allows us to quickly check if they are still alive and if we should search farther.
         * It is not guaranteed that each tile in this list has an enemy (since they may have died).*/
        var tilesWithEnemies: MutableList<Pair<TileView,Int>>
    )

    val roadPathingCache = RoadPathingCache()

    /** True once Rust O(V) theater flow-fields have been built for the active civ this turn. */
    var flowFieldsBuiltForCiv: Boolean = false

    @Cache private val neighboringOwnCities = HashMap<CityView, List<CityView>>()

    /** Own cities within [maxDistance] of [cityView], computed once per city for the whole civ instead of once per unit.
     *  The first call's [maxDistance] is used for later calls on the same city. */
    @Readonly @Suppress("purity")
    fun getNeighboringOwnCities(cityView: CityView, maxDistance: Int = 12): List<CityView> =
        neighboringOwnCities.getOrPut(cityView) { cityView.getNeighboringOwnCities(maxDistance) }

    val distanceToClosestEnemyTiles = HashMap<TileView, ClosestEnemyTileData>()

    @Cache private var knownEncampmentsCache: List<TileView>? = null
    @Cache private var knownEncampmentsCityCount: Int = -1

    /** Barbarian encampments within 6 tiles of any own city, computed once per civ turn instead of once per idle unit. */
    @Readonly @Suppress("purity")
    fun getKnownEncampmentsNearCities(civView: CivView): List<TileView> {
        val cities = civView.cities()
        val cached = knownEncampmentsCache
        if (cached != null && knownEncampmentsCityCount == cities.size) {
            return cached.filter { it.isBarbarianEncampment() }
        }
        val result = cities.asSequence()
            .flatMap { it.getCenterTile().getExploredTilesInDistance(6) }
            .filter { it.isBarbarianEncampment() }
            .distinct()
            .toList()
        knownEncampmentsCache = result
        knownEncampmentsCityCount = cities.size
        return result
    }

    @Cache private var neighboringOtherCivCitiesCache: List<Pair<ForeignCityView, ForeignCityView>>? = null
    @Cache private var neighboringOtherCivCitiesCount: Int = -1

    /** Cached equivalent of [ThreatManager.getNeighboringCitiesOfOtherCivs] to avoid O(C^2) recomputation per unit in `tryPrepare`. */
    @Readonly @Suppress("purity")
    fun getNeighboringCitiesOfOtherCivs(civView: CivView): List<Pair<ForeignCityView, ForeignCityView>> {
        val cities = civView.cities()
        val cached = neighboringOtherCivCitiesCache
        if (cached != null && neighboringOtherCivCitiesCount == cities.size) return cached
        val computed = ThreatManager.getNeighboringCitiesOfOtherCivs(civView).toList()
        neighboringOtherCivCitiesCache = computed
        neighboringOtherCivCitiesCount = cities.size
        return computed
    }

    @Cache private var wartimeBorderCityTiles: HashSet<TileView>? = null
    @Cache private var wartimeBorderCityCount: Int = -1

    /**
     * Cached border-city check for `UnitAutomation.isCityThatNeedsDefendingInWartime`,
     * avoiding repeated O(C * E) scans per unit during wartime garrisoning.
     */
    @Readonly @Suppress("purity")
    fun isCityNearWarringEnemyCity(civView: CivView, city: ForeignCityView): Boolean {
        val cities = civView.cities()
        var set = wartimeBorderCityTiles
        if (set == null || wartimeBorderCityCount != cities.size) {
            val newSet = HashSet<TileView>()
            val enemyCityTiles = civView.getCivsAtWarWith()
                .flatMap { it.cities() }
                .map { it.getCenterTile() }
                .toList()
            if (enemyCityTiles.isNotEmpty()) {
                for (ourCity in cities) {
                    val center = ourCity.getCenterTile()
                    for (ec in enemyCityTiles) {
                        if (center.aerialDistanceTo(ec) <= 5) {
                            newSet.add(center)
                            break
                        }
                    }
                }
            }
            wartimeBorderCityTiles = newSet
            wartimeBorderCityCount = cities.size
            set = newSet
        }
        return city.getCenterTile() in set
    }
}
