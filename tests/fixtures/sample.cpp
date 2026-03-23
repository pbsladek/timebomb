// sample.cpp — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

#include <string>
#include <vector>
#include <unordered_map>
#include <functional>
#include <optional>
#include <variant>
#include <chrono>
#include <mutex>
#include <stdexcept>
#include <algorithm>
#include <cmath>
#include <sstream>
#include <iomanip>
#include <regex>

namespace sample {

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Config {
    std::string host       = "0.0.0.0";
    int         port       = 3000;
    std::string dbUrl      = "postgres://localhost/app";
    std::string jwtSecret  = "change-me";
    std::string env        = "development";

    static std::string getEnvOr(const char *key, const char *fallback) {
        const char *val = std::getenv(key);
        return val ? val : fallback;
    }

    static Config fromEnv() {
        Config cfg;
        cfg.host      = getEnvOr("HOST",       "0.0.0.0");
        cfg.port      = std::stoi(getEnvOr("PORT", "3000"));
        cfg.dbUrl     = getEnvOr("DB_URL",     "postgres://localhost/app");
        cfg.jwtSecret = getEnvOr("JWT_SECRET", "change-me");
        cfg.env       = getEnvOr("APP_ENV",    "development");
        return cfg;
    }

    bool isProduction() const { return env == "production"; }
};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

template<typename T>
class Result {
    std::variant<T, std::string> _data;
public:
    static Result ok(T value)          { Result r; r._data = std::move(value); return r; }
    static Result err(std::string msg) { Result r; r._data = std::move(msg);   return r; }

    bool isOk()  const { return std::holds_alternative<T>(_data); }
    bool isErr() const { return !isOk(); }

    T unwrap() const {
        if (!isOk()) throw std::runtime_error("unwrap on Err: " + std::get<std::string>(_data));
        return std::get<T>(_data);
    }

    T unwrapOr(T def) const { return isOk() ? std::get<T>(_data) : def; }

    template<typename F>
    auto map(F &&f) -> Result<decltype(f(std::declval<T>()))> {
        using U = decltype(f(std::declval<T>()));
        if (isOk()) return Result<U>::ok(f(std::get<T>(_data)));
        return Result<U>::err(std::get<std::string>(_data));
    }
};

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

// TODO[2019-08-15]: replace hand-rolled validators with a JSON Schema library
struct ValidationFailure {
    std::string field;
    std::string message;
};

std::optional<ValidationFailure> validateRequired(const std::string &field, const std::string &value) {
    if (value.empty())
        return ValidationFailure{field, "is required"};
    return std::nullopt;
}

std::optional<ValidationFailure> validateEmail(const std::string &field, const std::string &value) {
    static const std::regex re(R"(^[^\s@]+@[^\s@]+\.[^\s@]+$)");
    if (std::regex_match(value, re)) return std::nullopt;
    return ValidationFailure{field, "must be a valid email address"};
}

std::optional<ValidationFailure> validateMinLength(const std::string &field,
                                                    const std::string &value,
                                                    size_t min) {
    if (value.size() >= min) return std::nullopt;
    return ValidationFailure{field, "must be at least " + std::to_string(min) + " characters"};
}

std::vector<ValidationFailure> collectFailures(
    std::vector<std::optional<ValidationFailure>> checks)
{
    std::vector<ValidationFailure> errs;
    for (auto &c : checks)
        if (c) errs.push_back(*c);
    return errs;
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2020-04-01]: unordered_map with no eviction policy; add LRU before any real load
template<typename K, typename V>
class Cache {
    struct Entry { V value; std::chrono::steady_clock::time_point expiresAt; };
    std::unordered_map<K, Entry> _store;
    mutable std::mutex           _mu;

public:
    std::optional<V> get(const K &key) const {
        std::lock_guard<std::mutex> lock(_mu);
        auto it = _store.find(key);
        if (it == _store.end()) return std::nullopt;
        if (std::chrono::steady_clock::now() < it->second.expiresAt)
            return it->second.value;
        return std::nullopt;
    }

    void set(const K &key, V value, int ttlSec) {
        std::lock_guard<std::mutex> lock(_mu);
        _store[key] = {std::move(value),
                       std::chrono::steady_clock::now() + std::chrono::seconds(ttlSec)};
    }

    void del(const K &key) {
        std::lock_guard<std::mutex> lock(_mu);
        _store.erase(key);
    }

    V getOrSet(const K &key, int ttlSec, std::function<V()> fn) {
        if (auto v = get(key)) return *v;
        V result = fn();
        set(key, result, ttlSec);
        return result;
    }
};

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

// FIXME[2018-06-01]: single-process only; add a Redis token bucket for multi-node setups
struct RateResult {
    bool allowed;
    int  remaining;
    int  retryAfter;
};

class RateLimiter {
    int         _windowSec;
    int         _maxRequests;
    struct Slot { int count; std::chrono::system_clock::time_point resetAt; };
    std::unordered_map<std::string, Slot> _store;
    std::mutex _mu;

public:
    RateLimiter(int windowSec, int maxRequests)
        : _windowSec(windowSec), _maxRequests(maxRequests) {}

    RateResult check(const std::string &key) {
        std::lock_guard<std::mutex> lock(_mu);
        auto now      = std::chrono::system_clock::now();
        auto &slot    = _store[key];
        if (slot.resetAt <= now) {
            slot.count   = 0;
            slot.resetAt = now + std::chrono::seconds(_windowSec);
        }
        slot.count++;
        bool allowed = slot.count <= _maxRequests;
        int  after   = 0;
        if (!allowed) {
            auto diff = std::chrono::duration_cast<std::chrono::seconds>(slot.resetAt - now);
            after = static_cast<int>(diff.count());
        }
        return { allowed, allowed ? _maxRequests - slot.count : 0, after };
    }
};

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

template<typename T>
struct Page {
    std::vector<T> items;
    size_t         total;
    int            pageNum;
    int            pageSize;
    bool           hasNext;
    bool           hasPrev;
};

template<typename T>
Page<T> paginate(const std::vector<T> &items, int pageNum, int pageSize) {
    size_t offset = static_cast<size_t>(std::max(0, (pageNum - 1) * pageSize));
    if (offset > items.size()) offset = items.size();
    auto begin = items.begin() + static_cast<ptrdiff_t>(offset);
    auto end   = begin + std::min(static_cast<ptrdiff_t>(pageSize),
                                  static_cast<ptrdiff_t>(items.end() - begin));
    std::vector<T> chunk(begin, end);
    return { chunk, items.size(), pageNum, pageSize,
             offset + chunk.size() < items.size(), pageNum > 1 };
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-11-01][platform]: replace in-process flag map with a remote LaunchDarkly client
class FeatureFlagService {
    struct Flag { bool enabled; int rollout; std::vector<std::string> allowlist; };
    std::unordered_map<std::string, Flag> _flags;
    std::mutex _mu;

public:
    void define(const std::string &name, bool enabled, int rollout,
                std::vector<std::string> allowlist = {}) {
        std::lock_guard<std::mutex> lock(_mu);
        _flags[name] = {enabled, rollout, std::move(allowlist)};
    }

    bool isEnabled(const std::string &name, const std::string &userId = "") {
        std::lock_guard<std::mutex> lock(_mu);
        auto it = _flags.find(name);
        if (it == _flags.end() || !it->second.enabled) return false;
        auto &al = it->second.allowlist;
        return it->second.rollout >= 100 ||
               std::find(al.begin(), al.end(), userId) != al.end();
    }
};

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

std::string slugify(std::string text) {
    std::transform(text.begin(), text.end(), text.begin(),
                   [](unsigned char c) { return std::tolower(c); });
    std::string out;
    bool lastDash = false;
    for (char c : text) {
        if (std::isalnum(static_cast<unsigned char>(c))) { out += c; lastDash = false; }
        else if (!out.empty() && !lastDash)              { out += '-'; lastDash = true; }
    }
    while (!out.empty() && out.back() == '-') out.pop_back();
    return out;
}

std::string maskEmail(const std::string &email) {
    auto at = email.find('@');
    if (at == std::string::npos) return email;
    std::string local  = email.substr(0, at);
    std::string domain = email.substr(at + 1);
    size_t visible_len = std::min<size_t>(2, local.size());
    std::string visible = local.substr(0, visible_len);
    std::string stars(std::max<size_t>(1, local.size() - 2), '*');
    return visible + stars + "@" + domain;
}

// REMOVEME[2021-03-15]: legacy string helper kept for ABI compatibility — remove after v3 ships
std::string legacyTrim(const std::string &s) {
    auto start = s.find_first_not_of(" \t\n\r");
    auto end   = s.find_last_not_of(" \t\n\r");
    return start == std::string::npos ? "" : s.substr(start, end - start + 1);
}

// FIXME[2025-06-08]: formatDuration does not handle negative durations
std::string formatDuration(long ms) {
    std::ostringstream oss;
    if      (ms < 1000)   oss << ms << "ms";
    else if (ms < 60000)  oss << std::fixed << std::setprecision(1) << ms / 1000.0 << "s";
    else                  oss << ms / 60000 << "m " << (ms % 60000) / 1000 << "s";
    return oss.str();
}

std::string formatBytes(long bytes) {
    const char *units[] = { "B", "KB", "MB", "GB", "TB" };
    double v = static_cast<double>(bytes);
    int    i = 0;
    while (v >= 1024 && i < 4) { v /= 1024; ++i; }
    std::ostringstream oss;
    oss << std::fixed << std::setprecision(2) << v << " " << units[i];
    return oss.str();
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-09-01][observability]: expose counters via Prometheus HTTP endpoint
class Counter {
    std::string _name;
    long        _value = 0;
    std::mutex  _mu;
public:
    explicit Counter(std::string name) : _name(std::move(name)) {}

    void inc(long by = 1)       { std::lock_guard<std::mutex> l(_mu); _value += by; }
    long read()                  { std::lock_guard<std::mutex> l(_mu); return _value; }
    void reset()                 { std::lock_guard<std::mutex> l(_mu); _value = 0; }
    const std::string &name() const { return _name; }
};

class MetricsRegistry {
    std::unordered_map<std::string, std::shared_ptr<Counter>> _counters;
    std::mutex _mu;
public:
    std::shared_ptr<Counter> counter(const std::string &name) {
        std::lock_guard<std::mutex> lock(_mu);
        auto it = _counters.find(name);
        if (it != _counters.end()) return it->second;
        auto c = std::make_shared<Counter>(name);
        _counters[name] = c;
        return c;
    }

    std::unordered_map<std::string, long> snapshot() {
        std::lock_guard<std::mutex> lock(_mu);
        std::unordered_map<std::string, long> snap;
        for (auto &[k, v] : _counters) snap[k] = v->read();
        return snap;
    }
};

} // namespace sample
