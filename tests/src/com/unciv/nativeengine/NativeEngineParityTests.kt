package com.unciv.nativeengine

import com.unciv.Constants
import com.unciv.UncivGame
import com.unciv.logic.automation.civilization.AutomationCache
import com.unciv.logic.automation.civilization.ThreatManager
import com.unciv.logic.civilization.Civilization
import com.unciv.logic.civilization.transients.CapitalConnectionsFinder
import com.unciv.logic.files.FileConversions
import com.unciv.logic.map.HexCoord
import com.unciv.logic.map.mapgenerator.MapGenerationRandomness
import com.unciv.logic.map.tile.RoadStatus
import com.unciv.logic.map.tile.toStats
import com.unciv.testing.BaseTestRunner
import com.unciv.testing.TestGame
import com.unciv.view.GameView
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.util.Base64
import java.util.zip.GZIPInputStream
import java.util.zip.GZIPOutputStream

/**
 * End-to-end parity tests asserting that the Rust native engine (`libunciv_native`) produces
 * identical rules results to the pure-Kotlin engine across pathfinding, hex LOS, incremental
 * civ vision, threat fields, capital connections, zero-allocation tile stats, Perlin noise,
 * and gzip+base64 save interop, and that fallback mode works cleanly.
 */
@RunWith(BaseTestRunner::class)
class NativeEngineParityTests {

    @After
    fun restoreNativeFlag() {
        NativeEngine.forceDisableForTesting = false
    }

    @Test
    fun nativeLibraryIsLoadedInTestEnv() {
        assertTrue("Expected libunciv_native to be loaded in :tests:test", NativeEngine.isLoaded)
        assertTrue(NativeEngine.isAvailable)
    }

    @Test
    fun pathfindingMatchesKotlinAcrossTerrainsRiversZoCAndRoads() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(6)
        val civ = testGame.addCiv()
        val enemy = testGame.addCiv()
        civ.diplomacyFunctions.makeCivilizationsMeet(enemy)
        civ.getDiplomacyManager(enemy)!!.declareWar()

        for (t in testGame.tileMap.values) t.setExplored(civ, true)

        testGame.setTileTerrainAndFeatures(HexCoord(1, 0), Constants.grassland, Constants.hill)
        testGame.setTileTerrainAndFeatures(HexCoord(0, 1), Constants.grassland, Constants.forest)
        testGame.setTileTerrain(HexCoord(-1, 1), Constants.mountain)
        testGame.tileMap[0, 0].setConnectedByRiver(testGame.tileMap[1, 1], true, false)
        testGame.tileMap[0, 0].setRoadStatus(RoadStatus.Road, civ)
        testGame.tileMap[0, -1].setRoadStatus(RoadStatus.Road, civ)
        testGame.tileMap[0, -2].setRoadStatus(RoadStatus.Road, civ)

        testGame.addUnit("Warrior", enemy, testGame.tileMap[2, 1])
        val unit = testGame.addUnit("Horseman", civ, testGame.tileMap[0, 0])
        unit.currentMovement = 4f

        UncivGame.Current.settings.useAStarPathfinding = false

        // Compare single-turn distances: Native vs Kotlin fallback
        NativeEngine.forceDisableForTesting = false
        unit.movement.clearPathfindingCache()
        val nativeDist = unit.movement.getDistanceToTiles()

        NativeEngine.forceDisableForTesting = true
        unit.movement.clearPathfindingCache()
        val kotlinDist = unit.movement.getDistanceToTiles()

        assertEquals("Reachable tile set mismatch", kotlinDist.keys, nativeDist.keys)
        for ((tile, kEntry) in kotlinDist) {
            val nEntry = nativeDist[tile]!!
            assertEquals("Movement cost mismatch at ${tile.position}", kEntry.totalMovement, nEntry.totalMovement, 0.02f)
        }

        // Compare multi-turn shortest path reachability
        val dest = testGame.tileMap[4, 0]
        NativeEngine.forceDisableForTesting = false
        unit.movement.clearPathfindingCache()
        val nativePath = unit.movement.getShortestPath(dest)
        assertTrue(nativePath.isNotEmpty())
        assertEquals(dest, nativePath.last())

        // Verify batchFirstReachable skips walled-off tile and finds reachable tile
        val walled = testGame.tileMap[-3, 0]
        for (nb in walled.neighbors) testGame.setTileTerrain(nb.position, Constants.mountain)
        val (bestIdx, batchPath) = unit.movement.batchFirstReachable(listOf(walled, dest))
        assertEquals(1, bestIdx)
        assertEquals(dest, batchPath.last())
    }

    @Test
    fun lineOfSightAndIncrementalVisionMatchKotlin() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(6)
        val civ = testGame.addCiv()
        testGame.setTileTerrainAndFeatures(HexCoord(1, 0), Constants.grassland, Constants.forest)
        testGame.setTileTerrainAndFeatures(HexCoord(2, 0), Constants.grassland, Constants.hill, Constants.forest)
        testGame.setTileTerrainAndFeatures(HexCoord(0, 1), Constants.grassland, Constants.hill)
        testGame.setTileTerrain(HexCoord(0, 3), Constants.mountain)

        for (forAttack in listOf(false, true)) {
            for (sight in 1..3) {
                NativeEngine.forceDisableForTesting = false
                val nativeTiles = testGame.tileMap.getViewableTiles(HexCoord.Zero, sight, forAttack).toSet()
                NativeEngine.forceDisableForTesting = true
                val kotlinTiles = testGame.tileMap.getViewableTiles(HexCoord.Zero, sight, forAttack).toSet()
                assertEquals("LOS mismatch (sight=$sight, forAttack=$forAttack)", kotlinTiles, nativeTiles)
            }
        }

        // Verify incremental civ vision across multi-step moves equals full rebuild
        NativeEngine.forceDisableForTesting = false
        val scout = testGame.addUnit("Scout", civ, testGame.tileMap[0, 0])
        civ.cache.updateOurTiles()
        civ.cache.updateViewableTiles()

        val steps = listOf(HexCoord(0, -1), HexCoord(-1, -1), HexCoord(-1, 0))
        for (pos in steps) {
            scout.currentMovement = 4f
            scout.movement.moveToTile(testGame.tileMap[pos])
            val afterIncremental = civ.viewableTiles.toSet()

            // Force full Kotlin rebuild and compare exact tile sets
            NativeEngine.forceDisableForTesting = true
            civ.cache.updateViewableTiles()
            val afterFullRebuild = civ.viewableTiles.toSet()
            NativeEngine.forceDisableForTesting = false
            civ.cache.updateViewableTiles()

            assertEquals("Incremental vision mismatch after moving to $pos", afterFullRebuild, afterIncremental)
        }
    }

    @Test
    fun threatFieldMatchesKotlinReference() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(6)
        val civ = testGame.addCiv()
        val enemy = testGame.addCiv()
        civ.diplomacyFunctions.makeCivilizationsMeet(enemy)
        civ.getDiplomacyManager(enemy)!!.declareWar()

        for (t in testGame.tileMap.values) t.setExplored(civ, true)
        testGame.addUnit("Warrior", civ, testGame.tileMap[0, 0])
        testGame.addUnit("Warrior", enemy, testGame.tileMap[2, 0])
        testGame.addUnit("Archer", enemy, testGame.tileMap[-3, 0])
        civ.cache.updateViewableTiles()

        val gameView = GameView(testGame.gameInfo, civ)
        for (probePos in listOf(HexCoord(0, 0), HexCoord(1, 0), HexCoord(0, 3), HexCoord(-2, 0))) {
            val tv = gameView.getTile(testGame.tileMap[probePos])
            NativeEngine.forceDisableForTesting = false
            val nDist = ThreatManager.getDistanceToClosestEnemyUnit(gameView.civView, AutomationCache(), tv, 4, true)
            val nTiles = ThreatManager.getTilesWithEnemyUnitsInDistance(gameView.civView, AutomationCache(), tv, 4).toSet()

            NativeEngine.forceDisableForTesting = true
            val kDist = ThreatManager.getDistanceToClosestEnemyUnit(gameView.civView, AutomationCache(), tv, 4, true)
            val kTiles = ThreatManager.getTilesWithEnemyUnitsInDistance(gameView.civView, AutomationCache(), tv, 4).toSet()

            assertEquals("Threat dist mismatch at $probePos", kDist, nDist)
            assertEquals("Threat tiles mismatch at $probePos", kTiles, nTiles)
        }
    }

    @Test
    fun capitalConnectionsMatchKotlinReference() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(5)
        val civ = testGame.addCiv()
        civ.tech.addTechnology("The Wheel")
        val cap = testGame.addCity(civ, testGame.tileMap[-2, 0])
        val cityB = testGame.addCity(civ, testGame.tileMap[0, 0])
        val cityC = testGame.addCity(civ, testGame.tileMap[3, 0])
        testGame.tileMap[-1, 0].setRoadStatus(RoadStatus.Road, civ)
        testGame.tileMap[-2, 0].setRoadStatus(RoadStatus.Road, civ)
        testGame.tileMap[0, 0].setRoadStatus(RoadStatus.Road, civ)

        NativeEngine.forceDisableForTesting = false
        val nativeConns = CapitalConnectionsFinder(civ).find()
        NativeEngine.forceDisableForTesting = true
        val kotlinConns = CapitalConnectionsFinder(civ).find()

        assertEquals(kotlinConns, nativeConns)
        assertTrue(cityB in nativeConns)
        assertFalse(cityC in nativeConns)
        assertTrue(cap in nativeConns)
    }

    @Test
    fun zeroAllocTileStatsMatchesBreakdownExactly() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(4)
        val civ = testGame.addCiv()
        val city = testGame.addCity(civ, testGame.tileMap[0, 0])
        testGame.setTileTerrainAndFeatures(HexCoord(1, 0), Constants.grassland, Constants.hill)
        testGame.tileMap[1, 0].setImprovement("Mine")
        testGame.tileMap[1, 0].setConnectedByRiver(testGame.tileMap[0, 0], true, false)
        testGame.setTileTerrainAndFeatures(HexCoord(0, 1), Constants.plains, Constants.forest)

        for (goldenAge in listOf(false, true)) {
            if (goldenAge) civ.goldenAges.enterGoldenAge(10)
            for (tile in city.getTiles()) {
                val fast = tile.stats.getTileStats(city, civ)
                val breakdown = tile.stats.getTileStatsBreakdown(city, civ)
                val imp = tile.getUnpillagedImprovement()
                val road = tile.getUnpillagedRoad()
                val pcts = tile.stats.getTilePercentageStats(civ, city)
                for ((cause, s) in breakdown) {
                    val cat = when (cause) {
                        imp -> com.unciv.logic.map.tile.TileStatFunctions.TilePercentageCategory.Improvement
                        road.name -> com.unciv.logic.map.tile.TileStatFunctions.TilePercentageCategory.Road
                        else -> com.unciv.logic.map.tile.TileStatFunctions.TilePercentageCategory.Terrain
                    }
                    for ((st, v) in pcts[cat]!!) {
                        s[st] *= 1f + v / 100f
                    }
                }
                val ref = breakdown.toStats()
                assertTrue("Stats mismatch at ${tile.position} (goldenAge=$goldenAge): expected $ref but was $fast", ref.equals(fast))
            }
        }
    }

    @Test
    fun perlinBatchIsBitExactWithPerTileKotlin() {
        val testGame = TestGame()
        testGame.makeHexagonalMap(6)
        testGame.tileMap.mapParameters.worldWrap = true
        val rng = MapGenerationRandomness()
        val seed = 123456.0

        NativeEngine.forceDisableForTesting = false
        val nativeBatch = rng.getPerlinNoiseBatch(testGame.tileMap, seed)

        val tiles = testGame.tileMap.tileList
        for (i in tiles.indices) {
            val ref = rng.getPerlinNoise(tiles[i], seed)
            assertEquals("Perlin mismatch at tile $i", ref, nativeBatch[i], 1e-12)
        }
    }

    @Test
    fun gzipBase64InteroperatesWithVanillaJavaStreams() {
        val sample = "{\"turns\":350,\"civilizations\":[{\"civName\":\"Rome\"}],\"padding\":\"${"Unciv-Rust-Save-Test-".repeat(2000)}\"}"

        // 1. Rust encodes -> Java GZIPInputStream decodes
        NativeEngine.forceDisableForTesting = false
        val rustEncoded = FileConversions.zip(sample)
        val javaDecoded = GZIPInputStream(Base64.getDecoder().wrap(ByteArrayInputStream(rustEncoded.toByteArray(Charsets.ISO_8859_1)))).use {
            it.readBytes().toString(Charsets.UTF_8)
        }
        assertEquals(sample, javaDecoded)

        // 2. Java GZIPOutputStream encodes -> Rust decodes
        val bos = ByteArrayOutputStream()
        GZIPOutputStream(Base64.getEncoder().wrap(bos)).use { it.write(sample.toByteArray(Charsets.UTF_8)) }
        val javaEncoded = bos.toString(Charsets.UTF_8.name())
        val rustDecoded = FileConversions.unzip(javaEncoded)
        assertEquals(sample, rustDecoded)

        // 3. Fallback mode works cleanly
        NativeEngine.forceDisableForTesting = true
        assertEquals(sample, FileConversions.unzip(FileConversions.zip(sample)))
    }
}
