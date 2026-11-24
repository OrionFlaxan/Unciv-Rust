package com.unciv.nativeengine

import com.unciv.logic.GameInfo
import java.util.concurrent.Executors
import java.util.concurrent.Future
import java.util.concurrent.atomic.AtomicReference

/**
 * Lightweight background speculative pre-warmer for human single-player turns.
 *
 * While the player reads the map and decides their moves on [com.unciv.ui.screens.worldscreen.WorldScreen],
 * this daemon thread ensures the live `originalGameInfo.tileMap.nativeWorld` static topology is warm and
 * pre-clones a ready Rust topology handle (via ~0.05 ms `nativeCloneWorldTopology`) so when the player
 * taps **Next Turn**, `gameInfoClone` skips rebuilding the entire 4,921-tile static terrain graph across JNI!
 */
object NativeSpeculativePrewarmer {

    private class WarmedTopology(
        val sourceGameId: String,
        val numTiles: Int,
        val sourceMirror: NativeWorldMirror
    )

    private val executor = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "unciv-rust-prewarm").apply {
            isDaemon = true
            priority = Thread.MIN_PRIORITY
        }
    }

    private val pendingTask = AtomicReference<Future<*>?>(null)
    private val readyRef = AtomicReference<WarmedTopology?>(null)

    /**
     * Schedules a low-priority background warmup for [gameInfo] during a human turn.
     * Safe to call repeatedly; any previous pending task is cancelled first.
     */
    fun scheduleWarmup(gameInfo: GameInfo) {
        if (!NativeEngine.isAvailable) return
        if (gameInfo.gameParameters.isOnlineMultiplayer) return
        val tileMap = gameInfo.tileMap
        if (tileMap.tileList.isEmpty() || tileMap.tileMatrix.isEmpty() || tileMap.ruleset == null) return

        val gameId = gameInfo.gameId
        val numTiles = tileMap.tileList.size
        val existingReady = readyRef.get()
        if (existingReady != null
            && existingReady.sourceGameId == gameId
            && existingReady.numTiles == numTiles
            && existingReady.sourceMirror.handle != 0L
            && !existingReady.sourceMirror.topologyDirty
        ) {
            return // Already warm for this map!
        }

        cancelPending()
        val future = executor.submit {
            try {
                val mirror = tileMap.getOrCreateNativeWorld() ?: return@submit
                mirror.ensureTopologySynced()
                if (mirror.handle != 0L && !mirror.topologyDirty) {
                    readyRef.set(WarmedTopology(gameId, numTiles, mirror))
                }
            } catch (_: Throwable) {
                // Best-effort speculative prewarm; never crash UI or turn thread
            }
        }
        pendingTask.set(future)
    }

    /**
     * Transfers a warm static topology clone into [gameInfoClone] right after `gameInfoClone.setTransients()`
     * in `WorldScreen.nextTurn()`. Falls back cleanly to cloning directly from [originalGameInfo] if needed.
     */
    fun handoffTopologyToClone(originalGameInfo: GameInfo, gameInfoClone: GameInfo): Boolean {
        if (!NativeEngine.isAvailable) return false
        cancelPending()
        val warmed = readyRef.get()
        val sourceMirror = if (warmed != null
            && warmed.sourceGameId == originalGameInfo.gameId
            && warmed.numTiles == gameInfoClone.tileMap.tileList.size
            && warmed.sourceMirror.handle != 0L
        ) {
            warmed.sourceMirror
        } else {
            originalGameInfo.tileMap.nativeWorld
        }
        if (sourceMirror == null || sourceMirror.handle == 0L) return false
        return gameInfoClone.tileMap.adoptClonedNativeTopology(sourceMirror)
    }

    fun cancelPending() {
        pendingTask.getAndSet(null)?.cancel(false)
    }

    fun clear() {
        cancelPending()
        readyRef.set(null)
    }
}
