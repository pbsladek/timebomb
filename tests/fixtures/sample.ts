// @ts-nocheck — fixture file; not compiled, type checking disabled intentionally.
/**
 * sample.ts — fixture file for timebomb scanner tests.
 *
 * Annotation inventory (hardcoded dates, never relative to today):
 *   Expired        (2018–2021): 6
 *   Expiring-soon  (2025-06):   2
 *   Future / OK    (2088/2099): 4
 */

import crypto from "crypto";
import { EventEmitter } from "events";
import fs from "fs";
import http from "http";
import path from "path";

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

export type HttpMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS";
export type StatusCode = 200 | 201 | 204 | 400 | 401 | 403 | 404 | 409 | 422 | 429 | 500 | 503;

export interface Headers {
  [key: string]: string | string[];
}

export interface ParsedQuery {
  [key: string]: string | string[] | undefined;
}

export interface RequestContext {
  id: string;
  method: HttpMethod;
  path: string;
  headers: Headers;
  query: ParsedQuery;
  body: unknown;
  startedAt: number;
  user?: AuthenticatedUser;
  traceId?: string;
}

export interface ResponseContext {
  statusCode: StatusCode;
  headers: Headers;
  body: unknown;
  sentAt?: number;
}

export interface AuthenticatedUser {
  id: string;
  email: string;
  roles: Role[];
  sessionId: string;
}

export type Role = "admin" | "moderator" | "user" | "service";

export interface PaginationParams {
  page: number;
  limit: number;
  sortBy?: string;
  sortDir?: "asc" | "desc";
}

export interface PaginatedResult<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
  hasNext: boolean;
  hasPrev: boolean;
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export class AppError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly statusCode: StatusCode = 500,
    public readonly details?: unknown,
  ) {
    super(message);
    this.name = "AppError";
  }
}

export class ValidationError extends AppError {
  constructor(message: string, details?: ValidationFailure[]) {
    super("VALIDATION_ERROR", message, 422, details);
    this.name = "ValidationError";
  }
}

export class AuthError extends AppError {
  constructor(message = "Unauthorized") {
    super("AUTH_ERROR", message, 401);
    this.name = "AuthError";
  }
}

export class ForbiddenError extends AppError {
  constructor(message = "Forbidden") {
    super("FORBIDDEN", message, 403);
    this.name = "ForbiddenError";
  }
}

export class NotFoundError extends AppError {
  constructor(resource: string) {
    super("NOT_FOUND", `${resource} not found`, 404);
    this.name = "NotFoundError";
  }
}

export class ConflictError extends AppError {
  constructor(message: string) {
    super("CONFLICT", message, 409);
    this.name = "ConflictError";
  }
}

export class RateLimitError extends AppError {
  constructor(public readonly retryAfter: number) {
    super("RATE_LIMITED", "Too many requests", 429);
    this.name = "RateLimitError";
  }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

export interface ValidationFailure {
  field: string;
  message: string;
  value?: unknown;
}

export type Validator<T> = (value: T) => ValidationFailure | null;

export function required(field: string): Validator<unknown> {
  return (value) =>
    value === null || value === undefined || value === ""
      ? { field, message: "is required" }
      : null;
}

export function minLength(field: string, min: number): Validator<string> {
  return (value) =>
    typeof value === "string" && value.length < min
      ? { field, message: `must be at least ${min} characters`, value }
      : null;
}

export function maxLength(field: string, max: number): Validator<string> {
  return (value) =>
    typeof value === "string" && value.length > max
      ? { field, message: `must be at most ${max} characters`, value }
      : null;
}

export function isEmail(field: string): Validator<string> {
  // TODO[2020-06-01]: replace with a proper RFC 5322 parser
  const re = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
  return (value) =>
    typeof value === "string" && !re.test(value)
      ? { field, message: "must be a valid email address", value }
      : null;
}

export function isUuid(field: string): Validator<string> {
  const re = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  return (value) =>
    typeof value === "string" && !re.test(value)
      ? { field, message: "must be a valid UUID v4", value }
      : null;
}

export function validate<T>(value: T, validators: Validator<T>[]): ValidationFailure[] {
  return validators.map((v) => v(value)).filter((f): f is ValidationFailure => f !== null);
}

export function validateObject(
  obj: Record<string, unknown>,
  schema: Record<string, Validator<unknown>[]>,
): ValidationFailure[] {
  return Object.entries(schema).flatMap(([field, validators]) =>
    validate(obj[field], validators),
  );
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

export interface ServerConfig {
  host: string;
  port: number;
  trustProxy: boolean;
  requestTimeout: number;
  maxBodySize: number;
}

export interface DatabaseConfig {
  host: string;
  port: number;
  name: string;
  user: string;
  password: string;
  poolMin: number;
  poolMax: number;
  idleTimeout: number;
  connectionTimeout: number;
  ssl: boolean;
}

export interface CacheConfig {
  host: string;
  port: number;
  password?: string;
  db: number;
  keyPrefix: string;
  defaultTtl: number;
}

export interface AuthConfig {
  jwtSecret: string;
  jwtExpiry: number;
  refreshExpiry: number;
  bcryptRounds: number;
}

export interface RateLimitConfig {
  windowMs: number;
  maxRequests: number;
  skipSuccessfulRequests: boolean;
}

export interface AppConfig {
  env: "development" | "staging" | "production";
  server: ServerConfig;
  database: DatabaseConfig;
  cache: CacheConfig;
  auth: AuthConfig;
  rateLimit: RateLimitConfig;
}

export function loadConfig(): AppConfig {
  const env = (process.env.NODE_ENV ?? "development") as AppConfig["env"];
  return {
    env,
    server: {
      host: process.env.HOST ?? "0.0.0.0",
      port: parseInt(process.env.PORT ?? "3000", 10),
      trustProxy: process.env.TRUST_PROXY === "true",
      requestTimeout: parseInt(process.env.REQUEST_TIMEOUT ?? "30000", 10),
      maxBodySize: parseInt(process.env.MAX_BODY_SIZE ?? "1048576", 10),
    },
    database: {
      host: process.env.DB_HOST ?? "localhost",
      port: parseInt(process.env.DB_PORT ?? "5432", 10),
      name: process.env.DB_NAME ?? "app",
      user: process.env.DB_USER ?? "postgres",
      password: process.env.DB_PASSWORD ?? "",
      poolMin: parseInt(process.env.DB_POOL_MIN ?? "2", 10),
      poolMax: parseInt(process.env.DB_POOL_MAX ?? "10", 10),
      idleTimeout: parseInt(process.env.DB_IDLE_TIMEOUT ?? "10000", 10),
      connectionTimeout: parseInt(process.env.DB_CONN_TIMEOUT ?? "5000", 10),
      ssl: process.env.DB_SSL === "true",
    },
    cache: {
      host: process.env.REDIS_HOST ?? "localhost",
      port: parseInt(process.env.REDIS_PORT ?? "6379", 10),
      password: process.env.REDIS_PASSWORD,
      db: parseInt(process.env.REDIS_DB ?? "0", 10),
      keyPrefix: process.env.REDIS_PREFIX ?? "app:",
      defaultTtl: parseInt(process.env.CACHE_TTL ?? "300", 10),
    },
    auth: {
      jwtSecret: process.env.JWT_SECRET ?? "change-me",
      jwtExpiry: parseInt(process.env.JWT_EXPIRY ?? "3600", 10),
      refreshExpiry: parseInt(process.env.REFRESH_EXPIRY ?? "604800", 10),
      bcryptRounds: parseInt(process.env.BCRYPT_ROUNDS ?? "12", 10),
    },
    rateLimit: {
      windowMs: parseInt(process.env.RATE_WINDOW_MS ?? "60000", 10),
      maxRequests: parseInt(process.env.RATE_MAX ?? "100", 10),
      skipSuccessfulRequests: process.env.RATE_SKIP_SUCCESS === "true",
    },
  };
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface LogEntry {
  level: LogLevel;
  message: string;
  timestamp: string;
  traceId?: string;
  [key: string]: unknown;
}

export interface Logger {
  debug(message: string, meta?: Record<string, unknown>): void;
  info(message: string, meta?: Record<string, unknown>): void;
  warn(message: string, meta?: Record<string, unknown>): void;
  error(message: string, meta?: Record<string, unknown>): void;
  child(bindings: Record<string, unknown>): Logger;
}

export class ConsoleLogger implements Logger {
  private bindings: Record<string, unknown>;

  constructor(bindings: Record<string, unknown> = {}) {
    this.bindings = bindings;
  }

  private log(level: LogLevel, message: string, meta?: Record<string, unknown>): void {
    const entry: LogEntry = {
      level,
      message,
      timestamp: new Date().toISOString(),
      ...this.bindings,
      ...meta,
    };
    // FIXME[2019-03-01]: stream to stdout for structured log ingestion rather than console
    console.log(JSON.stringify(entry));
  }

  debug(message: string, meta?: Record<string, unknown>): void { this.log("debug", message, meta); }
  info(message: string, meta?: Record<string, unknown>): void  { this.log("info",  message, meta); }
  warn(message: string, meta?: Record<string, unknown>): void  { this.log("warn",  message, meta); }
  error(message: string, meta?: Record<string, unknown>): void { this.log("error", message, meta); }

  child(bindings: Record<string, unknown>): Logger {
    return new ConsoleLogger({ ...this.bindings, ...bindings });
  }
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

export type NextFn = (err?: Error) => Promise<void> | void;
export type MiddlewareFn = (req: RequestContext, res: ResponseContext, next: NextFn) => Promise<void> | void;

export function compose(...middlewares: MiddlewareFn[]): MiddlewareFn {
  return async (req, res, next) => {
    let index = -1;

    async function dispatch(i: number): Promise<void> {
      if (i <= index) throw new Error("next() called multiple times");
      index = i;
      const fn = i < middlewares.length ? middlewares[i] : next;
      await fn(req, res, () => dispatch(i + 1));
    }

    await dispatch(0);
  };
}

export function requestId(): MiddlewareFn {
  return async (req, _res, next) => {
    req.id = crypto.randomUUID();
    req.traceId = (req.headers["x-trace-id"] as string) ?? req.id;
    await next();
  };
}

export function requestLogger(logger: Logger): MiddlewareFn {
  return async (req, res, next) => {
    const log = logger.child({ requestId: req.id, traceId: req.traceId });
    log.info("request started", { method: req.method, path: req.path });
    await next();
    const duration = Date.now() - req.startedAt;
    log.info("request completed", { statusCode: res.statusCode, duration });
  };
}

export function cors(allowedOrigins: string[]): MiddlewareFn {
  return async (req, res, next) => {
    const origin = req.headers["origin"] as string | undefined;
    if (origin && allowedOrigins.includes(origin)) {
      res.headers["access-control-allow-origin"] = origin;
      res.headers["access-control-allow-credentials"] = "true";
      res.headers["vary"] = "Origin";
    }
    if (req.method === "OPTIONS") {
      res.headers["access-control-allow-methods"] = "GET,POST,PUT,PATCH,DELETE";
      res.headers["access-control-allow-headers"] = "content-type,authorization";
      res.headers["access-control-max-age"] = "86400";
      res.statusCode = 204;
      return;
    }
    await next();
  };
}

export function bodyParser(maxSize: number): MiddlewareFn {
  // TODO[2025-06-10]: add streaming multipart support for file uploads
  return async (req, _res, next) => {
    if (!["POST", "PUT", "PATCH"].includes(req.method)) {
      await next();
      return;
    }
    const ct = req.headers["content-type"] as string ?? "";
    if (!ct.includes("application/json")) {
      await next();
      return;
    }
    if (typeof req.body === "string" && req.body.length > maxSize) {
      throw new AppError("PAYLOAD_TOO_LARGE", "Request body exceeds size limit", 400);
    }
    if (typeof req.body === "string") {
      try {
        req.body = JSON.parse(req.body);
      } catch {
        throw new AppError("INVALID_JSON", "Request body is not valid JSON", 400);
      }
    }
    await next();
  };
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

interface RateLimitEntry {
  count: number;
  resetAt: number;
}

export class RateLimiter {
  private store = new Map<string, RateLimitEntry>();
  private cleanupTimer: ReturnType<typeof setInterval>;

  constructor(private readonly config: RateLimitConfig) {
    // Purge expired entries every window to avoid unbounded memory growth.
    this.cleanupTimer = setInterval(
      () => this.cleanup(),
      config.windowMs,
    );
  }

  check(key: string): { allowed: boolean; remaining: number; retryAfter: number } {
    const now = Date.now();
    let entry = this.store.get(key);

    if (!entry || entry.resetAt <= now) {
      entry = { count: 0, resetAt: now + this.config.windowMs };
      this.store.set(key, entry);
    }

    entry.count++;
    const remaining = Math.max(0, this.config.maxRequests - entry.count);
    const allowed = entry.count <= this.config.maxRequests;
    const retryAfter = allowed ? 0 : Math.ceil((entry.resetAt - now) / 1000);

    return { allowed, remaining, retryAfter };
  }

  middleware(): MiddlewareFn {
    return async (req, res, next) => {
      const key = req.user?.id ?? (req.headers["x-forwarded-for"] as string) ?? "anonymous";
      const result = this.check(key);

      res.headers["x-ratelimit-limit"] = String(this.config.maxRequests);
      res.headers["x-ratelimit-remaining"] = String(result.remaining);

      if (!result.allowed) {
        res.headers["retry-after"] = String(result.retryAfter);
        throw new RateLimitError(result.retryAfter);
      }

      await next();
    };
  }

  private cleanup(): void {
    const now = Date.now();
    for (const [key, entry] of this.store) {
      if (entry.resetAt <= now) this.store.delete(key);
    }
  }

  destroy(): void {
    clearInterval(this.cleanupTimer);
  }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2018-11-01]: in-process cache used as a stand-in; replace with Redis client
export class InProcessCache {
  private store = new Map<string, { value: unknown; expiresAt: number }>();

  async get<T>(key: string): Promise<T | null> {
    const entry = this.store.get(key);
    if (!entry) return null;
    if (entry.expiresAt <= Date.now()) {
      this.store.delete(key);
      return null;
    }
    return entry.value as T;
  }

  async set(key: string, value: unknown, ttlSeconds: number): Promise<void> {
    this.store.set(key, { value, expiresAt: Date.now() + ttlSeconds * 1000 });
  }

  async del(key: string): Promise<void> {
    this.store.delete(key);
  }

  async delPattern(pattern: string): Promise<void> {
    const re = new RegExp("^" + pattern.replace(/\*/g, ".*") + "$");
    for (const key of this.store.keys()) {
      if (re.test(key)) this.store.delete(key);
    }
  }

  async getOrSet<T>(key: string, ttl: number, fn: () => Promise<T>): Promise<T> {
    const cached = await this.get<T>(key);
    if (cached !== null) return cached;
    const value = await fn();
    await this.set(key, value, ttl);
    return value;
  }
}

// ---------------------------------------------------------------------------
// Database (stub — real impl would use pg or similar)
// ---------------------------------------------------------------------------

export interface QueryResult<T> {
  rows: T[];
  rowCount: number;
  duration: number;
}

export interface DbClient {
  query<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<QueryResult<T>>;
  transaction<T>(fn: (client: DbClient) => Promise<T>): Promise<T>;
  release(): void;
}

export interface DbPool {
  connect(): Promise<DbClient>;
  query<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<QueryResult<T>>;
  end(): Promise<void>;
}

// TODO[2099-06-01][platform]: replace stub with actual pg Pool once driver is chosen
export class StubDbPool implements DbPool {
  async connect(): Promise<DbClient> {
    throw new Error("StubDbPool: not connected to a real database");
  }

  async query<T>(_sql: string, _params?: unknown[]): Promise<QueryResult<T>> {
    return { rows: [], rowCount: 0, duration: 0 };
  }

  async end(): Promise<void> {}
}

// ---------------------------------------------------------------------------
// Repository base
// ---------------------------------------------------------------------------

export abstract class BaseRepository<T extends { id: string }> {
  constructor(
    protected readonly db: DbPool,
    protected readonly table: string,
    protected readonly cache?: InProcessCache,
    protected readonly cacheTtl: number = 300,
  ) {}

  protected cacheKey(id: string): string {
    return `${this.table}:${id}`;
  }

  async findById(id: string): Promise<T | null> {
    if (this.cache) {
      const cached = await this.cache.get<T>(this.cacheKey(id));
      if (cached) return cached;
    }

    const result = await this.db.query<T>(
      `SELECT * FROM ${this.table} WHERE id = $1 AND deleted_at IS NULL LIMIT 1`,
      [id],
    );

    const row = result.rows[0] ?? null;
    if (row && this.cache) {
      await this.cache.set(this.cacheKey(id), row, this.cacheTtl);
    }
    return row;
  }

  async findAll(params: PaginationParams): Promise<PaginatedResult<T>> {
    const offset = (params.page - 1) * params.limit;
    const orderCol = params.sortBy ?? "created_at";
    const orderDir = params.sortDir ?? "desc";

    const countResult = await this.db.query<{ total: string }>(
      `SELECT COUNT(*) AS total FROM ${this.table} WHERE deleted_at IS NULL`,
    );

    const total = parseInt(countResult.rows[0]?.total ?? "0", 10);

    const dataResult = await this.db.query<T>(
      `SELECT * FROM ${this.table}
       WHERE deleted_at IS NULL
       ORDER BY ${orderCol} ${orderDir}
       LIMIT $1 OFFSET $2`,
      [params.limit, offset],
    );

    return {
      items: dataResult.rows,
      total,
      page: params.page,
      limit: params.limit,
      hasNext: offset + params.limit < total,
      hasPrev: params.page > 1,
    };
  }

  async save(entity: T): Promise<T> {
    if (this.cache) {
      await this.cache.del(this.cacheKey(entity.id));
    }
    return entity;
  }

  async delete(id: string): Promise<void> {
    await this.db.query(
      `UPDATE ${this.table} SET deleted_at = NOW() WHERE id = $1`,
      [id],
    );
    if (this.cache) {
      await this.cache.del(this.cacheKey(id));
    }
  }
}

// ---------------------------------------------------------------------------
// Domain: Users
// ---------------------------------------------------------------------------

export interface User {
  id: string;
  email: string;
  passwordHash: string;
  displayName: string;
  role: Role;
  emailVerified: boolean;
  lastLoginAt: string | null;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface CreateUserInput {
  email: string;
  password: string;
  displayName: string;
  role?: Role;
}

export interface UpdateUserInput {
  displayName?: string;
  role?: Role;
  emailVerified?: boolean;
}

export class UserRepository extends BaseRepository<User> {
  constructor(db: DbPool, cache?: InProcessCache) {
    super(db, "users", cache, 60);
  }

  async findByEmail(email: string): Promise<User | null> {
    const result = await this.db.query<User>(
      `SELECT * FROM users WHERE email = $1 AND deleted_at IS NULL LIMIT 1`,
      [email.toLowerCase()],
    );
    return result.rows[0] ?? null;
  }

  async updateLastLogin(id: string): Promise<void> {
    await this.db.query(
      `UPDATE users SET last_login_at = NOW(), updated_at = NOW() WHERE id = $1`,
      [id],
    );
    if (this.cache) await this.cache.del(this.cacheKey(id));
  }

  async countByRole(role: Role): Promise<number> {
    const result = await this.db.query<{ count: string }>(
      `SELECT COUNT(*) AS count FROM users WHERE role = $1 AND deleted_at IS NULL`,
      [role],
    );
    return parseInt(result.rows[0]?.count ?? "0", 10);
  }
}

// ---------------------------------------------------------------------------
// Domain: Sessions
// ---------------------------------------------------------------------------

export interface Session {
  id: string;
  userId: string;
  token: string;
  refreshToken: string;
  userAgent: string | null;
  ipAddress: string | null;
  expiresAt: string;
  createdAt: string;
}

export class SessionRepository extends BaseRepository<Session> {
  constructor(db: DbPool, cache?: InProcessCache) {
    super(db, "sessions", cache, 30);
  }

  async findByToken(token: string): Promise<Session | null> {
    const result = await this.db.query<Session>(
      `SELECT * FROM sessions WHERE token = $1 AND expires_at > NOW() LIMIT 1`,
      [token],
    );
    return result.rows[0] ?? null;
  }

  async findByRefreshToken(refreshToken: string): Promise<Session | null> {
    const result = await this.db.query<Session>(
      `SELECT * FROM sessions WHERE refresh_token = $1 AND expires_at > NOW() LIMIT 1`,
      [refreshToken],
    );
    return result.rows[0] ?? null;
  }

  async deleteExpired(): Promise<number> {
    // TODO[2021-09-01]: move to a scheduled background job
    const result = await this.db.query(
      `DELETE FROM sessions WHERE expires_at <= NOW()`,
    );
    return result.rowCount;
  }

  async deleteAllForUser(userId: string): Promise<void> {
    await this.db.query(`DELETE FROM sessions WHERE user_id = $1`, [userId]);
    if (this.cache) await this.cache.delPattern(`sessions:*`);
  }
}

// ---------------------------------------------------------------------------
// Auth service
// ---------------------------------------------------------------------------

function hashPassword(password: string, rounds: number): string {
  // Stub: real impl would use bcrypt
  return crypto.createHash("sha256").update(password + rounds).digest("hex");
}

function verifyPassword(password: string, hash: string, rounds: number): boolean {
  return hashPassword(password, rounds) === hash;
}

function generateToken(): string {
  return crypto.randomBytes(48).toString("base64url");
}

export class AuthService {
  constructor(
    private readonly users: UserRepository,
    private readonly sessions: SessionRepository,
    private readonly config: AuthConfig,
    private readonly logger: Logger,
  ) {}

  async register(input: CreateUserInput): Promise<User> {
    const failures = validateObject(input as Record<string, unknown>, {
      email: [required("email"), isEmail("email")],
      password: [required("password"), minLength("password", 8), maxLength("password", 128)],
      displayName: [required("displayName"), minLength("displayName", 2), maxLength("displayName", 64)],
    });

    if (failures.length > 0) throw new ValidationError("Invalid input", failures);

    const existing = await this.users.findByEmail(input.email);
    if (existing) throw new ConflictError("Email address already registered");

    const user: User = {
      id: crypto.randomUUID(),
      email: input.email.toLowerCase(),
      passwordHash: hashPassword(input.password, this.config.bcryptRounds),
      displayName: input.displayName,
      role: input.role ?? "user",
      emailVerified: false,
      lastLoginAt: null,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      deletedAt: null,
    };

    await this.users.save(user);
    this.logger.info("user registered", { userId: user.id, email: user.email });
    return user;
  }

  async login(email: string, password: string, meta: { ip?: string; ua?: string }): Promise<Session> {
    const user = await this.users.findByEmail(email);
    if (!user || !verifyPassword(password, user.passwordHash, this.config.bcryptRounds)) {
      // Deliberate vague message to prevent user enumeration.
      throw new AuthError("Invalid email or password");
    }

    if (user.deletedAt) throw new AuthError("Account is deactivated");

    const expiresAt = new Date(Date.now() + this.config.jwtExpiry * 1000).toISOString();
    const session: Session = {
      id: crypto.randomUUID(),
      userId: user.id,
      token: generateToken(),
      refreshToken: generateToken(),
      userAgent: meta.ua ?? null,
      ipAddress: meta.ip ?? null,
      expiresAt,
      createdAt: new Date().toISOString(),
    };

    await this.sessions.save(session);
    await this.users.updateLastLogin(user.id);
    this.logger.info("user logged in", { userId: user.id });
    return session;
  }

  async logout(token: string): Promise<void> {
    const session = await this.sessions.findByToken(token);
    if (session) {
      await this.sessions.delete(session.id);
      this.logger.info("user logged out", { userId: session.userId });
    }
  }

  async verify(token: string): Promise<AuthenticatedUser> {
    const session = await this.sessions.findByToken(token);
    if (!session) throw new AuthError();

    const user = await this.users.findById(session.userId);
    if (!user || user.deletedAt) throw new AuthError("Account not found or deactivated");

    return {
      id: user.id,
      email: user.email,
      roles: [user.role],
      sessionId: session.id,
    };
  }

  authMiddleware(): MiddlewareFn {
    return async (req, _res, next) => {
      const authHeader = req.headers["authorization"] as string | undefined;
      if (!authHeader?.startsWith("Bearer ")) throw new AuthError();
      const token = authHeader.slice(7);
      req.user = await this.verify(token);
      await next();
    };
  }

  requireRole(...roles: Role[]): MiddlewareFn {
    return async (req, _res, next) => {
      if (!req.user) throw new AuthError();
      const hasRole = req.user.roles.some((r) => roles.includes(r));
      if (!hasRole) throw new ForbiddenError();
      await next();
    };
  }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

type RouteHandler = (req: RequestContext) => Promise<unknown>;

interface Route {
  method: HttpMethod;
  pattern: RegExp;
  paramNames: string[];
  middlewares: MiddlewareFn[];
  handler: RouteHandler;
}

export class Router {
  private routes: Route[] = [];

  private addRoute(
    method: HttpMethod,
    path: string,
    middlewares: MiddlewareFn[],
    handler: RouteHandler,
  ): this {
    const paramNames: string[] = [];
    const pattern = new RegExp(
      "^" + path.replace(/:([a-z_]+)/gi, (_, name) => { paramNames.push(name); return "([^/]+)"; }) + "$",
    );
    this.routes.push({ method, pattern, paramNames, middlewares, handler });
    return this;
  }

  get(path: string, ...args: [...MiddlewareFn[], RouteHandler]): this {
    const handler = args.pop() as RouteHandler;
    return this.addRoute("GET", path, args as MiddlewareFn[], handler);
  }

  post(path: string, ...args: [...MiddlewareFn[], RouteHandler]): this {
    const handler = args.pop() as RouteHandler;
    return this.addRoute("POST", path, args as MiddlewareFn[], handler);
  }

  put(path: string, ...args: [...MiddlewareFn[], RouteHandler]): this {
    const handler = args.pop() as RouteHandler;
    return this.addRoute("PUT", path, args as MiddlewareFn[], handler);
  }

  patch(path: string, ...args: [...MiddlewareFn[], RouteHandler]): this {
    const handler = args.pop() as RouteHandler;
    return this.addRoute("PATCH", path, args as MiddlewareFn[], handler);
  }

  delete(path: string, ...args: [...MiddlewareFn[], RouteHandler]): this {
    const handler = args.pop() as RouteHandler;
    return this.addRoute("DELETE", path, args as MiddlewareFn[], handler);
  }

  match(method: HttpMethod, pathname: string): { route: Route; params: Record<string, string> } | null {
    for (const route of this.routes) {
      if (route.method !== method) continue;
      const m = pathname.match(route.pattern);
      if (!m) continue;
      const params: Record<string, string> = {};
      route.paramNames.forEach((name, i) => { params[name] = m[i + 1]!; });
      return { route, params };
    }
    return null;
  }
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

export type EventPayload = Record<string, unknown>;

export interface DomainEvent<T extends EventPayload = EventPayload> {
  id: string;
  type: string;
  occurredAt: string;
  payload: T;
}

export type EventHandler<T extends EventPayload = EventPayload> = (
  event: DomainEvent<T>,
) => Promise<void> | void;

export class EventBus extends EventEmitter {
  // FIXME[2025-06-08]: add dead-letter queue for failed handlers
  async publish<T extends EventPayload>(type: string, payload: T): Promise<void> {
    const event: DomainEvent<T> = {
      id: crypto.randomUUID(),
      type,
      occurredAt: new Date().toISOString(),
      payload,
    };
    this.emit(type, event);
    this.emit("*", event);
  }

  subscribe<T extends EventPayload>(type: string, handler: EventHandler<T>): () => void {
    const wrapped = (event: DomainEvent<T>) => { void handler(event); };
    this.on(type, wrapped);
    return () => this.off(type, wrapped);
  }
}

// ---------------------------------------------------------------------------
// Job queue (in-process stub)
// ---------------------------------------------------------------------------

export interface Job<T = unknown> {
  id: string;
  type: string;
  payload: T;
  attempts: number;
  maxAttempts: number;
  createdAt: string;
  scheduledAt: string;
}

export type JobHandler<T = unknown> = (job: Job<T>) => Promise<void>;

// TEMP[2021-04-15]: in-process queue; swap for BullMQ before production
export class InProcessQueue {
  private handlers = new Map<string, JobHandler>();
  private queue: Job[] = [];
  private running = false;
  private timer?: ReturnType<typeof setInterval>;

  register<T>(type: string, handler: JobHandler<T>): void {
    this.handlers.set(type, handler as JobHandler);
  }

  async enqueue<T>(type: string, payload: T, delayMs = 0): Promise<Job<T>> {
    const job: Job<T> = {
      id: crypto.randomUUID(),
      type,
      payload,
      attempts: 0,
      maxAttempts: 3,
      createdAt: new Date().toISOString(),
      scheduledAt: new Date(Date.now() + delayMs).toISOString(),
    };
    this.queue.push(job as Job);
    return job;
  }

  start(intervalMs = 100): void {
    if (this.running) return;
    this.running = true;
    this.timer = setInterval(() => { void this.tick(); }, intervalMs);
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearInterval(this.timer);
  }

  private async tick(): Promise<void> {
    const now = new Date().toISOString();
    const ready = this.queue.filter((j) => j.scheduledAt <= now);
    this.queue = this.queue.filter((j) => j.scheduledAt > now);

    for (const job of ready) {
      const handler = this.handlers.get(job.type);
      if (!handler) continue;
      job.attempts++;
      try {
        await handler(job);
      } catch (err) {
        if (job.attempts < job.maxAttempts) {
          job.scheduledAt = new Date(Date.now() + 1000 * Math.pow(2, job.attempts)).toISOString();
          this.queue.push(job);
        }
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

export interface HealthStatus {
  status: "healthy" | "degraded" | "unhealthy";
  version: string;
  uptime: number;
  checks: Record<string, { status: "ok" | "fail"; latency?: number; message?: string }>;
}

export type HealthCheck = () => Promise<{ ok: boolean; latency?: number; message?: string }>;

export class HealthMonitor {
  private checks = new Map<string, HealthCheck>();
  private readonly startedAt = Date.now();

  register(name: string, check: HealthCheck): void {
    this.checks.set(name, check);
  }

  async run(): Promise<HealthStatus> {
    const results: HealthStatus["checks"] = {};
    let anyFail = false;
    let anyDegraded = false;

    await Promise.all(
      Array.from(this.checks.entries()).map(async ([name, check]) => {
        try {
          const r = await check();
          results[name] = { status: r.ok ? "ok" : "fail", latency: r.latency, message: r.message };
          if (!r.ok) anyFail = true;
        } catch (err) {
          results[name] = { status: "fail", message: err instanceof Error ? err.message : "unknown" };
          anyDegraded = true;
        }
      }),
    );

    const status = anyFail ? "unhealthy" : anyDegraded ? "degraded" : "healthy";
    return {
      status,
      version: process.env.APP_VERSION ?? "dev",
      uptime: Math.floor((Date.now() - this.startedAt) / 1000),
      checks: results,
    };
  }
}

// ---------------------------------------------------------------------------
// Metrics (stub counter/histogram)
// ---------------------------------------------------------------------------

export class Counter {
  private value = 0;
  constructor(public readonly name: string, public readonly labels: Record<string, string> = {}) {}
  inc(by = 1): void { this.value += by; }
  reset(): void { this.value = 0; }
  read(): number { return this.value; }
}

export class Histogram {
  // TODO[2088-01-01][observability]: emit to Prometheus pushgateway
  private samples: number[] = [];
  constructor(
    public readonly name: string,
    public readonly buckets: number[] = [5, 10, 25, 50, 100, 250, 500, 1000],
  ) {}

  observe(value: number): void { this.samples.push(value); }

  percentile(p: number): number {
    if (this.samples.length === 0) return 0;
    const sorted = [...this.samples].sort((a, b) => a - b);
    const idx = Math.ceil((p / 100) * sorted.length) - 1;
    return sorted[Math.max(0, idx)]!;
  }

  reset(): void { this.samples = []; }
}

export class MetricsRegistry {
  private counters = new Map<string, Counter>();
  private histograms = new Map<string, Histogram>();

  counter(name: string, labels?: Record<string, string>): Counter {
    const key = `${name}:${JSON.stringify(labels ?? {})}`;
    let c = this.counters.get(key);
    if (!c) { c = new Counter(name, labels); this.counters.set(key, c); }
    return c;
  }

  histogram(name: string, buckets?: number[]): Histogram {
    let h = this.histograms.get(name);
    if (!h) { h = new Histogram(name, buckets); this.histograms.set(name, h); }
    return h;
  }

  snapshot(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [k, c] of this.counters) out[`counter:${k}`] = c.read();
    for (const [k, h] of this.histograms) {
      out[`histogram:${k}`] = { p50: h.percentile(50), p95: h.percentile(95), p99: h.percentile(99) };
    }
    return out;
  }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

export interface FeatureFlag {
  name: string;
  enabled: boolean;
  rolloutPercent: number;
  allowlist: string[];
}

// TODO[2099-03-15][platform]: wire to LaunchDarkly SDK
export class FeatureFlagService {
  private flags = new Map<string, FeatureFlag>();

  define(flag: FeatureFlag): void {
    this.flags.set(flag.name, flag);
  }

  isEnabled(name: string, context?: { userId?: string }): boolean {
    const flag = this.flags.get(name);
    if (!flag) return false;
    if (!flag.enabled) return false;
    if (context?.userId && flag.allowlist.includes(context.userId)) return true;
    if (flag.rolloutPercent >= 100) return true;
    if (flag.rolloutPercent <= 0) return false;

    // Deterministic bucket via hash so same user always gets same result.
    const hash = crypto.createHash("md5").update(`${name}:${context?.userId ?? "anon"}`).digest("hex");
    const bucket = parseInt(hash.slice(0, 8), 16) % 100;
    return bucket < flag.rolloutPercent;
  }
}

// ---------------------------------------------------------------------------
// Application bootstrap
// ---------------------------------------------------------------------------

export interface AppDependencies {
  config: AppConfig;
  logger: Logger;
  db: DbPool;
  cache: InProcessCache;
  events: EventBus;
  metrics: MetricsRegistry;
  queue: InProcessQueue;
  flags: FeatureFlagService;
}

export async function createApp(deps: AppDependencies): Promise<http.Server> {
  const { config, logger, db, cache, events, metrics, queue, flags } = deps;

  // TODO[2019-08-15]: wire up graceful shutdown with SIGTERM/SIGINT handlers
  const userRepo = new UserRepository(db, cache);
  const sessionRepo = new SessionRepository(db, cache);
  const auth = new AuthService(userRepo, sessionRepo, config.auth, logger);
  const rateLimiter = new RateLimiter(config.rateLimit);
  const health = new HealthMonitor();
  const router = new Router();

  health.register("db", async () => {
    const start = Date.now();
    try {
      await db.query("SELECT 1");
      return { ok: true, latency: Date.now() - start };
    } catch (err) {
      return { ok: false, message: err instanceof Error ? err.message : "db error" };
    }
  });

  // Public routes
  router.post("/auth/register", async (req) => {
    const body = req.body as CreateUserInput;
    const user = await auth.register(body);
    metrics.counter("users.registered").inc();
    events.publish("user.registered", { userId: user.id, email: user.email });
    const { passwordHash: _, ...safe } = user;
    return { statusCode: 201, data: safe };
  });

  router.post("/auth/login", async (req) => {
    const body = req.body as { email: string; password: string };
    const ip = req.headers["x-forwarded-for"] as string | undefined;
    const ua = req.headers["user-agent"] as string | undefined;
    const session = await auth.login(body.email, body.password, { ip, ua });
    metrics.counter("sessions.created").inc();
    return { statusCode: 200, data: { token: session.token, refreshToken: session.refreshToken } };
  });

  router.get("/health", async () => health.run());
  router.get("/metrics", async () => metrics.snapshot());

  // Authenticated routes
  router.get("/users", auth.authMiddleware(), auth.requireRole("admin"), async (req) => {
    const page = parseInt((req.query["page"] as string) ?? "1", 10);
    const limit = Math.min(parseInt((req.query["limit"] as string) ?? "20", 10), 100);
    return userRepo.findAll({ page, limit });
  });

  router.get("/users/:id", auth.authMiddleware(), async (req) => {
    const user = await userRepo.findById((req as RequestContext & { params: Record<string, string> }).params["id"]!);
    if (!user) throw new NotFoundError("User");
    const { passwordHash: _, ...safe } = user;
    return safe;
  });

  router.post("/auth/logout", auth.authMiddleware(), async (req) => {
    const token = (req.headers["authorization"] as string).slice(7);
    await auth.logout(token);
    metrics.counter("sessions.destroyed").inc();
    return { statusCode: 204, data: null };
  });

  // Queue setup
  queue.register<{ userId: string }>("send.welcome_email", async (job) => {
    logger.info("sending welcome email", { userId: job.payload.userId });
  });

  events.subscribe<{ userId: string; email: string }>("user.registered", async (evt) => {
    await queue.enqueue("send.welcome_email", { userId: evt.payload.userId }, 0);
  });

  queue.start();

  const server = http.createServer(async (rawReq, rawRes) => {
    const reqCtx: RequestContext = {
      id: crypto.randomUUID(),
      method: (rawReq.method ?? "GET") as HttpMethod,
      path: rawReq.url?.split("?")[0] ?? "/",
      headers: rawReq.headers as Headers,
      query: {},
      body: null,
      startedAt: Date.now(),
    };

    const resCtx: ResponseContext = {
      statusCode: 200,
      headers: { "content-type": "application/json" },
      body: null,
    };

    const global = compose(
      requestId(),
      requestLogger(logger),
      bodyParser(config.server.maxBodySize),
      rateLimiter.middleware(),
    );

    try {
      await global(reqCtx, resCtx, async () => {
        const match = router.match(reqCtx.method, reqCtx.path);
        if (!match) throw new NotFoundError("Route");

        const mw = compose(...match.route.middlewares);
        await mw(reqCtx, resCtx, async () => {
          resCtx.body = await match.route.handler(reqCtx);
        });
      });
    } catch (err) {
      if (err instanceof AppError) {
        resCtx.statusCode = err.statusCode;
        resCtx.body = { error: err.code, message: err.message, details: err.details };
      } else {
        resCtx.statusCode = 500;
        resCtx.body = { error: "INTERNAL_ERROR", message: "An unexpected error occurred" };
        logger.error("unhandled error", { err });
      }
    }

    const body = JSON.stringify(resCtx.body);
    rawRes.writeHead(resCtx.statusCode, {
      ...resCtx.headers,
      "content-length": Buffer.byteLength(body),
    });
    rawRes.end(body);
    metrics.histogram("http.response_time_ms").observe(Date.now() - reqCtx.startedAt);
  });

  return server;
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function retry<T>(
  fn: () => Promise<T>,
  opts: { attempts: number; delayMs: number; backoff?: number },
): Promise<T> {
  let lastErr: Error = new Error("retry called with attempts=0");
  for (let i = 0; i < opts.attempts; i++) {
    try {
      return await fn();
    } catch (err) {
      lastErr = err instanceof Error ? err : new Error(String(err));
      if (i < opts.attempts - 1) {
        const delay = opts.delayMs * Math.pow(opts.backoff ?? 1, i);
        await sleep(delay);
      }
    }
  }
  throw lastErr;
}

export function chunk<T>(arr: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < arr.length; i += size) out.push(arr.slice(i, i + size));
  return out;
}

export function groupBy<T, K extends string | number>(
  arr: T[],
  key: (item: T) => K,
): Record<K, T[]> {
  return arr.reduce(
    (acc, item) => {
      const k = key(item);
      (acc[k] ??= []).push(item);
      return acc;
    },
    {} as Record<K, T[]>,
  );
}

export function pick<T extends object, K extends keyof T>(obj: T, keys: K[]): Pick<T, K> {
  return Object.fromEntries(keys.map((k) => [k, obj[k]])) as Pick<T, K>;
}

export function omit<T extends object, K extends keyof T>(obj: T, keys: K[]): Omit<T, K> {
  const set = new Set(keys as string[]);
  return Object.fromEntries(Object.entries(obj).filter(([k]) => !set.has(k))) as Omit<T, K>;
}

export function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || b === null) return false;
  if (typeof a !== "object" || typeof b !== "object") return false;
  const ka = Object.keys(a as object).sort();
  const kb = Object.keys(b as object).sort();
  if (ka.length !== kb.length) return false;
  return ka.every((k, i) => k === kb[i] && deepEqual((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]));
}

export function memoize<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const cache = new Map<string, R>();
  return (...args) => {
    const key = JSON.stringify(args);
    if (cache.has(key)) return cache.get(key)!;
    const result = fn(...args);
    cache.set(key, result);
    return result;
  };
}

export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (...args) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

export function throttle<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): (...args: A) => void {
  let last = 0;
  return (...args) => {
    const now = Date.now();
    if (now - last >= ms) { last = now; fn(...args); }
  };
}

// REMOVEME[2021-07-20]: legacy snake_case alias kept for backwards-compat with v1 clients
export const deep_equal = deepEqual;

// ---------------------------------------------------------------------------
// File utilities (sync, for CLI tooling only — not for request handlers)
// ---------------------------------------------------------------------------

export function ensureDir(dirPath: string): void {
  fs.mkdirSync(dirPath, { recursive: true });
}

export function readJsonFile<T>(filePath: string): T {
  return JSON.parse(fs.readFileSync(filePath, "utf8")) as T;
}

export function writeJsonFile(filePath: string, value: unknown): void {
  ensureDir(path.dirname(filePath));
  fs.writeFileSync(filePath, JSON.stringify(value, null, 2) + "\n", "utf8");
}

export function fileExists(filePath: string): boolean {
  try { fs.accessSync(filePath); return true; } catch { return false; }
}

// ---------------------------------------------------------------------------
// String utilities
// ---------------------------------------------------------------------------

export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^\w\s-]/g, "")
    .replace(/[\s_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

export function truncate(text: string, maxLen: number, suffix = "…"): string {
  if (text.length <= maxLen) return text;
  return text.slice(0, maxLen - suffix.length) + suffix;
}

export function camelToSnake(str: string): string {
  return str.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
}

export function snakeToCamel(str: string): string {
  return str.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
}

export function capitalize(str: string): string {
  if (!str) return str;
  return str.charAt(0).toUpperCase() + str.slice(1);
}

export function escapeHtml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function stripTags(html: string): string {
  return html.replace(/<[^>]*>/g, "");
}

export function countWords(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

export function maskEmail(email: string): string {
  const [local, domain] = email.split("@");
  if (!local || !domain) return email;
  const visible = local.length > 2 ? local.slice(0, 2) : local.slice(0, 1);
  return `${visible}${"*".repeat(Math.max(1, local.length - 2))}@${domain}`;
}

// ---------------------------------------------------------------------------
// Number utilities
// ---------------------------------------------------------------------------

export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

export function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * clamp(t, 0, 1);
}

export function roundTo(value: number, decimals: number): number {
  const factor = Math.pow(10, decimals);
  return Math.round(value * factor) / factor;
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${roundTo(bytes / Math.pow(1024, i), 2)} ${units[i]}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${roundTo(ms / 1000, 1)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
  return `${Math.floor(ms / 3_600_000)}h ${Math.floor((ms % 3_600_000) / 60_000)}m`;
}

export function randomInt(min: number, max: number): number {
  return Math.floor(Math.random() * (max - min + 1)) + min;
}

export function sum(nums: number[]): number {
  return nums.reduce((a, b) => a + b, 0);
}

export function mean(nums: number[]): number {
  if (nums.length === 0) return 0;
  return sum(nums) / nums.length;
}

export function stddev(nums: number[]): number {
  if (nums.length === 0) return 0;
  const avg = mean(nums);
  return Math.sqrt(mean(nums.map((n) => Math.pow(n - avg, 2))));
}

// ---------------------------------------------------------------------------
// Date utilities
// ---------------------------------------------------------------------------

export function addDays(date: Date, days: number): Date {
  const result = new Date(date);
  result.setDate(result.getDate() + days);
  return result;
}

export function diffDays(a: Date, b: Date): number {
  return Math.round((b.getTime() - a.getTime()) / 86_400_000);
}

export function startOfDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

export function endOfDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(23, 59, 59, 999);
  return d;
}

export function isWeekend(date: Date): boolean {
  const day = date.getDay();
  return day === 0 || day === 6;
}

export function formatIso(date: Date): string {
  return date.toISOString().slice(0, 10);
}

export function parseIso(str: string): Date {
  const d = new Date(str);
  if (isNaN(d.getTime())) throw new Error(`Invalid ISO date: ${str}`);
  return d;
}

// ---------------------------------------------------------------------------
// Async utilities
// ---------------------------------------------------------------------------

export function timeout<T>(promise: Promise<T>, ms: number, message?: string): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error(message ?? `Timed out after ${ms}ms`)),
      ms,
    );
    promise.then(
      (v) => { clearTimeout(timer); resolve(v); },
      (e) => { clearTimeout(timer); reject(e); },
    );
  });
}

export async function allSettledMap<K, V>(
  map: Map<K, Promise<V>>,
): Promise<Map<K, { ok: true; value: V } | { ok: false; error: unknown }>> {
  const out = new Map<K, { ok: true; value: V } | { ok: false; error: unknown }>();
  await Promise.all(
    Array.from(map.entries()).map(async ([k, p]) => {
      try {
        out.set(k, { ok: true, value: await p });
      } catch (e) {
        out.set(k, { ok: false, error: e });
      }
    }),
  );
  return out;
}

export async function mapAsync<T, U>(
  arr: T[],
  fn: (item: T, index: number) => Promise<U>,
  concurrency = Infinity,
): Promise<U[]> {
  if (concurrency === Infinity) return Promise.all(arr.map(fn));

  const results: U[] = new Array(arr.length);
  let i = 0;

  async function worker(): Promise<void> {
    while (i < arr.length) {
      const idx = i++;
      results[idx] = await fn(arr[idx]!, idx);
    }
  }

  const workers = Array.from({ length: Math.min(concurrency, arr.length) }, worker);
  await Promise.all(workers);
  return results;
}

export function createDeferredPromise<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

// ---------------------------------------------------------------------------
// LRU Cache
// ---------------------------------------------------------------------------

export class LRUCache<K, V> {
  private map = new Map<K, V>();

  constructor(private readonly capacity: number) {}

  get(key: K): V | undefined {
    if (!this.map.has(key)) return undefined;
    const value = this.map.get(key)!;
    // Refresh position by deleting and re-inserting.
    this.map.delete(key);
    this.map.set(key, value);
    return value;
  }

  set(key: K, value: V): void {
    if (this.map.has(key)) this.map.delete(key);
    else if (this.map.size >= this.capacity) {
      // Evict the oldest entry (first key in insertion order).
      this.map.delete(this.map.keys().next().value!);
    }
    this.map.set(key, value);
  }

  has(key: K): boolean { return this.map.has(key); }
  delete(key: K): boolean { return this.map.delete(key); }
  clear(): void { this.map.clear(); }
  get size(): number { return this.map.size; }
}

// ---------------------------------------------------------------------------
// Bloom filter (approximate membership)
// ---------------------------------------------------------------------------

export class BloomFilter {
  private bits: Uint8Array;
  private readonly numHashes: number;

  constructor(
    private readonly capacity: number,
    private readonly errorRate: number,
  ) {
    const m = Math.ceil(-capacity * Math.log(errorRate) / Math.pow(Math.log(2), 2));
    this.bits = new Uint8Array(Math.ceil(m / 8));
    this.numHashes = Math.ceil((m / capacity) * Math.log(2));
  }

  private hashes(item: string): number[] {
    const h1 = this.fnv1a(item);
    const h2 = this.fnv1a(item + "\0");
    return Array.from({ length: this.numHashes }, (_, i) =>
      Math.abs((h1 + i * h2) % (this.bits.length * 8)),
    );
  }

  private fnv1a(str: string): number {
    let hash = 2166136261;
    for (let i = 0; i < str.length; i++) {
      hash ^= str.charCodeAt(i);
      hash = (hash * 16777619) >>> 0;
    }
    return hash;
  }

  add(item: string): void {
    for (const h of this.hashes(item)) {
      this.bits[Math.floor(h / 8)]! |= 1 << (h % 8);
    }
  }

  mightContain(item: string): boolean {
    return this.hashes(item).every((h) => (this.bits[Math.floor(h / 8)]! & (1 << (h % 8))) !== 0);
  }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

type CircuitState = "closed" | "open" | "half-open";

export class CircuitBreaker {
  private state: CircuitState = "closed";
  private failures = 0;
  private lastFailureAt = 0;

  constructor(
    private readonly threshold: number,
    private readonly resetTimeoutMs: number,
  ) {}

  async call<T>(fn: () => Promise<T>): Promise<T> {
    if (this.state === "open") {
      if (Date.now() - this.lastFailureAt >= this.resetTimeoutMs) {
        this.state = "half-open";
      } else {
        throw new AppError("CIRCUIT_OPEN", "Service temporarily unavailable", 503);
      }
    }

    try {
      const result = await fn();
      this.onSuccess();
      return result;
    } catch (err) {
      this.onFailure();
      throw err;
    }
  }

  private onSuccess(): void {
    this.failures = 0;
    this.state = "closed";
  }

  private onFailure(): void {
    this.failures++;
    this.lastFailureAt = Date.now();
    if (this.failures >= this.threshold) this.state = "open";
  }

  getState(): CircuitState { return this.state; }
  reset(): void { this.state = "closed"; this.failures = 0; }
}

// ---------------------------------------------------------------------------
// Type guards
// ---------------------------------------------------------------------------

export function isString(v: unknown): v is string { return typeof v === "string"; }
export function isNumber(v: unknown): v is number { return typeof v === "number" && !isNaN(v); }
export function isBoolean(v: unknown): v is boolean { return typeof v === "boolean"; }
export function isArray(v: unknown): v is unknown[] { return Array.isArray(v); }
export function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}
export function isNullish(v: unknown): v is null | undefined { return v === null || v === undefined; }

export function assertNever(x: never): never {
  throw new Error(`Unexpected value: ${JSON.stringify(x)}`);
}

// ---------------------------------------------------------------------------
// Result type (lightweight alternative to throwing)
// ---------------------------------------------------------------------------

export type Result<T, E = Error> = { ok: true; value: T } | { ok: false; error: E };

export function ok<T>(value: T): Result<T, never> { return { ok: true, value }; }
export function err<E>(error: E): Result<never, E> { return { ok: false, error }; }

export function mapResult<T, U, E>(result: Result<T, E>, fn: (value: T) => U): Result<U, E> {
  return result.ok ? ok(fn(result.value)) : result;
}

export function flatMapResult<T, U, E>(
  result: Result<T, E>,
  fn: (value: T) => Result<U, E>,
): Result<U, E> {
  return result.ok ? fn(result.value) : result;
}

export function unwrapOr<T, E>(result: Result<T, E>, fallback: T): T {
  return result.ok ? result.value : fallback;
}

export function unwrap<T, E>(result: Result<T, E>): T {
  if (result.ok) return result.value;
  throw result.error;
}

export async function tryAsync<T>(fn: () => Promise<T>): Promise<Result<T>> {
  try { return ok(await fn()); } catch (e) { return err(e instanceof Error ? e : new Error(String(e))); }
}

// ---------------------------------------------------------------------------
// Observable / reactive cell
// ---------------------------------------------------------------------------

export type Observer<T> = (value: T, prev: T | undefined) => void;

export class Observable<T> {
  private _value: T;
  private observers: Set<Observer<T>> = new Set();

  constructor(initial: T) { this._value = initial; }

  get value(): T { return this._value; }

  set(next: T): void {
    if (next === this._value) return;
    const prev = this._value;
    this._value = next;
    for (const obs of this.observers) obs(next, prev);
  }

  update(fn: (current: T) => T): void { this.set(fn(this._value)); }

  subscribe(observer: Observer<T>): () => void {
    this.observers.add(observer);
    return () => this.observers.delete(observer);
  }

  map<U>(fn: (value: T) => U): Observable<U> {
    const derived = new Observable(fn(this._value));
    this.subscribe((v) => derived.set(fn(v)));
    return derived;
  }
}

// ---------------------------------------------------------------------------
// Trie (prefix tree) for fast prefix lookups
// ---------------------------------------------------------------------------

interface TrieNode {
  children: Map<string, TrieNode>;
  isEnd: boolean;
  value?: unknown;
}

export class Trie {
  private root: TrieNode = { children: new Map(), isEnd: false };

  insert(word: string, value?: unknown): void {
    let node = this.root;
    for (const ch of word) {
      if (!node.children.has(ch)) node.children.set(ch, { children: new Map(), isEnd: false });
      node = node.children.get(ch)!;
    }
    node.isEnd = true;
    node.value = value;
  }

  search(word: string): { found: boolean; value?: unknown } {
    let node = this.root;
    for (const ch of word) {
      if (!node.children.has(ch)) return { found: false };
      node = node.children.get(ch)!;
    }
    return { found: node.isEnd, value: node.value };
  }

  startsWith(prefix: string): boolean {
    let node = this.root;
    for (const ch of prefix) {
      if (!node.children.has(ch)) return false;
      node = node.children.get(ch)!;
    }
    return true;
  }

  collectWithPrefix(prefix: string): string[] {
    let node = this.root;
    for (const ch of prefix) {
      if (!node.children.has(ch)) return [];
      node = node.children.get(ch)!;
    }
    const results: string[] = [];
    this.dfs(node, prefix, results);
    return results;
  }

  private dfs(node: TrieNode, current: string, results: string[]): void {
    if (node.isEnd) results.push(current);
    for (const [ch, child] of node.children) this.dfs(child, current + ch, results);
  }
}

// ---------------------------------------------------------------------------
// Environment helpers
// ---------------------------------------------------------------------------

export function requireEnv(name: string): string {
  const val = process.env[name];
  if (!val) throw new Error(`Required environment variable ${name} is not set`);
  return val;
}

export function optionalEnv(name: string, fallback: string): string {
  return process.env[name] ?? fallback;
}

export function boolEnv(name: string, fallback = false): boolean {
  const val = process.env[name];
  if (val === undefined) return fallback;
  return val === "1" || val.toLowerCase() === "true";
}

export function intEnv(name: string, fallback: number): number {
  const val = process.env[name];
  if (!val) return fallback;
  const n = parseInt(val, 10);
  if (isNaN(n)) throw new Error(`Environment variable ${name} must be an integer, got: ${val}`);
  return n;
}
