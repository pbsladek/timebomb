// sample.d — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

module sample;

import std.stdio;
import std.string;
import std.conv;
import std.algorithm;
import std.array;
import std.datetime;
import std.typecons : Nullable, nullable;
import std.range : take, drop;
import std.math : abs;
import core.sync.mutex : Mutex;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Config {
    string host       = "0.0.0.0";
    int    port       = 3000;
    string dbUrl      = "postgres://localhost/app";
    string jwtSecret  = "change-me";
    string env        = "development";

    static Config fromEnv() {
        import std.process : environment;
        Config cfg;
        cfg.host      = environment.get("HOST",       "0.0.0.0");
        cfg.port      = environment.get("PORT",       "3000").to!int;
        cfg.dbUrl     = environment.get("DB_URL",     "postgres://localhost/app");
        cfg.jwtSecret = environment.get("JWT_SECRET", "change-me");
        cfg.env       = environment.get("APP_ENV",    "development");
        return cfg;
    }

    bool isProduction() const { return env == "production"; }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

struct Result(T) {
    private bool  _ok;
    private T     _value;
    private string _error;

    static Result ok(T value) {
        Result r;
        r._ok    = true;
        r._value = value;
        return r;
    }

    static Result err(string error) {
        Result r;
        r._ok    = false;
        r._error = error;
        return r;
    }

    bool isOk()  const { return  _ok; }
    bool isErr() const { return !_ok; }

    T unwrap() {
        if (!_ok) throw new Exception("unwrap on Err: " ~ _error);
        return _value;
    }

    T unwrapOr(T default_) { return _ok ? _value : default_; }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

// TODO[2020-02-15]: replace hand-rolled validators with a schema library
struct ValidationError {
    string field;
    string message;
}

Nullable!ValidationError validateRequired(string field, string value) {
    if (value.length == 0)
        return nullable(ValidationError(field, "is required"));
    return Nullable!ValidationError.init;
}

Nullable!ValidationError validateEmail(string field, string value) {
    import std.regex : matchFirst, regex;
    auto re = regex(`^[^\s@]+@[^\s@]+\.[^\s@]+$`);
    if (matchFirst(value, re))
        return Nullable!ValidationError.init;
    return nullable(ValidationError(field, "must be a valid email address"));
}

ValidationError[] collectFailures(Nullable!ValidationError[] checks) {
    ValidationError[] errs;
    foreach (c; checks)
        if (!c.isNull) errs ~= c.get;
    return errs;
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2019-06-01]: associative-array cache; wire up Memcached before any load test
struct CacheEntry(V) {
    V      value;
    long   expiresAt;
}

class Cache(K, V) {
    private CacheEntry!V[K] _store;
    private Mutex _mu;

    this() { _mu = new Mutex(); }

    Nullable!V get(K key) {
        synchronized (_mu) {
            if (auto e = key in _store) {
                if (Clock.currTime.toUnixTime() < e.expiresAt)
                    return nullable(e.value);
                _store.remove(key);
            }
            return Nullable!V.init;
        }
    }

    void set(K key, V value, int ttlSec) {
        synchronized (_mu) {
            _store[key] = CacheEntry!V(value, Clock.currTime.toUnixTime() + ttlSec);
        }
    }

    void del(K key) {
        synchronized (_mu) { _store.remove(key); }
    }

    V getOrSet(K key, int ttlSec, V delegate() fn) {
        auto v = get(key);
        if (!v.isNull) return v.get;
        V result = fn();
        set(key, result, ttlSec);
        return result;
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

// FIXME[2021-11-01]: no distributed coordination; add a Redis-backed token bucket
struct RateResult {
    bool allowed;
    int  remaining;
    int  retryAfter;
}

class RateLimiter {
    private int   _windowSec;
    private int   _maxRequests;
    private int[string]   _counts;
    private long[string]  _resets;
    private Mutex         _mu;

    this(int windowSec, int maxRequests) {
        _windowSec   = windowSec;
        _maxRequests = maxRequests;
        _mu          = new Mutex();
    }

    RateResult check(string key) {
        synchronized (_mu) {
            long now = Clock.currTime.toUnixTime();
            if (key !in _resets || _resets[key] <= now) {
                _counts[key] = 0;
                _resets[key] = now + _windowSec;
            }
            _counts[key]++;
            int  count   = _counts[key];
            bool allowed = count <= _maxRequests;
            return RateResult(
                allowed,
                allowed ? _maxRequests - count : 0,
                allowed ? 0 : cast(int)(_resets[key] - now)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

struct Page(T) {
    T[]  items;
    size_t total;
    int  pageNum;
    int  pageSize;
    bool hasNext;
    bool hasPrev;
}

Page!T paginate(T)(T[] items, int pageNum, int pageSize) {
    size_t offset = cast(size_t)(max(0, (pageNum - 1) * pageSize));
    if (offset > items.length) offset = items.length;
    T[] chunk = items.drop(offset).take(pageSize).array;
    return Page!T(
        chunk, items.length, pageNum, pageSize,
        offset + chunk.length < items.length,
        pageNum > 1
    );
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-08-01][platform]: replace in-process flags with a remote config service
class FeatureFlagService {
    private struct Flag { bool enabled; int rollout; string[] allowlist; }
    private Flag[string] _flags;

    void define(string name, bool enabled, int rollout, string[] allowlist = []) {
        _flags[name] = Flag(enabled, rollout, allowlist);
    }

    bool isEnabled(string name, string userId = null) {
        if (auto f = name in _flags) {
            return f.enabled && (userId != null && f.allowlist.canFind(userId) || f.rollout >= 100);
        }
        return false;
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

string slugify(string text) {
    import std.uni : toLower;
    import std.regex : replaceAll, regex;
    auto lower  = text.toLower;
    auto noSpec = lower.replaceAll(regex(`[^a-z0-9\s-]`), "");
    auto dashed = noSpec.replaceAll(regex(`[\s-]+`), "-");
    return dashed.strip('-');
}

string maskEmail(string email) {
    auto parts = email.findSplit("@");
    if (parts[1].length == 0) return email;
    string local  = parts[0];
    string domain = parts[2];
    string visible = local[0 .. min(2, local.length)];
    string stars   = replicate("*", max(1, cast(int)local.length - 2));
    return visible ~ stars ~ "@" ~ domain;
}

// FIXME[2025-06-10]: formatDuration loses sub-millisecond precision
string formatDuration(long ms) {
    if (ms < 1000)   return ms.to!string ~ "ms";
    if (ms < 60000)  return format("%.1fs", ms / 1000.0);
    return format("%dm %ds", ms / 60000, (ms % 60000) / 1000);
}

string formatBytes(long bytes) {
    string[] units = ["B", "KB", "MB", "GB", "TB"];
    double v = bytes;
    int    i = 0;
    while (v >= 1024 && i < cast(int)units.length - 1) { v /= 1024; i++; }
    return format("%.2f %s", v, units[i]);
}

Result!T retry(T)(int n, Result!T delegate() fn) {
    Result!T last;
    foreach (_; 0 .. n) {
        last = fn();
        if (last.isOk) return last;
    }
    return last;
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-02-01][observability]: expose counters via a Prometheus HTTP endpoint
class Counter {
    private string _name;
    private long   _value;

    this(string name) { _name = name; }

    void   inc(long by = 1) { _value += by; }
    long   read() const     { return _value; }
    void   reset()          { _value = 0; }
    string name() const     { return _name; }
}

class MetricsRegistry {
    private Counter[string] _counters;

    Counter counter(string name) {
        if (auto c = name in _counters) return *c;
        auto c = new Counter(name);
        _counters[name] = c;
        return c;
    }

    long[string] snapshot() const {
        long[string] snap;
        foreach (k, v; _counters) snap[k] = v.read();
        return snap;
    }
}
