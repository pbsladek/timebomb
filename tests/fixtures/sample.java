// sample.java — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

package fixture;

import java.security.MessageDigest;
import java.time.Duration;
import java.time.Instant;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.*;
import java.util.regex.Pattern;
import java.util.stream.*;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

class AppException extends RuntimeException {
    private final String code;
    private final int    statusCode;
    private final Object details;

    AppException(String code, String message, int statusCode, Object details) {
        super(message);
        this.code       = code;
        this.statusCode = statusCode;
        this.details    = details;
    }

    String getCode()       { return code; }
    int    getStatusCode() { return statusCode; }
    Object getDetails()    { return details; }
}

class ValidationException extends AppException {
    ValidationException(String message, Object details) {
        super("VALIDATION_ERROR", message, 422, details);
    }
}

class NotFoundException extends AppException {
    NotFoundException(String resource) {
        super("NOT_FOUND", resource + " not found", 404, null);
    }
}

class ConflictException extends AppException {
    ConflictException(String message) {
        super("CONFLICT", message, 409, null);
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

sealed interface Result<T> permits Result.Ok, Result.Err {
    record Ok<T>(T value)       implements Result<T> {}
    record Err<T>(String reason) implements Result<T> {}

    default boolean isOk()  { return this instanceof Ok<T>; }
    default T       unwrap() {
        if (this instanceof Ok<T> ok) return ok.value();
        throw new NoSuchElementException(((Err<T>) this).reason());
    }
    default T unwrapOr(T fallback) {
        return this instanceof Ok<T> ok ? ok.value() : fallback;
    }
    default <U> Result<U> map(Function<T, U> fn) {
        return this instanceof Ok<T> ok ? new Ok<>(fn.apply(ok.value())) : new Err<>(((Err<T>) this).reason());
    }

    static <T> Result<T> ok(T value)        { return new Ok<>(value); }
    static <T> Result<T> err(String reason)  { return new Err<>(reason); }
    static <T> Result<T> tryGet(Supplier<T> fn) {
        try { return ok(fn.get()); } catch (Exception e) { return err(e.getMessage()); }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

record ValidationFailure(String field, String message, String value) {}

// TODO[2021-01-10]: replace hand-rolled validators with Bean Validation (JSR-380)
class Validator {
    private static final Pattern EMAIL = Pattern.compile("^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$");

    static ValidationFailure required(String field, String value) {
        return (value == null || value.isBlank())
            ? new ValidationFailure(field, "is required", null)
            : null;
    }

    static ValidationFailure email(String field, String value) {
        return (value == null || !EMAIL.matcher(value).matches())
            ? new ValidationFailure(field, "must be a valid email address", value)
            : null;
    }

    static ValidationFailure minLength(String field, String value, int min) {
        return (value == null || value.length() < min)
            ? new ValidationFailure(field, "must be at least " + min + " characters", value)
            : null;
    }

    static ValidationFailure maxLength(String field, String value, int max) {
        return (value == null || value.length() > max)
            ? new ValidationFailure(field, "must be at most " + max + " characters", value)
            : null;
    }

    @SafeVarargs
    static List<ValidationFailure> collect(ValidationFailure... failures) {
        return Arrays.stream(failures).filter(Objects::nonNull).collect(Collectors.toList());
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

record Config(
    String host,
    int    port,
    String dbUrl,
    String jwtSecret,
    int    jwtExpirySec,
    int    cacheTtlSec,
    int    rateMaxRequests,
    int    rateWindowSec,
    String env
) {
    static Config fromEnv() {
        return new Config(
            getenv("HOST", "0.0.0.0"),
            Integer.parseInt(getenv("PORT", "3000")),
            getenv("DB_URL", "jdbc:postgresql://localhost/app"),
            getenv("JWT_SECRET", "change-me"),
            Integer.parseInt(getenv("JWT_EXPIRY", "3600")),
            Integer.parseInt(getenv("CACHE_TTL", "300")),
            Integer.parseInt(getenv("RATE_MAX", "100")),
            Integer.parseInt(getenv("RATE_WINDOW", "60")),
            getenv("APP_ENV", "development"));
    }

    boolean isProduction() { return "production".equals(env); }

    private static String getenv(String key, String fallback) {
        String v = System.getenv(key);
        return (v != null && !v.isBlank()) ? v : fallback;
    }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2019-09-01]: ConcurrentHashMap stand-in; replace with Caffeine or Redis
class Cache<K, V> {
    private record Entry<V>(V value, Instant expiresAt) {}

    private final ConcurrentHashMap<K, Entry<V>> store = new ConcurrentHashMap<>();

    Optional<V> get(K key) {
        var entry = store.get(key);
        if (entry == null || Instant.now().isAfter(entry.expiresAt())) {
            store.remove(key);
            return Optional.empty();
        }
        return Optional.of(entry.value());
    }

    void set(K key, V value, Duration ttl) {
        store.put(key, new Entry<>(value, Instant.now().plus(ttl)));
    }

    void delete(K key) { store.remove(key); }

    V getOrSet(K key, Duration ttl, Supplier<V> factory) {
        return get(key).orElseGet(() -> {
            V v = factory.get();
            set(key, v, ttl);
            return v;
        });
    }

    void cleanup() {
        Instant now = Instant.now();
        store.entrySet().removeIf(e -> now.isAfter(e.getValue().expiresAt()));
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

class RateLimiter {
    private record Entry(int count, Instant resetAt) {}

    private final ConcurrentHashMap<String, Entry> store = new ConcurrentHashMap<>();
    private final int windowSec, maxRequests;

    RateLimiter(int windowSec, int maxRequests) {
        this.windowSec   = windowSec;
        this.maxRequests = maxRequests;
    }

    record CheckResult(boolean allowed, int remaining, int retryAfterSec) {}

    CheckResult check(String key) {
        Instant now = Instant.now();
        Entry entry = store.compute(key, (k, e) -> {
            if (e == null || now.isAfter(e.resetAt())) return new Entry(1, now.plusSeconds(windowSec));
            return new Entry(e.count() + 1, e.resetAt());
        });
        int remaining   = Math.max(0, maxRequests - entry.count());
        boolean allowed = entry.count() <= maxRequests;
        int retryAfter  = allowed ? 0 : (int) Duration.between(now, entry.resetAt()).getSeconds();
        return new CheckResult(allowed, remaining, retryAfter);
    }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

record Page<T>(List<T> items, int total, int pageNum, int pageSize) {
    boolean hasNext() { return (pageNum - 1) * pageSize + items.size() < total; }
    boolean hasPrev() { return pageNum > 1; }

    static <T> Page<T> of(List<T> source, int pageNum, int pageSize) {
        int offset = (pageNum - 1) * pageSize;
        var items  = source.stream().skip(offset).limit(pageSize).collect(Collectors.toList());
        return new Page<>(items, source.size(), pageNum, pageSize);
    }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-11-01][platform]: replace with LaunchDarkly Java SDK
class FeatureFlagService {
    private record Flag(boolean enabled, int rolloutPercent, Set<String> allowlist) {}
    private final ConcurrentHashMap<String, Flag> flags = new ConcurrentHashMap<>();

    void define(String name, boolean enabled, int rollout, String... allowlist) {
        flags.put(name, new Flag(enabled, rollout, Set.of(allowlist)));
    }

    boolean isEnabled(String name, String userId) {
        Flag flag = flags.get(name);
        if (flag == null || !flag.enabled()) return false;
        if (userId != null && flag.allowlist().contains(userId)) return true;
        if (flag.rolloutPercent() >= 100) return true;
        try {
            var md5    = MessageDigest.getInstance("MD5");
            var hash   = md5.digest((name + ":" + (userId != null ? userId : "anon")).getBytes());
            int bucket = Math.abs(hash[0] & 0xFF) % 100;
            return bucket < flag.rolloutPercent();
        } catch (Exception e) { return false; }
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

// FIXME[2020-07-15]: open state does not honour half-open probe limit
class CircuitBreaker {
    private enum State { CLOSED, OPEN, HALF_OPEN }

    private volatile State   state       = State.CLOSED;
    private volatile int     failures    = 0;
    private volatile Instant lastFailure = Instant.MIN;
    private final    int     threshold;
    private final    Duration resetTimeout;

    CircuitBreaker(int threshold, Duration resetTimeout) {
        this.threshold    = threshold;
        this.resetTimeout = resetTimeout;
    }

    <T> T call(Supplier<T> fn) {
        synchronized (this) {
            if (state == State.OPEN) {
                if (Duration.between(lastFailure, Instant.now()).compareTo(resetTimeout) >= 0)
                    state = State.HALF_OPEN;
                else
                    throw new AppException("CIRCUIT_OPEN", "Service temporarily unavailable", 503, null);
            }
        }
        try {
            T result = fn.get();
            synchronized (this) { failures = 0; state = State.CLOSED; }
            return result;
        } catch (Exception e) {
            synchronized (this) {
                failures++;
                lastFailure = Instant.now();
                if (failures >= threshold) state = State.OPEN;
            }
            throw e;
        }
    }

    String getState() { return state.toString(); }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

class Counter {
    private final AtomicLong value = new AtomicLong();
    void inc()          { value.incrementAndGet(); }
    void add(long n)    { value.addAndGet(n); }
    long read()         { return value.get(); }
    void reset()        { value.set(0); }
}

class Histogram {
    // TODO[2025-06-10]: wire to Micrometer and export to Prometheus
    private final List<Double> samples = Collections.synchronizedList(new ArrayList<>());

    void observe(double value) { samples.add(value); }

    double percentile(double p) {
        if (samples.isEmpty()) return 0;
        List<Double> sorted = new ArrayList<>(samples);
        Collections.sort(sorted);
        int idx = (int) Math.ceil(p / 100.0 * sorted.size()) - 1;
        return sorted.get(Math.max(0, idx));
    }
    void reset() { samples.clear(); }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

class StringUtils {
    private static final Pattern NON_ALNUM = Pattern.compile("[^a-z0-9]+");

    static String slugify(String text) {
        return NON_ALNUM.matcher(text.toLowerCase()).replaceAll("-").replaceAll("^-|-$", "");
    }

    static String maskEmail(String email) {
        String[] parts = email.split("@", 2);
        if (parts.length != 2) return email;
        String local   = parts[0];
        String visible = local.length() > 2 ? local.substring(0, 2) : local.substring(0, 1);
        String stars   = "*".repeat(Math.max(1, local.length() - 2));
        return visible + stars + "@" + parts[1];
    }

    static String truncate(String s, int maxLen, String suffix) {
        return s.length() <= maxLen ? s : s.substring(0, maxLen - suffix.length()) + suffix;
    }

    static String formatBytes(long bytes) {
        String[] units = {"B", "KB", "MB", "GB", "TB"};
        double v = bytes;
        int i = 0;
        while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
        return String.format("%.2f %s", v, units[i]);
    }

    static String formatDuration(long ms) {
        if (ms < 1000)   return ms + "ms";
        if (ms < 60_000) return String.format("%.1fs", ms / 1000.0);
        return (ms / 60_000) + "m " + (ms % 60_000 / 1000) + "s";
    }
}
