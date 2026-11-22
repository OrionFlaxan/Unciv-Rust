package com.unciv.nativeengine

import com.unciv.utils.Log
import yairm210.purity.annotations.Readonly
import java.io.File
import java.io.FileOutputStream

/**
 * Loads `libunciv_native` once across Android (`jniLibs`), Desktop/Tests (`-Dunciv.nativeLibDir`
 * or `java.library.path`), and fat JARs (classpath extraction).
 *
 * Gracefully degrades to pure Kotlin (`isAvailable == false`) on platforms without JNI support
 * (e.g., Web/TeaVM) or when [forceDisableForTesting] is toggled.
 */
object NativeEngine {
    private const val LIB_BASE_NAME = "unciv_native"

    /** True once the shared library was loaded cleanly into this JVM/ART process. */
    val isLoaded: Boolean

    /** Test-only toggle to verify pure-Kotlin fallback parity at runtime. */
    @Volatile
    var forceDisableForTesting: Boolean = false

    /** True when native Rust kernels should be dispatched. */
    @get:Readonly
    val isAvailable: Boolean
        get() = isLoaded && !forceDisableForTesting

    init {
        isLoaded = tryLoadLibrary()
        if (isLoaded) {
            Log.debug("Unciv-Rust native engine loaded successfully.")
        } else {
            Log.debug("Unciv-Rust native engine unavailable; running in pure-Kotlin fallback mode.")
        }
    }

    private fun tryLoadLibrary(): Boolean {
        if (System.getProperty("unciv.disableNative")?.equals("true", ignoreCase = true) == true) {
            return false
        }

        // 1. Explicit directory passed by Gradle (:tests:test, :tests:ojh, :desktop:run)
        val explicitDir = System.getProperty("unciv.nativeLibDir")
        if (!explicitDir.isNullOrBlank()) {
            val candidate = File(explicitDir, platformLibFileName())
            if (candidate.isFile && tryLoadPath(candidate.absolutePath)) {
                return true
            }
        }

        // 2. Standard Android jniLibs / java.library.path lookup
        try {
            System.loadLibrary(LIB_BASE_NAME)
            return true
        } catch (_: Throwable) {
            // Fall through to classpath/workspace fallback
        }

        // 3. Extract bundled native from JAR classpath: /natives/<os>-<arch>/<lib>
        val resourcePath = "/natives/${platformTag()}/${platformLibFileName()}"
        if (tryLoadFromClasspath(resourcePath)) {
            return true
        }

        // 4. Local dev workspace staging path fallback (e.g., running directly from IDE)
        val devCandidates = listOf(
            File("core/build/rust-natives/${platformTag()}/${platformLibFileName()}"),
            File("../core/build/rust-natives/${platformTag()}/${platformLibFileName()}"),
            File("../../core/build/rust-natives/${platformTag()}/${platformLibFileName()}")
        )
        for (file in devCandidates) {
            if (file.isFile && tryLoadPath(file.absolutePath)) {
                return true
            }
        }

        return false
    }

    private fun tryLoadPath(absPath: String): Boolean = try {
        System.load(absPath)
        true
    } catch (_: Throwable) {
        false
    }

    private fun tryLoadFromClasspath(resourcePath: String): Boolean {
        return try {
            val stream = NativeEngine::class.java.getResourceAsStream(resourcePath) ?: return false
            val suffix = platformLibFileName().substringAfterLast('.', "so")
            val tmpFile = File.createTempFile("libunciv_native_", ".$suffix")
            tmpFile.deleteOnExit()
            stream.use { input ->
                FileOutputStream(tmpFile).use { output ->
                    val buf = ByteArray(16384)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        output.write(buf, 0, n)
                    }
                }
            }
            System.load(tmpFile.absolutePath)
            true
        } catch (_: Throwable) {
            false
        }
    }

    private fun platformLibFileName(): String {
        val os = (System.getProperty("os.name") ?: "").lowercase()
        return when {
            os.contains("win") -> "$LIB_BASE_NAME.dll"
            os.contains("mac") || os.contains("darwin") -> "lib$LIB_BASE_NAME.dylib"
            else -> "lib$LIB_BASE_NAME.so"
        }
    }

    private fun platformTag(): String {
        val os = (System.getProperty("os.name") ?: "").lowercase()
        val osTag = when {
            os.contains("win") -> "windows"
            os.contains("mac") || os.contains("darwin") -> "macos"
            else -> "linux"
        }
        val arch = (System.getProperty("os.arch") ?: "").lowercase()
        val archTag = when {
            arch == "amd64" || arch == "x86_64" -> "x86_64"
            arch == "aarch64" || arch == "arm64" -> "arm64"
            else -> arch
        }
        return "$osTag-$archTag"
    }
}
