// sample.js — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 6
//   Expiring-soon  (2025-06):   2
//   Future / OK    (2088/2099): 4

"use strict";

const crypto  = require("crypto");
const events  = require("events");
const fs      = require("fs");
const path    = require("path");
const http    = require("http");

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

class AppError extends Error {
  constructor(code, message, statusCode = 500, details = null) {
    super(message);
    this.name       = "AppError";
    this.code       = code;
    this.statusCode = statusCode;
    this.details    = details;
  }
}

class ValidationError extends AppError {
  constructor(message, details) {
    super("VALIDATION_ERROR", message, 422, details);
    this.name = "ValidationError";
  }
}

class AuthError extends AppError {
  constructor(message = "Unauthorized") {
    super("AUTH_ERROR", message, 401);
    this.name = "AuthError";
  }
}

class ForbiddenError extends AppError {
  constructor(message = "Forbidden") {
    super("FORBIDDEN", message, 403);
    this.name = "ForbiddenError";
  }
}

class NotFoundError extends AppError {
  constructor(resource) {
    super("NOT_FOUND", `${resource} not found`, 404);
    this.name = "NotFoundError";
  }
}

class ConflictError extends AppError {
  constructor(message) {
    super("CONFLICT", message, 409);
    this.name = "ConflictError";
  }
}

class RateLimitError extends AppError {
  constructor(retryAfter) {
    super("RATE_LIMITED", "Too many requests", 429);
    this.name       = "RateLimitError";
    this.retryAfter = retryAfter;
  }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

function loadConfig() {
  return {
    host:            process.env.HOST             ?? "0.0.0.0",
    port:            parseInt(process.env.PORT    ?? "3000", 10),
    env:             process.env.NODE_ENV         ?? "development",
    dbHost:          process.env.DB_HOST          ?? "localhost",
    dbPort:          parseInt(process.env.DB_PORT ?? "5432", 10),
    dbName:          process.env.DB_NAME          ?? "app",
    dbUser:          process.env.DB_USER          ?? "postgres",
    dbPassword:      process.env.DB_PASSWORD      ?? "",
    dbPoolMax:       parseInt(process.env.DB_POOL_MAX ?? "10", 10),
    redisHost:       process.env.REDIS_HOST       ?? "localhost",
    redisPort:       parseInt(process.env.REDIS_PORT ?? "6379", 10),
    cacheTtl:        parseInt(process.env.CACHE_TTL  ?? "300", 10),
    jwtSecret:       process.env.JWT_SECRET       ?? "change-me",
    jwtExpiry:       parseInt(process.env.JWT_EXPIRY ?? "3600", 10),
    rateLimitWindow: parseInt(process.env.RATE_WINDOW_MS ?? "60000", 10),
    rateLimitMax:    parseInt(process.env.RATE_MAX       ?? "100", 10),
  };
}

const CONFIG = loadConfig();

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

const LOG_LEVELS = { debug: 0, info: 1, warn: 2, error: 3 };

class Logger {
  constructor(level = "info", bindings = {}) {
    this.level    = LOG_LEVELS[level] ?? 1;
    this.bindings = bindings;
  }

  _log(level, message, meta = {}) {
    if (LOG_LEVELS[level] < this.level) return;
    const entry = {
      level,
      message,
      timestamp: new Date().toISOString(),
      ...this.bindings,
      ...meta,
    };
    process.stdout.write(JSON.stringify(entry) + "\n");
  }

  debug(message, meta)  { this._log("debug", message, meta); }
  info(message, meta)   { this._log("info",  message, meta); }
  warn(message, meta)   { this._log("warn",  message, meta); }
  error(message, meta)  { this._log("error", message, meta); }

  child(bindings) {
    return new Logger(
      Object.keys(LOG_LEVELS)[this.level] ?? "info",
      { ...this.bindings, ...bindings },
    );
  }
}

const logger = new Logger(process.env.LOG_LEVEL ?? "info");

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

// TODO[2020-07-01]: replace with a proper validation library (joi / zod)
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function validateRequired(field, value) {
  if (value === null || value === undefined || value === "") {
    return { field, message: "is required" };
  }
  return null;
}

function validateEmail(field, value) {
  if (typeof value !== "string" || !EMAIL_RE.test(value)) {
    return { field, message: "must be a valid email address", value };
  }
  return null;
}

function validateMinLength(field, value, min) {
  if (typeof value === "string" && value.length < min) {
    return { field, message: `must be at least ${min} characters`, value };
  }
  return null;
}

function validateMaxLength(field, value, max) {
  if (typeof value === "string" && value.length > max) {
    return { field, message: `must be at most ${max} characters`, value };
  }
  return null;
}

function collectFailures(...checks) {
  return checks.filter(Boolean);
}

// ---------------------------------------------------------------------------
// In-process cache
// ---------------------------------------------------------------------------

// HACK[2018-12-01]: in-memory stand-in; wire up Redis before production launch
class Cache {
  constructor() {
    this._store = new Map();
    this._timer = setInterval(() => this._cleanup(), 60_000);
    this._timer.unref?.();
  }

  get(key) {
    const entry = this._store.get(key);
    if (!entry) return null;
    if (entry.expiresAt <= Date.now()) { this._store.delete(key); return null; }
    return entry.value;
  }

  set(key, value, ttlSeconds) {
    this._store.set(key, { value, expiresAt: Date.now() + ttlSeconds * 1000 });
  }

  delete(key) { this._store.delete(key); }

  getOrSet(key, ttlSeconds, fn) {
    const cached = this.get(key);
    if (cached !== null) return Promise.resolve(cached);
    return Promise.resolve(fn()).then((value) => { this.set(key, value, ttlSeconds); return value; });
  }

  flushPattern(pattern) {
    const re = new RegExp("^" + pattern.replace(/\*/g, ".*") + "$");
    for (const key of this._store.keys()) {
      if (re.test(key)) this._store.delete(key);
    }
  }

  _cleanup() {
    const now = Date.now();
    for (const [key, entry] of this._store) {
      if (entry.expiresAt <= now) this._store.delete(key);
    }
  }

  destroy() { clearInterval(this._timer); }
}

const cache = new Cache();

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

class RateLimiter {
  constructor({ windowMs, maxRequests }) {
    this._windowMs    = windowMs;
    this._maxRequests = maxRequests;
    this._store       = new Map();
  }

  check(key) {
    const now = Date.now();
    let entry = this._store.get(key);
    if (!entry || entry.resetAt <= now) {
      entry = { count: 0, resetAt: now + this._windowMs };
      this._store.set(key, entry);
    }
    entry.count++;
    const remaining  = Math.max(0, this._maxRequests - entry.count);
    const allowed    = entry.count <= this._maxRequests;
    const retryAfter = allowed ? 0 : Math.ceil((entry.resetAt - now) / 1000);
    return { allowed, remaining, retryAfter };
  }

  cleanup() {
    const now = Date.now();
    for (const [key, entry] of this._store) {
      if (entry.resetAt <= now) this._store.delete(key);
    }
  }
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

// FIXME[2025-06-08]: add dead-letter queue for handlers that throw
class EventBus extends events.EventEmitter {
  publish(type, payload) {
    const event = {
      id:         crypto.randomUUID(),
      type,
      occurredAt: new Date().toISOString(),
      payload,
    };
    this.emit(type, event);
    this.emit("*", event);
    return event;
  }

  subscribe(type, handler) {
    this.on(type, handler);
    return () => this.off(type, handler);
  }
}

const eventBus = new EventBus();

// ---------------------------------------------------------------------------
// LRU Cache
// ---------------------------------------------------------------------------

class LRUCache {
  constructor(capacity) {
    this._capacity = capacity;
    this._map      = new Map();
  }

  get(key) {
    if (!this._map.has(key)) return undefined;
    const value = this._map.get(key);
    this._map.delete(key);
    this._map.set(key, value);
    return value;
  }

  set(key, value) {
    if (this._map.has(key)) this._map.delete(key);
    else if (this._map.size >= this._capacity) {
      this._map.delete(this._map.keys().next().value);
    }
    this._map.set(key, value);
  }

  has(key)    { return this._map.has(key); }
  delete(key) { return this._map.delete(key); }
  get size()  { return this._map.size; }
  clear()     { this._map.clear(); }
}

// ---------------------------------------------------------------------------
// Job queue (in-process stub)
// ---------------------------------------------------------------------------

// TEMP[2021-05-01]: swap for BullMQ once Redis is provisioned
class Queue {
  constructor() {
    this._handlers = new Map();
    this._queue    = [];
    this._timer    = null;
  }

  register(type, handler) {
    this._handlers.set(type, handler);
    return this;
  }

  enqueue(type, payload, delayMs = 0) {
    const job = {
      id:          crypto.randomUUID(),
      type,
      payload,
      attempts:    0,
      maxAttempts: 3,
      runAt:       Date.now() + delayMs,
    };
    this._queue.push(job);
    return job;
  }

  start(intervalMs = 100) {
    if (this._timer) return;
    this._timer = setInterval(() => this._tick(), intervalMs);
    this._timer.unref?.();
  }

  stop() {
    if (this._timer) { clearInterval(this._timer); this._timer = null; }
  }

  async _tick() {
    const now  = Date.now();
    const ready = this._queue.filter((j) => j.runAt <= now);
    this._queue  = this._queue.filter((j) => j.runAt > now);

    for (const job of ready) {
      const handler = this._handlers.get(job.type);
      if (!handler) continue;
      job.attempts++;
      try {
        await handler(job);
      } catch (err) {
        logger.error("job failed", { jobId: job.id, type: job.type, error: err.message });
        if (job.attempts < job.maxAttempts) {
          job.runAt = Date.now() + 1000 * Math.pow(2, job.attempts);
          this._queue.push(job);
        }
      }
    }
  }
}

const queue = new Queue();

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

class CircuitBreaker {
  constructor(threshold, resetTimeoutMs) {
    this._threshold      = threshold;
    this._resetTimeoutMs = resetTimeoutMs;
    this._state          = "closed";
    this._failures       = 0;
    this._lastFailureAt  = null;
  }

  async call(fn) {
    if (this._state === "open") {
      const elapsed = Date.now() - this._lastFailureAt;
      if (elapsed >= this._resetTimeoutMs) {
        this._state = "half-open";
      } else {
        throw new AppError("CIRCUIT_OPEN", "Service temporarily unavailable", 503);
      }
    }
    try {
      const result = await fn();
      this._onSuccess();
      return result;
    } catch (err) {
      this._onFailure();
      throw err;
    }
  }

  _onSuccess() { this._failures = 0; this._state = "closed"; }

  _onFailure() {
    this._failures++;
    this._lastFailureAt = Date.now();
    if (this._failures >= this._threshold) this._state = "open";
  }

  get state() { return this._state; }
  reset()     { this._state = "closed"; this._failures = 0; }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-02-01][platform]: wire to LaunchDarkly Node.js SDK
class FeatureFlagService {
  constructor() {
    this._flags = new Map();
  }

  define({ name, enabled = true, rolloutPercent = 100, allowlist = [] }) {
    this._flags.set(name, { name, enabled, rolloutPercent, allowlist: new Set(allowlist) });
    return this;
  }

  isEnabled(name, userId = null) {
    const flag = this._flags.get(name);
    if (!flag || !flag.enabled) return false;
    if (userId && flag.allowlist.has(userId)) return true;
    if (flag.rolloutPercent >= 100) return true;
    if (flag.rolloutPercent <= 0) return false;
    const hash   = crypto.createHash("md5").update(`${name}:${userId ?? "anon"}`).digest("hex");
    const bucket = parseInt(hash.slice(0, 8), 16) % 100;
    return bucket < flag.rolloutPercent;
  }
}

// ---------------------------------------------------------------------------
// Health monitor
// ---------------------------------------------------------------------------

class HealthMonitor {
  constructor() {
    this._checks    = new Map();
    this._startedAt = Date.now();
  }

  register(name, check) {
    this._checks.set(name, check);
    return this;
  }

  async run() {
    const results = {};
    await Promise.all(
      Array.from(this._checks.entries()).map(async ([name, check]) => {
        const start = Date.now();
        try {
          const ok = await check();
          results[name] = { status: ok ? "ok" : "fail", latencyMs: Date.now() - start };
        } catch (err) {
          results[name] = { status: "fail", latencyMs: Date.now() - start, message: err.message };
        }
      }),
    );
    const overall = Object.values(results).every((r) => r.status === "ok") ? "healthy" : "unhealthy";
    return {
      status:  overall,
      uptime:  Math.floor((Date.now() - this._startedAt) / 1000),
      version: process.env.APP_VERSION ?? "dev",
      checks:  results,
    };
  }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-03-01][observability]: push snapshots to StatsD / Prometheus
class Metrics {
  constructor() {
    this._counters   = new Map();
    this._histograms = new Map();
  }

  counter(name) {
    if (!this._counters.has(name)) this._counters.set(name, 0);
    return {
      inc: (by = 1) => this._counters.set(name, (this._counters.get(name) ?? 0) + by),
      read: () => this._counters.get(name) ?? 0,
      reset: () => this._counters.set(name, 0),
    };
  }

  histogram(name) {
    if (!this._histograms.has(name)) this._histograms.set(name, []);
    const samples = this._histograms.get(name);
    return {
      observe: (v) => samples.push(v),
      percentile: (p) => {
        if (samples.length === 0) return 0;
        const sorted = [...samples].sort((a, b) => a - b);
        const idx    = Math.ceil((p / 100) * sorted.length) - 1;
        return sorted[Math.max(0, idx)];
      },
      reset: () => samples.splice(0),
    };
  }

  snapshot() {
    const counters   = Object.fromEntries(this._counters);
    const histograms = {};
    for (const [name, samples] of this._histograms) {
      const sorted  = [...samples].sort((a, b) => a - b);
      const pct = (p) => {
        if (sorted.length === 0) return 0;
        return sorted[Math.max(0, Math.ceil((p / 100) * sorted.length) - 1)];
      };
      histograms[name] = { p50: pct(50), p95: pct(95), p99: pct(99) };
    }
    return { counters, histograms };
  }
}

const metrics = new Metrics();

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function retry(fn, { attempts = 3, delayMs = 100, backoff = 2 } = {}) {
  let lastErr = new Error("retry called with attempts=0");
  for (let i = 0; i < attempts; i++) {
    try {
      return await fn();
    } catch (err) {
      lastErr = err;
      if (i < attempts - 1) await sleep(delayMs * Math.pow(backoff, i));
    }
  }
  throw lastErr;
}

function chunk(arr, size) {
  const out = [];
  for (let i = 0; i < arr.length; i += size) out.push(arr.slice(i, i + size));
  return out;
}

function groupBy(arr, keyFn) {
  return arr.reduce((acc, item) => {
    const key = keyFn(item);
    (acc[key] = acc[key] ?? []).push(item);
    return acc;
  }, {});
}

function pick(obj, keys) {
  return Object.fromEntries(keys.filter((k) => k in obj).map((k) => [k, obj[k]]));
}

function omit(obj, keys) {
  const set = new Set(keys);
  return Object.fromEntries(Object.entries(obj).filter(([k]) => !set.has(k)));
}

function deepMerge(base, override) {
  const result = { ...base };
  for (const [key, val] of Object.entries(override)) {
    result[key] =
      val !== null && typeof val === "object" && !Array.isArray(val) &&
      base[key] !== null && typeof base[key] === "object" && !Array.isArray(base[key])
        ? deepMerge(base[key], val)
        : val;
  }
  return result;
}

function memoize(fn) {
  const cache = new Map();
  return function (...args) {
    const key = JSON.stringify(args);
    if (cache.has(key)) return cache.get(key);
    const result = fn.apply(this, args);
    cache.set(key, result);
    return result;
  };
}

function debounce(fn, ms) {
  let timer;
  return function (...args) {
    clearTimeout(timer);
    timer = setTimeout(() => fn.apply(this, args), ms);
  };
}

function throttle(fn, ms) {
  let last = 0;
  return function (...args) {
    const now = Date.now();
    if (now - last >= ms) { last = now; return fn.apply(this, args); }
  };
}

function truncate(str, maxLen, suffix = "…") {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - suffix.length) + suffix;
}

function slugify(text) {
  return text.toLowerCase().replace(/[^\w\s-]/g, "").replace(/[\s_-]+/g, "-").replace(/^-+|-+$/g, "");
}

function maskEmail(email) {
  const [local, domain] = email.split("@");
  if (!local || !domain) return email;
  const visible = local.length > 2 ? local.slice(0, 2) : local.slice(0, 1);
  return `${visible}${"*".repeat(Math.max(1, local.length - 2))}@${domain}`;
}

function formatBytes(bytes) {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i     = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(2)} ${units[i]}`;
}

function formatDuration(ms) {
  if (ms < 1000)    return `${ms}ms`;
  if (ms < 60_000)  return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m ${s}s`;
}

function sum(nums)  { return nums.reduce((a, b) => a + b, 0); }
function mean(nums) { return nums.length === 0 ? 0 : sum(nums) / nums.length; }
function clamp(v, min, max) { return Math.min(Math.max(v, min), max); }

function stddev(nums) {
  if (nums.length === 0) return 0;
  const avg = mean(nums);
  return Math.sqrt(mean(nums.map((n) => Math.pow(n - avg, 2))));
}

function parseIso(str) {
  const d = new Date(str);
  if (isNaN(d.getTime())) throw new Error(`Invalid ISO date: ${str}`);
  return d;
}

function addDays(date, days) {
  const d = new Date(date);
  d.setDate(d.getDate() + days);
  return d;
}

function diffDays(a, b) {
  return Math.round((b.getTime() - a.getTime()) / 86_400_000);
}

// ---------------------------------------------------------------------------
// Observable
// ---------------------------------------------------------------------------

class Observable {
  constructor(initial) {
    this._value     = initial;
    this._observers = new Set();
  }

  get value() { return this._value; }

  set(next) {
    if (next === this._value) return;
    const prev   = this._value;
    this._value  = next;
    for (const obs of this._observers) obs(next, prev);
  }

  update(fn) { this.set(fn(this._value)); }

  subscribe(observer) {
    this._observers.add(observer);
    return () => this._observers.delete(observer);
  }

  map(fn) {
    const derived = new Observable(fn(this._value));
    this.subscribe((v) => derived.set(fn(v)));
    return derived;
  }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

// REMOVEME[2021-08-15]: kept for legacy callers; migrate to plain try/catch
class Result {
  static ok(value)  { return new Result(value, null); }
  static err(error) { return new Result(null, error instanceof Error ? error : new Error(String(error))); }

  static async tryAsync(fn) {
    try { return Result.ok(await fn()); } catch (e) { return Result.err(e); }
  }

  constructor(value, error) {
    this._value = value;
    this._error = error;
  }

  get isOk()    { return this._error === null; }
  get error()   { return this._error; }
  get value()   { return this._value; }

  map(fn)     { return this.isOk ? Result.ok(fn(this._value)) : this; }
  flatMap(fn) { return this.isOk ? fn(this._value) : this; }
  unwrap()    { if (!this.isOk) throw this._error; return this._value; }
  unwrapOr(fallback) { return this.isOk ? this._value : fallback; }
}

// ---------------------------------------------------------------------------
// File utilities (sync, for CLI tooling only)
// ---------------------------------------------------------------------------

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function readJsonFile(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function writeJsonFile(filePath, value) {
  ensureDir(path.dirname(filePath));
  fs.writeFileSync(filePath, JSON.stringify(value, null, 2) + "\n", "utf8");
}

function fileExists(filePath) {
  try { fs.accessSync(filePath); return true; } catch { return false; }
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  // Errors
  AppError, ValidationError, AuthError, ForbiddenError,
  NotFoundError, ConflictError, RateLimitError,
  // Core
  loadConfig, Logger, logger, Cache, cache,
  RateLimiter, EventBus, eventBus, Queue, queue,
  CircuitBreaker, FeatureFlagService, HealthMonitor,
  LRUCache, Metrics, metrics, Observable, Result,
  // Utils
  sleep, retry, chunk, groupBy, pick, omit, deepMerge,
  memoize, debounce, throttle, truncate, slugify,
  maskEmail, formatBytes, formatDuration,
  sum, mean, clamp, stddev, parseIso, addDays, diffDays,
  // Validation
  validateRequired, validateEmail, validateMinLength,
  validateMaxLength, collectFailures,
  // File
  ensureDir, readJsonFile, writeJsonFile, fileExists,
};
