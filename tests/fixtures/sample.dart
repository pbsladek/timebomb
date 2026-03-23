// sample.dart — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:math';

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

class Config {
  final String host;
  final int    port;
  final String dbUrl;
  final String jwtSecret;
  final String env;

  const Config({
    this.host      = '0.0.0.0',
    this.port      = 3000,
    this.dbUrl     = 'postgres://localhost/app',
    this.jwtSecret = 'change-me',
    this.env       = 'development',
  });

  factory Config.fromEnvironment() {
    return Config(
      host:      const String.fromEnvironment('HOST',       defaultValue: '0.0.0.0'),
      port:      int.parse(const String.fromEnvironment('PORT', defaultValue: '3000')),
      dbUrl:     const String.fromEnvironment('DB_URL',     defaultValue: 'postgres://localhost/app'),
      jwtSecret: const String.fromEnvironment('JWT_SECRET', defaultValue: 'change-me'),
      env:       const String.fromEnvironment('APP_ENV',    defaultValue: 'development'),
    );
  }

  bool get isProduction => env == 'production';
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

sealed class Result<T> {
  const Result();
  bool get isOk  => this is Ok<T>;
  bool get isErr => this is Err<T>;
}

final class Ok<T> extends Result<T> {
  final T value;
  const Ok(this.value);
}

final class Err<T> extends Result<T> {
  final String error;
  const Err(this.error);
}

T unwrap<T>(Result<T> r) {
  return switch (r) {
    Ok<T>(:final value) => value,
    Err<T>(:final error) => throw StateError('unwrap on Err: $error'),
  };
}

T unwrapOr<T>(Result<T> r, T def) => r is Ok<T> ? (r as Ok<T>).value : def;

Result<U> mapResult<T, U>(Result<T> r, U Function(T) f) => switch (r) {
  Ok<T>(:final value) => Ok(f(value)),
  Err<T>(:final error) => Err(error),
};

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

// TODO[2020-06-15]: replace with a json_schema or built_value validation approach
class ValidationFailure {
  final String field;
  final String message;
  const ValidationFailure({required this.field, required this.message});
}

ValidationFailure? validateRequired(String field, String? value) {
  if (value == null || value.isEmpty) {
    return ValidationFailure(field: field, message: 'is required');
  }
  return null;
}

ValidationFailure? validateEmail(String field, String value) {
  final re = RegExp(r'^[^\s@]+@[^\s@]+\.[^\s@]+$');
  if (re.hasMatch(value)) return null;
  return ValidationFailure(field: field, message: 'must be a valid email address');
}

ValidationFailure? validateMinLength(String field, String value, int min) {
  if (value.length >= min) return null;
  return ValidationFailure(field: field, message: 'must be at least $min characters');
}

List<ValidationFailure> collectFailures(List<ValidationFailure?> checks) =>
    checks.whereType<ValidationFailure>().toList();

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2019-02-28]: HashMap with no eviction; add LRU + TTL sweep before going to prod
class Cache<K, V> {
  final _store = <K, ({V value, DateTime expiresAt})>{};

  V? get(K key) {
    final e = _store[key];
    if (e == null) return null;
    if (DateTime.now().isBefore(e.expiresAt)) return e.value;
    _store.remove(key);
    return null;
  }

  void set(K key, V value, Duration ttl) {
    _store[key] = (value: value, expiresAt: DateTime.now().add(ttl));
  }

  void delete(K key) => _store.remove(key);

  V getOrSet(K key, Duration ttl, V Function() fn) {
    final v = get(key);
    if (v != null) return v;
    final result = fn();
    set(key, result, ttl);
    return result;
  }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

// FIXME[2021-10-01]: no distributed state; move to a Redis-backed token bucket before scaling
class RateResult {
  final bool allowed;
  final int  remaining;
  final int  retryAfter;
  const RateResult({required this.allowed, required this.remaining, required this.retryAfter});
}

class RateLimiter {
  final int _windowSec;
  final int _maxRequests;
  final _store = <String, ({int count, DateTime resetAt})>{};

  RateLimiter({required int windowSec, required int maxRequests})
      : _windowSec   = windowSec,
        _maxRequests = maxRequests;

  RateResult check(String key) {
    final now   = DateTime.now();
    var   entry = _store[key];
    if (entry == null || !now.isBefore(entry.resetAt)) {
      entry = (count: 0, resetAt: now.add(Duration(seconds: _windowSec)));
      _store[key] = entry;
    }
    final newCount = entry.count + 1;
    _store[key]    = (count: newCount, resetAt: entry.resetAt);
    final allowed  = newCount <= _maxRequests;
    final after    = allowed ? 0 : entry.resetAt.difference(now).inSeconds;
    return RateResult(
      allowed:    allowed,
      remaining:  allowed ? _maxRequests - newCount : 0,
      retryAfter: after,
    );
  }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

class Page<T> {
  final List<T> items;
  final int     total;
  final int     pageNum;
  final int     pageSize;
  final bool    hasNext;
  final bool    hasPrev;

  const Page({
    required this.items,
    required this.total,
    required this.pageNum,
    required this.pageSize,
    required this.hasNext,
    required this.hasPrev,
  });
}

Page<T> paginate<T>(List<T> items, int pageNum, int pageSize) {
  final offset = max(0, (pageNum - 1) * pageSize);
  final chunk  = items.skip(offset).take(pageSize).toList();
  return Page(
    items:    chunk,
    total:    items.length,
    pageNum:  pageNum,
    pageSize: pageSize,
    hasNext:  offset + chunk.length < items.length,
    hasPrev:  pageNum > 1,
  );
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-09-01][platform]: replace in-process map with a remote LaunchDarkly client
class FeatureFlagService {
  final _flags = <String, ({bool enabled, int rollout, Set<String> allowlist})>{};

  void define(String name, {required bool enabled, required int rollout, List<String> allowlist = const []}) {
    _flags[name] = (enabled: enabled, rollout: rollout, allowlist: allowlist.toSet());
  }

  bool isEnabled(String name, {String? userId}) {
    final f = _flags[name];
    if (f == null || !f.enabled) return false;
    return f.rollout >= 100 || (userId != null && f.allowlist.contains(userId));
  }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

String slugify(String text) {
  return text
      .toLowerCase()
      .replaceAll(RegExp(r'[^a-z0-9\s-]'), '')
      .replaceAll(RegExp(r'[\s-]+'), '-')
      .replaceAll(RegExp(r'^-+|-+$'), '');
}

String maskEmail(String email) {
  final at = email.indexOf('@');
  if (at < 0) return email;
  final local  = email.substring(0, at);
  final domain = email.substring(at + 1);
  final vis    = local.substring(0, min(2, local.length));
  final stars  = '*' * max(1, local.length - 2);
  return '$vis$stars@$domain';
}

// FIXME[2025-06-10]: formatDuration does not handle negative values
String formatDuration(int ms) {
  if (ms < 1000)  return '${ms}ms';
  if (ms < 60000) return '${(ms / 1000).toStringAsFixed(1)}s';
  return '${ms ~/ 60000}m ${(ms % 60000) ~/ 1000}s';
}

String formatBytes(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var v = bytes.toDouble();
  var i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return '${v.toStringAsFixed(2)} ${units[i]}';
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-03-01][observability]: wire up dart:developer Timeline and expose Prometheus metrics
class Counter {
  final String name;
  int _value = 0;

  Counter(this.name);

  void increment([int by = 1]) => _value += by;
  int  read()                   => _value;
  void reset()                  => _value = 0;
}

class MetricsRegistry {
  final _counters = <String, Counter>{};

  Counter counter(String name) => _counters.putIfAbsent(name, () => Counter(name));

  Map<String, int> snapshot() =>
      {for (final e in _counters.entries) e.key: e.value.read()};
}

// REMOVEME[2018-05-12]: legacy metrics shim — remove after all callers migrate to MetricsRegistry
void legacyRecord(String name, int value) {}
