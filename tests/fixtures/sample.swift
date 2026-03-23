// sample.swift — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

import Foundation

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

struct Config {
    let host:      String
    let port:      Int
    let dbURL:     String
    let jwtSecret: String
    let env:       String

    static func fromEnvironment() -> Config {
        Config(
            host:      ProcessInfo.processInfo.environment["HOST"]       ?? "0.0.0.0",
            port:      Int(ProcessInfo.processInfo.environment["PORT"]   ?? "3000") ?? 3000,
            dbURL:     ProcessInfo.processInfo.environment["DB_URL"]     ?? "postgres://localhost/app",
            jwtSecret: ProcessInfo.processInfo.environment["JWT_SECRET"] ?? "change-me",
            env:       ProcessInfo.processInfo.environment["APP_ENV"]    ?? "development"
        )
    }

    var isProduction: Bool { env == "production" }
}

// ---------------------------------------------------------------------------
// Result helpers
// ---------------------------------------------------------------------------

enum AppError: Error {
    case validation(String)
    case notFound(String)
    case unauthorized
    case custom(String)
}

// REMOVEME[2018-07-01]: legacy error bridge — remove once ObjC interop layer is gone
func bridgeError(_ err: AppError) -> NSError {
    NSError(domain: "AppError", code: -1, userInfo: [NSLocalizedDescriptionKey: "\(err)"])
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

struct ValidationFailure {
    let field:   String
    let message: String
}

// TODO[2020-03-15]: replace with Codable-based schema validation
func validateRequired(field: String, value: String?) -> ValidationFailure? {
    guard let v = value, !v.isEmpty else {
        return ValidationFailure(field: field, message: "is required")
    }
    return nil
}

func validateEmail(field: String, value: String) -> ValidationFailure? {
    let pattern = #"^[^\s@]+@[^\s@]+\.[^\s@]+$"#
    guard let _ = value.range(of: pattern, options: .regularExpression) else {
        return ValidationFailure(field: field, message: "must be a valid email address")
    }
    return nil
}

func validateMinLength(field: String, value: String, min: Int) -> ValidationFailure? {
    guard value.count >= min else {
        return ValidationFailure(field: field, message: "must be at least \(min) characters")
    }
    return nil
}

func collectFailures(_ checks: [ValidationFailure?]) -> [ValidationFailure] {
    checks.compactMap { $0 }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2019-08-01]: NSCache wrapper with no per-entry TTL; replace with a proper LRU+TTL cache
final class Cache<Key: Hashable, Value> {
    private struct Entry {
        let value:     Value
        let expiresAt: Date
    }
    private var store: [Key: Entry] = [:]
    private let lock = NSLock()

    func get(_ key: Key) -> Value? {
        lock.lock(); defer { lock.unlock() }
        guard let e = store[key] else { return nil }
        if Date() < e.expiresAt { return e.value }
        store.removeValue(forKey: key)
        return nil
    }

    func set(_ key: Key, value: Value, ttl: TimeInterval) {
        lock.lock(); defer { lock.unlock() }
        store[key] = Entry(value: value, expiresAt: Date().addingTimeInterval(ttl))
    }

    func delete(_ key: Key) {
        lock.lock(); defer { lock.unlock() }
        store.removeValue(forKey: key)
    }

    func getOrSet(_ key: Key, ttl: TimeInterval, _ fn: () -> Value) -> Value {
        if let v = get(key) { return v }
        let v = fn()
        set(key, value: v, ttl: ttl)
        return v
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

// FIXME[2021-04-20]: single-instance only; add a distributed token bucket before multi-node deploy
struct RateResult {
    let allowed:    Bool
    let remaining:  Int
    let retryAfter: Int
}

final class RateLimiter {
    private struct Slot { var count: Int; var resetAt: Date }
    private let windowSec:   Int
    private let maxRequests: Int
    private var store: [String: Slot] = [:]
    private let lock = NSLock()

    init(windowSec: Int, maxRequests: Int) {
        self.windowSec   = windowSec
        self.maxRequests = maxRequests
    }

    func check(key: String) -> RateResult {
        lock.lock(); defer { lock.unlock() }
        let now = Date()
        if store[key] == nil || store[key]!.resetAt <= now {
            store[key] = Slot(count: 0, resetAt: now.addingTimeInterval(Double(windowSec)))
        }
        store[key]!.count += 1
        let count   = store[key]!.count
        let allowed = count <= maxRequests
        let after   = allowed ? 0 : Int(store[key]!.resetAt.timeIntervalSince(now))
        return RateResult(allowed: allowed,
                          remaining: allowed ? maxRequests - count : 0,
                          retryAfter: after)
    }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

struct Page<T> {
    let items:    [T]
    let total:    Int
    let pageNum:  Int
    let pageSize: Int
    let hasNext:  Bool
    let hasPrev:  Bool
}

func paginate<T>(_ items: [T], pageNum: Int, pageSize: Int) -> Page<T> {
    let offset = max(0, (pageNum - 1) * pageSize)
    let chunk  = Array(items.dropFirst(offset).prefix(pageSize))
    return Page(
        items:    chunk,
        total:    items.count,
        pageNum:  pageNum,
        pageSize: pageSize,
        hasNext:  offset + chunk.count < items.count,
        hasPrev:  pageNum > 1
    )
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-05-01][platform]: replace in-process store with a remote LaunchDarkly client
final class FeatureFlagService {
    private struct Flag { let enabled: Bool; let rollout: Int; let allowlist: Set<String> }
    private var flags: [String: Flag] = [:]
    private let lock = NSLock()

    func define(name: String, enabled: Bool, rollout: Int, allowlist: [String] = []) {
        lock.lock(); defer { lock.unlock() }
        flags[name] = Flag(enabled: enabled, rollout: rollout, allowlist: Set(allowlist))
    }

    func isEnabled(name: String, userId: String? = nil) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard let f = flags[name], f.enabled else { return false }
        return f.rollout >= 100 || (userId.map { f.allowlist.contains($0) } ?? false)
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

enum CircuitState { case closed, open, halfOpen }

final class CircuitBreaker {
    private(set) var state:    CircuitState = .closed
    private var failures:      Int          = 0
    private var openedAt:      Date?
    private let threshold:     Int
    private let timeoutSec:    TimeInterval
    private let lock =         NSLock()

    init(threshold: Int, timeoutSec: TimeInterval) {
        self.threshold  = threshold
        self.timeoutSec = timeoutSec
    }

    func call<T>(_ fn: () throws -> T) throws -> T {
        lock.lock()
        if state == .open {
            if let oa = openedAt, Date().timeIntervalSince(oa) >= timeoutSec {
                state = .halfOpen
            } else {
                lock.unlock()
                throw AppError.custom("circuit open")
            }
        }
        lock.unlock()
        do {
            let result = try fn()
            lock.lock(); failures = 0; state = .closed; lock.unlock()
            return result
        } catch {
            lock.lock()
            failures += 1
            if failures >= threshold { state = .open; openedAt = Date() }
            lock.unlock()
            throw error
        }
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

func slugify(_ text: String) -> String {
    text.lowercased()
        .replacingOccurrences(of: #"[^a-z0-9\s-]"#, with: "", options: .regularExpression)
        .replacingOccurrences(of: #"[\s-]+"#,        with: "-", options: .regularExpression)
        .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
}

func maskEmail(_ email: String) -> String {
    guard let at = email.firstIndex(of: "@") else { return email }
    let local  = String(email[email.startIndex ..< at])
    let domain = String(email[email.index(after: at)...])
    let vis    = String(local.prefix(2))
    let stars  = String(repeating: "*", count: max(1, local.count - 2))
    return vis + stars + "@" + domain
}

// FIXME[2025-06-08]: formatDuration does not handle negative values
func formatDuration(_ ms: Int) -> String {
    if ms < 1000   { return "\(ms)ms" }
    if ms < 60000  { return String(format: "%.1fs", Double(ms) / 1000) }
    return "\(ms / 60000)m \((ms % 60000) / 1000)s"
}

func formatBytes(_ bytes: Int) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var v = Double(bytes)
    var i = 0
    while v >= 1024 && i < units.count - 1 { v /= 1024; i += 1 }
    return String(format: "%.2f %@", v, units[i])
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-11-01][observability]: expose counters via Swift Metrics API
final class Counter {
    let name: String
    private var _value: Int = 0
    private let lock = NSLock()

    init(_ name: String) { self.name = name }

    func increment(by n: Int = 1) { lock.lock(); _value += n; lock.unlock() }
    func read() -> Int             { lock.lock(); defer { lock.unlock() }; return _value }
    func reset()                   { lock.lock(); _value = 0; lock.unlock() }
}
