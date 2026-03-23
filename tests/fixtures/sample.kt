// sample.kt — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Detonated      (2018–2021): 4
//   Ticking        (2025-06):   1
//   Inert / OK     (2088/2099): 2

package com.example.app

import kotlinx.coroutines.*
import kotlinx.serialization.*
import kotlinx.serialization.json.*
import java.time.Instant
import java.util.concurrent.ConcurrentHashMap

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

@Serializable
data class AppConfig(
    val host: String = "0.0.0.0",
    val port: Int = 3000,
    val dbUrl: String = "postgres://localhost/app",
    val jwtSecret: String = "change-me",
    val env: String = "development",
) {
    val isProduction: Boolean get() = env == "production"

    companion object {
        fun fromEnvironment() = AppConfig(
            host      = System.getenv("HOST")       ?: "0.0.0.0",
            port      = System.getenv("PORT")?.toInt() ?: 3000,
            dbUrl     = System.getenv("DB_URL")     ?: "postgres://localhost/app",
            jwtSecret = System.getenv("JWT_SECRET") ?: "change-me",
            env       = System.getenv("APP_ENV")    ?: "development",
        )
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

sealed class Result<out T> {
    data class Ok<T>(val value: T) : Result<T>()
    data class Err(val error: String) : Result<Nothing>()

    val isOk: Boolean get() = this is Ok
    val isErr: Boolean get() = this is Err
}

fun <T> Result<T>.unwrap(): T = when (this) {
    is Result.Ok  -> value
    is Result.Err -> error("unwrap on Err: $error")
}

fun <T> Result<T>.unwrapOr(default: T): T = when (this) {
    is Result.Ok  -> value
    is Result.Err -> default
}

fun <T, U> Result<T>.map(f: (T) -> U): Result<U> = when (this) {
    is Result.Ok  -> Result.Ok(f(value))
    is Result.Err -> this
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

data class ValidationFailure(val field: String, val message: String)

// TODO[2020-08-10]: replace hand-rolled validators with a proper schema library (e.g. konform)
fun validateRequired(field: String, value: String?): ValidationFailure? =
    if (value.isNullOrEmpty()) ValidationFailure(field, "is required") else null

fun validateEmail(field: String, value: String): ValidationFailure? {
    val re = Regex("""^[^\s@]+@[^\s@]+\.[^\s@]+$""")
    return if (re.matches(value)) null
    else ValidationFailure(field, "must be a valid email address")
}

fun validateMinLength(field: String, value: String, min: Int): ValidationFailure? =
    if (value.length >= min) null
    else ValidationFailure(field, "must be at least $min characters")

fun collectFailures(vararg checks: ValidationFailure?): List<ValidationFailure> =
    checks.filterNotNull()

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

private data class CacheEntry<V>(val value: V, val expiresAt: Long)

// HACK[2019-07-20]: unbounded in-memory map; add LRU eviction and TTL sweep before going to prod
class Cache<K : Any, V : Any> {
    private val store = ConcurrentHashMap<K, CacheEntry<V>>()

    fun get(key: K): V? {
        val entry = store[key] ?: return null
        if (System.currentTimeMillis() < entry.expiresAt) return entry.value
        store.remove(key)
        return null
    }

    fun set(key: K, value: V, ttlMs: Long) {
        store[key] = CacheEntry(value, System.currentTimeMillis() + ttlMs)
    }

    fun delete(key: K) { store.remove(key) }

    fun getOrSet(key: K, ttlMs: Long, fn: () -> V): V =
        get(key) ?: fn().also { set(key, it, ttlMs) }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

data class RateResult(val allowed: Boolean, val remaining: Int, val retryAfterMs: Long)

private data class RateEntry(val count: Int, val resetAt: Long)

// FIXME[2021-05-01]: single-instance only; migrate to Redis token bucket before horizontal scale-out
class RateLimiter(private val windowMs: Long, private val maxRequests: Int) {
    private val store = ConcurrentHashMap<String, RateEntry>()

    fun check(key: String): RateResult {
        val now   = System.currentTimeMillis()
        val entry = store[key].let { e ->
            if (e == null || e.resetAt <= now) RateEntry(0, now + windowMs) else e
        }
        val updated  = entry.copy(count = entry.count + 1)
        store[key]   = updated
        val allowed  = updated.count <= maxRequests
        val after    = if (allowed) 0L else entry.resetAt - now
        return RateResult(
            allowed      = allowed,
            remaining    = if (allowed) maxRequests - updated.count else 0,
            retryAfterMs = after,
        )
    }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

data class Page<T>(
    val items: List<T>,
    val total: Int,
    val pageNum: Int,
    val pageSize: Int,
    val hasNext: Boolean,
    val hasPrev: Boolean,
)

fun <T> paginate(items: List<T>, pageNum: Int, pageSize: Int): Page<T> {
    val offset = maxOf(0, (pageNum - 1) * pageSize)
    val chunk  = items.drop(offset).take(pageSize)
    return Page(
        items    = chunk,
        total    = items.size,
        pageNum  = pageNum,
        pageSize = pageSize,
        hasNext  = offset + chunk.size < items.size,
        hasPrev  = pageNum > 1,
    )
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

private data class FlagDef(val enabled: Boolean, val rollout: Int, val allowlist: Set<String>)

// TODO[2099-04-01][platform]: replace in-process map with a remote LaunchDarkly client
class FeatureFlagService {
    private val flags = ConcurrentHashMap<String, FlagDef>()

    fun define(name: String, enabled: Boolean, rollout: Int, allowlist: List<String> = emptyList()) {
        flags[name] = FlagDef(enabled, rollout, allowlist.toSet())
    }

    fun isEnabled(name: String, userId: String? = null): Boolean {
        val f = flags[name] ?: return false
        if (!f.enabled) return false
        return f.rollout >= 100 || (userId != null && f.allowlist.contains(userId))
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

fun slugify(text: String): String =
    text.lowercase()
        .replace(Regex("[^a-z0-9\\s-]"), "")
        .replace(Regex("[\\s-]+"), "-")
        .trimStart('-')
        .trimEnd('-')

fun maskEmail(email: String): String {
    val at = email.indexOf('@')
    if (at < 0) return email
    val local  = email.substring(0, at)
    val domain = email.substring(at + 1)
    val vis    = local.take(2)
    val stars  = "*".repeat(maxOf(1, local.length - 2))
    return "$vis$stars@$domain"
}

// FIXME[2025-06-12]: formatDuration does not handle negative values
fun formatDuration(ms: Long): String = when {
    ms < 1_000L  -> "${ms}ms"
    ms < 60_000L -> "${"%.1f".format(ms / 1_000.0)}s"
    else         -> "${ms / 60_000}m ${(ms % 60_000) / 1_000}s"
}

fun formatBytes(bytes: Long): String {
    val units = listOf("B", "KB", "MB", "GB", "TB")
    var v = bytes.toDouble()
    var i = 0
    while (v >= 1024 && i < units.lastIndex) { v /= 1024; i++ }
    return "${"%.2f".format(v)} ${units[i]}"
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-06-01][observability]: expose counters via Micrometer + Prometheus endpoint
class Counter(val name: String) {
    private var value: Long = 0L

    fun increment(by: Long = 1) { value += by }
    fun read(): Long = value
    fun reset() { value = 0L }
}

class MetricsRegistry {
    private val counters = ConcurrentHashMap<String, Counter>()

    fun counter(name: String): Counter = counters.getOrPut(name) { Counter(name) }

    fun snapshot(): Map<String, Long> = counters.mapValues { (_, c) -> c.read() }
}

// STOPSHIP[2018-11-01]: legacy metrics bridge leaks heap; remove before next release
fun legacyRecord(name: String, @Suppress("UNUSED_PARAMETER") value: Long) {
    // intentionally empty — bridge only, callers must migrate to MetricsRegistry
}
