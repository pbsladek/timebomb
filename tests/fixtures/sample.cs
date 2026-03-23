// sample.cs — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace Sample;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

public class AppException : Exception
{
    public string Code       { get; }
    public int    StatusCode { get; }
    public object Details    { get; }

    public AppException(string code, string message, int statusCode = 500, object details = null)
        : base(message)
    {
        Code       = code;
        StatusCode = statusCode;
        Details    = details;
    }
}

public class ValidationException : AppException
{
    public ValidationException(string message, object details = null)
        : base("VALIDATION_ERROR", message, 422, details) { }
}

public class NotFoundException : AppException
{
    public NotFoundException(string resource)
        : base("NOT_FOUND", $"{resource} not found", 404) { }
}

public class ConflictException : AppException
{
    public ConflictException(string message)
        : base("CONFLICT", message, 409) { }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

public abstract record Result<T>
{
    public record Ok(T Value)    : Result<T>;
    public record Err(string Message) : Result<T>;

    public bool IsOk  => this is Ok;
    public bool IsErr => this is Err;

    public Result<U> Map<U>(Func<T, U> fn) => this switch
    {
        Ok ok   => new Result<U>.Ok(fn(ok.Value)),
        Err err => new Result<U>.Err(err.Message),
        _       => throw new InvalidOperationException()
    };

    public T UnwrapOr(T fallback) => this is Ok ok ? ok.Value : fallback;
}

public static class Result
{
    public static Result<T> Ok<T>(T value)        => new Result<T>.Ok(value);
    public static Result<T> Err<T>(string message) => new Result<T>.Err(message);

    public static async Task<Result<T>> TryAsync<T>(Func<Task<T>> fn)
    {
        try   { return Ok(await fn()); }
        catch (Exception ex) { return Err<T>(ex.Message); }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

public record ValidationFailure(string Field, string Message, string Value = null);

public static class Validate
{
    // TODO[2020-11-01]: replace with FluentValidation or DataAnnotations
    private static readonly Regex EmailRe = new(@"^[^\s@]+@[^\s@]+\.[^\s@]+$", RegexOptions.Compiled);

    public static ValidationFailure Required(string field, string value) =>
        string.IsNullOrWhiteSpace(value)
            ? new ValidationFailure(field, "is required")
            : null;

    public static ValidationFailure Email(string field, string value) =>
        !EmailRe.IsMatch(value ?? "")
            ? new ValidationFailure(field, "must be a valid email address", value)
            : null;

    public static ValidationFailure MinLength(string field, string value, int min) =>
        (value?.Length ?? 0) < min
            ? new ValidationFailure(field, $"must be at least {min} characters", value)
            : null;

    public static ValidationFailure MaxLength(string field, string value, int max) =>
        (value?.Length ?? 0) > max
            ? new ValidationFailure(field, $"must be at most {max} characters", value)
            : null;

    public static IReadOnlyList<ValidationFailure> Collect(params ValidationFailure[] failures) =>
        failures.Where(f => f is not null).ToList();
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

public record Config(
    string Host,
    int    Port,
    string DbConnectionString,
    string JwtSecret,
    int    JwtExpirySec,
    int    CacheTtlSec,
    int    RateMaxRequests,
    int    RateWindowSec,
    string Environment)
{
    public static Config FromEnvironment() => new(
        Host:               Environment.GetEnvironmentVariable("HOST")          ?? "0.0.0.0",
        Port:               int.Parse(Environment.GetEnvironmentVariable("PORT") ?? "3000"),
        DbConnectionString: Environment.GetEnvironmentVariable("DB_URL")        ?? "Host=localhost;Database=app",
        JwtSecret:          Environment.GetEnvironmentVariable("JWT_SECRET")    ?? "change-me",
        JwtExpirySec:       int.Parse(Environment.GetEnvironmentVariable("JWT_EXPIRY") ?? "3600"),
        CacheTtlSec:        int.Parse(Environment.GetEnvironmentVariable("CACHE_TTL")  ?? "300"),
        RateMaxRequests:    int.Parse(Environment.GetEnvironmentVariable("RATE_MAX")   ?? "100"),
        RateWindowSec:      int.Parse(Environment.GetEnvironmentVariable("RATE_WINDOW") ?? "60"),
        Environment:        Environment.GetEnvironmentVariable("ASPNETCORE_ENVIRONMENT") ?? "Development");

    public bool IsProduction => Environment == "Production";
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2019-06-01]: MemoryCache wrapper; replace with IDistributedCache + Redis
public sealed class Cache<TKey, TValue> : IDisposable where TKey : notnull
{
    private record Entry(TValue Value, DateTimeOffset ExpiresAt);

    private readonly ConcurrentDictionary<TKey, Entry> _store = new();
    private readonly Timer _cleanupTimer;

    public Cache()
    {
        _cleanupTimer = new Timer(_ => Cleanup(), null, TimeSpan.FromMinutes(1), TimeSpan.FromMinutes(1));
    }

    public TValue Get(TKey key)
    {
        if (_store.TryGetValue(key, out var entry) && entry.ExpiresAt > DateTimeOffset.UtcNow)
            return entry.Value;
        _store.TryRemove(key, out _);
        return default;
    }

    public void Set(TKey key, TValue value, TimeSpan ttl) =>
        _store[key] = new Entry(value, DateTimeOffset.UtcNow.Add(ttl));

    public void Delete(TKey key) => _store.TryRemove(key, out _);

    public TValue GetOrSet(TKey key, TimeSpan ttl, Func<TValue> factory)
    {
        var cached = Get(key);
        if (cached is not null) return cached;
        var value = factory();
        Set(key, value, ttl);
        return value;
    }

    private void Cleanup()
    {
        var now = DateTimeOffset.UtcNow;
        foreach (var kvp in _store.Where(k => k.Value.ExpiresAt <= now))
            _store.TryRemove(kvp.Key, out _);
    }

    public void Dispose() => _cleanupTimer.Dispose();
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

public sealed class RateLimiter
{
    private record Entry(int Count, DateTimeOffset ResetAt);
    private readonly ConcurrentDictionary<string, Entry> _store = new();
    private readonly int _windowSec, _maxRequests;

    public RateLimiter(int windowSec, int maxRequests)
    {
        _windowSec   = windowSec;
        _maxRequests = maxRequests;
    }

    public (bool Allowed, int Remaining, int RetryAfterSec) Check(string key)
    {
        var now = DateTimeOffset.UtcNow;
        var entry = _store.AddOrUpdate(key,
            _ => new Entry(1, now.AddSeconds(_windowSec)),
            (_, e) => e.ResetAt > now ? e with { Count = e.Count + 1 } : new Entry(1, now.AddSeconds(_windowSec)));

        var remaining   = Math.Max(0, _maxRequests - entry.Count);
        var allowed     = entry.Count <= _maxRequests;
        var retryAfter  = allowed ? 0 : (int)(entry.ResetAt - now).TotalSeconds;
        return (allowed, remaining, retryAfter);
    }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

public record Page<T>(IReadOnlyList<T> Items, int Total, int PageNum, int PageSize)
{
    public bool HasNext => (PageNum - 1) * PageSize + Items.Count < Total;
    public bool HasPrev => PageNum > 1;
}

public static class Pagination
{
    public static Page<T> Paginate<T>(IEnumerable<T> source, int page, int size)
    {
        var list   = source.ToList();
        var items  = list.Skip((page - 1) * size).Take(size).ToList();
        return new Page<T>(items, list.Count, page, size);
    }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-08-01][platform]: replace with Microsoft.FeatureManagement
public sealed class FeatureFlagService
{
    private record FlagDef(bool Enabled, int RolloutPercent, HashSet<string> Allowlist);
    private readonly ConcurrentDictionary<string, FlagDef> _flags = new();

    public void Define(string name, bool enabled, int rollout = 100, IEnumerable<string> allowlist = null)
    {
        _flags[name] = new FlagDef(enabled, rollout, new HashSet<string>(allowlist ?? []));
    }

    public bool IsEnabled(string name, string userId = null)
    {
        if (!_flags.TryGetValue(name, out var flag) || !flag.Enabled) return false;
        if (userId is not null && flag.Allowlist.Contains(userId)) return true;
        if (flag.RolloutPercent >= 100) return true;
        if (flag.RolloutPercent <= 0)  return false;

        using var md5    = MD5.Create();
        var hashBytes    = md5.ComputeHash(Encoding.UTF8.GetBytes($"{name}:{userId ?? "anon"}"));
        var bucket       = Math.Abs(BitConverter.ToInt32(hashBytes, 0)) % 100;
        return bucket < flag.RolloutPercent;
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

// FIXME[2021-03-15]: no half-open success threshold; add counter before closing
public sealed class CircuitBreaker
{
    private enum State { Closed, Open, HalfOpen }

    private State _state = State.Closed;
    private int _failures;
    private DateTimeOffset _lastFailure;
    private readonly int _threshold;
    private readonly TimeSpan _resetTimeout;
    private readonly object _lock = new();

    public CircuitBreaker(int threshold, TimeSpan resetTimeout)
    {
        _threshold    = threshold;
        _resetTimeout = resetTimeout;
    }

    public async Task<T> CallAsync<T>(Func<Task<T>> fn)
    {
        lock (_lock)
        {
            if (_state == State.Open)
            {
                if (DateTimeOffset.UtcNow - _lastFailure >= _resetTimeout)
                    _state = State.HalfOpen;
                else
                    throw new AppException("CIRCUIT_OPEN", "Service temporarily unavailable", 503);
            }
        }

        try
        {
            var result = await fn();
            lock (_lock) { _failures = 0; _state = State.Closed; }
            return result;
        }
        catch
        {
            lock (_lock)
            {
                _failures++;
                _lastFailure = DateTimeOffset.UtcNow;
                if (_failures >= _threshold) _state = State.Open;
            }
            throw;
        }
    }

    public string State => _state.ToString();
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

public static class StringUtils
{
    private static readonly Regex SlugRe = new(@"[^a-z0-9]+", RegexOptions.Compiled);

    public static string Slugify(string text) =>
        SlugRe.Replace(text.ToLowerInvariant(), "-").Trim('-');

    public static string MaskEmail(string email)
    {
        var parts = email.Split('@');
        if (parts.Length != 2) return email;
        var local   = parts[0];
        var visible = local.Length > 2 ? local[..2] : local[..1];
        var stars   = new string('*', Math.Max(1, local.Length - 2));
        return $"{visible}{stars}@{parts[1]}";
    }

    public static string Truncate(string s, int maxLen, string suffix = "…") =>
        s.Length <= maxLen ? s : s[..(maxLen - suffix.Length)] + suffix;
}

public static class FormatUtils
{
    public static string Bytes(long bytes)
    {
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        double v = bytes;
        int i = 0;
        while (v >= 1024 && i < units.Length - 1) { v /= 1024; i++; }
        return $"{v:F2} {units[i]}";
    }

    public static string Duration(long ms) => ms switch
    {
        < 1000        => $"{ms}ms",
        < 60_000      => $"{ms / 1000.0:F1}s",
        _             => $"{ms / 60_000}m {ms % 60_000 / 1000}s"
    };
}

// TODO[2025-06-10]: add health-check endpoint wired to DI health checks pipeline
public sealed class HealthMonitor
{
    private readonly Dictionary<string, Func<Task<bool>>> _checks = new();
    private readonly DateTimeOffset _startedAt = DateTimeOffset.UtcNow;

    public void Register(string name, Func<Task<bool>> check) => _checks[name] = check;

    public async Task<object> RunAsync()
    {
        var results = new Dictionary<string, object>();
        foreach (var (name, check) in _checks)
        {
            var start = DateTimeOffset.UtcNow;
            try
            {
                var ok = await check();
                results[name] = new { status = ok ? "ok" : "fail", latencyMs = (DateTimeOffset.UtcNow - start).TotalMilliseconds };
            }
            catch (Exception ex)
            {
                results[name] = new { status = "fail", message = ex.Message };
            }
        }
        var healthy = results.Values.Cast<dynamic>().All(r => r.status == "ok");
        return new { status = healthy ? "healthy" : "unhealthy", uptime = (long)(DateTimeOffset.UtcNow - _startedAt).TotalSeconds, checks = results };
    }
}
