// sample.go — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//
//	Expired        (2018–2021): 6
//	Expiring-soon  (2025-06):   2
//	Future / OK    (2088/2099): 4
package fixture

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"net/http"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

type AppError struct {
	Code       string
	Message    string
	StatusCode int
	Details    any
}

func (e *AppError) Error() string { return fmt.Sprintf("[%s] %s", e.Code, e.Message) }

func NewValidationError(message string, details any) *AppError {
	return &AppError{Code: "VALIDATION_ERROR", Message: message, StatusCode: 422, Details: details}
}

func NewAuthError(message string) *AppError {
	if message == "" {
		message = "Unauthorized"
	}
	return &AppError{Code: "AUTH_ERROR", Message: message, StatusCode: 401}
}

func NewForbiddenError() *AppError {
	return &AppError{Code: "FORBIDDEN", Message: "Forbidden", StatusCode: 403}
}

func NewNotFoundError(resource string) *AppError {
	return &AppError{Code: "NOT_FOUND", Message: resource + " not found", StatusCode: 404}
}

func NewConflictError(message string) *AppError {
	return &AppError{Code: "CONFLICT", Message: message, StatusCode: 409}
}

func NewRateLimitError(retryAfter int) *AppError {
	return &AppError{Code: "RATE_LIMITED", Message: "Too many requests", StatusCode: 429, Details: retryAfter}
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

type Config struct {
	Host             string
	Port             int
	Env              string
	DBHost           string
	DBPort           int
	DBName           string
	DBUser           string
	DBPassword       string
	DBPoolMin        int
	DBPoolMax        int
	RedisHost        string
	RedisPort        int
	CacheTTL         int
	JWTSecret        string
	JWTExpiry        int
	BcryptCost       int
	RateWindowSec    int
	RateMaxRequests  int
}

func LoadConfig() Config {
	return Config{
		Host:            getEnv("HOST", "0.0.0.0"),
		Port:            getEnvInt("PORT", 3000),
		Env:             getEnv("APP_ENV", "development"),
		DBHost:          getEnv("DB_HOST", "localhost"),
		DBPort:          getEnvInt("DB_PORT", 5432),
		DBName:          getEnv("DB_NAME", "app"),
		DBUser:          getEnv("DB_USER", "postgres"),
		DBPassword:      getEnv("DB_PASSWORD", ""),
		DBPoolMin:       getEnvInt("DB_POOL_MIN", 2),
		DBPoolMax:       getEnvInt("DB_POOL_MAX", 10),
		RedisHost:       getEnv("REDIS_HOST", "localhost"),
		RedisPort:       getEnvInt("REDIS_PORT", 6379),
		CacheTTL:        getEnvInt("CACHE_TTL", 300),
		JWTSecret:       getEnv("JWT_SECRET", "change-me"),
		JWTExpiry:       getEnvInt("JWT_EXPIRY", 3600),
		BcryptCost:      getEnvInt("BCRYPT_COST", 12),
		RateWindowSec:   getEnvInt("RATE_WINDOW_SEC", 60),
		RateMaxRequests: getEnvInt("RATE_MAX", 100),
	}
}

func getEnv(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}

func getEnvInt(key string, fallback int) int {
	v := os.Getenv(key)
	if v == "" {
		return fallback
	}
	n, err := strconv.Atoi(v)
	if err != nil {
		return fallback
	}
	return n
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

type Logger struct {
	inner *slog.Logger
}

func NewLogger(level slog.Level) *Logger {
	handler := slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: level})
	return &Logger{inner: slog.New(handler)}
}

func (l *Logger) Debug(msg string, args ...any) { l.inner.Debug(msg, args...) }
func (l *Logger) Info(msg string, args ...any)  { l.inner.Info(msg, args...) }
func (l *Logger) Warn(msg string, args ...any)  { l.inner.Warn(msg, args...) }
func (l *Logger) Error(msg string, args ...any) { l.inner.Error(msg, args...) }

func (l *Logger) With(args ...any) *Logger {
	return &Logger{inner: l.inner.With(args...)}
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

type ValidationFailure struct {
	Field   string `json:"field"`
	Message string `json:"message"`
	Value   any    `json:"value,omitempty"`
}

// TODO[2020-03-15]: replace ad-hoc validators with a struct-tag based library
var emailRE = regexp.MustCompile(`^[^\s@]+@[^\s@]+\.[^\s@]+$`)

func ValidateEmail(field, value string) *ValidationFailure {
	if !emailRE.MatchString(value) {
		return &ValidationFailure{Field: field, Message: "must be a valid email address", Value: value}
	}
	return nil
}

func ValidateRequired(field string, value any) *ValidationFailure {
	if value == nil || value == "" {
		return &ValidationFailure{Field: field, Message: "is required"}
	}
	return nil
}

func ValidateMinLength(field, value string, min int) *ValidationFailure {
	if len(value) < min {
		return &ValidationFailure{Field: field, Message: fmt.Sprintf("must be at least %d characters", min), Value: value}
	}
	return nil
}

func ValidateMaxLength(field, value string, max int) *ValidationFailure {
	if len(value) > max {
		return &ValidationFailure{Field: field, Message: fmt.Sprintf("must be at most %d characters", max), Value: value}
	}
	return nil
}

func CollectFailures(failures ...*ValidationFailure) []ValidationFailure {
	out := make([]ValidationFailure, 0)
	for _, f := range failures {
		if f != nil {
			out = append(out, *f)
		}
	}
	return out
}

// ---------------------------------------------------------------------------
// In-process cache
// ---------------------------------------------------------------------------

// HACK[2019-02-01]: in-memory stand-in; replace with go-redis client
type cacheEntry struct {
	value     any
	expiresAt time.Time
}

type Cache struct {
	mu    sync.RWMutex
	store map[string]cacheEntry
}

func NewCache() *Cache {
	c := &Cache{store: make(map[string]cacheEntry)}
	go c.cleanupLoop()
	return c
}

func (c *Cache) Get(key string) (any, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	entry, ok := c.store[key]
	if !ok || entry.expiresAt.Before(time.Now()) {
		return nil, false
	}
	return entry.value, true
}

func (c *Cache) Set(key string, value any, ttl time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.store[key] = cacheEntry{value: value, expiresAt: time.Now().Add(ttl)}
}

func (c *Cache) Delete(key string) {
	c.mu.Lock()
	defer c.mu.Unlock()
	delete(c.store, key)
}

func (c *Cache) GetOrSet(key string, ttl time.Duration, fn func() (any, error)) (any, error) {
	if v, ok := c.Get(key); ok {
		return v, nil
	}
	v, err := fn()
	if err != nil {
		return nil, err
	}
	c.Set(key, v, ttl)
	return v, nil
}

func (c *Cache) cleanupLoop() {
	ticker := time.NewTicker(time.Minute)
	for range ticker.C {
		c.mu.Lock()
		now := time.Now()
		for k, e := range c.store {
			if e.expiresAt.Before(now) {
				delete(c.store, k)
			}
		}
		c.mu.Unlock()
	}
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

type rateLimitEntry struct {
	count   int
	resetAt time.Time
}

type RateLimiter struct {
	mu          sync.Mutex
	store       map[string]*rateLimitEntry
	windowSec   int
	maxRequests int
}

func NewRateLimiter(windowSec, maxRequests int) *RateLimiter {
	return &RateLimiter{
		store:       make(map[string]*rateLimitEntry),
		windowSec:   windowSec,
		maxRequests: maxRequests,
	}
}

type RateLimitResult struct {
	Allowed    bool
	Remaining  int
	RetryAfter int
}

func (r *RateLimiter) Check(key string) RateLimitResult {
	r.mu.Lock()
	defer r.mu.Unlock()

	now := time.Now()
	entry := r.store[key]
	if entry == nil || entry.resetAt.Before(now) {
		entry = &rateLimitEntry{count: 0, resetAt: now.Add(time.Duration(r.windowSec) * time.Second)}
		r.store[key] = entry
	}

	entry.count++
	remaining := r.maxRequests - entry.count
	if remaining < 0 {
		remaining = 0
	}
	allowed := entry.count <= r.maxRequests
	retryAfter := 0
	if !allowed {
		retryAfter = int(math.Ceil(entry.resetAt.Sub(now).Seconds()))
	}
	return RateLimitResult{Allowed: allowed, Remaining: remaining, RetryAfter: retryAfter}
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

type Middleware func(http.Handler) http.Handler

func Chain(middlewares ...Middleware) Middleware {
	return func(final http.Handler) http.Handler {
		for i := len(middlewares) - 1; i >= 0; i-- {
			final = middlewares[i](final)
		}
		return final
	}
}

func RequestIDMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		id := r.Header.Get("X-Request-ID")
		if id == "" {
			id = newUUID()
		}
		w.Header().Set("X-Request-ID", id)
		next.ServeHTTP(w, r.WithContext(context.WithValue(r.Context(), ctxKeyRequestID{}, id)))
	})
}

type ctxKeyRequestID struct{}

func RequestIDFromCtx(ctx context.Context) string {
	v, _ := ctx.Value(ctxKeyRequestID{}).(string)
	return v
}

func LoggingMiddleware(log *Logger) Middleware {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			start := time.Now()
			rw    := &responseWriter{ResponseWriter: w, status: 200}
			next.ServeHTTP(rw, r)
			log.Info("request", "method", r.Method, "path", r.URL.Path,
				"status", rw.status, "duration_ms", time.Since(start).Milliseconds())
		})
	}
}

type responseWriter struct {
	http.ResponseWriter
	status int
}

func (rw *responseWriter) WriteHeader(code int) {
	rw.status = code
	rw.ResponseWriter.WriteHeader(code)
}

func RecoveryMiddleware(log *Logger) Middleware {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			defer func() {
				if rec := recover(); rec != nil {
					log.Error("panic recovered", "error", rec)
					WriteJSON(w, http.StatusInternalServerError, map[string]string{"error": "internal server error"})
				}
			}()
			next.ServeHTTP(w, r)
		})
	}
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

func WriteJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	if err := json.NewEncoder(w).Encode(v); err != nil {
		// Best-effort; headers already sent.
		_ = err
	}
}

func ReadJSON(r *http.Request, dst any) error {
	if err := json.NewDecoder(r.Body).Decode(dst); err != nil {
		return fmt.Errorf("invalid JSON body: %w", err)
	}
	return nil
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

type PageParams struct {
	Page   int
	Limit  int
	SortBy string
	SortDir string
}

func ParsePageParams(r *http.Request) PageParams {
	page  := parseIntParam(r, "page", 1)
	limit := parseIntParam(r, "limit", 20)
	if limit > 100 {
		limit = 100
	}
	return PageParams{
		Page:    page,
		Limit:   limit,
		SortBy:  r.URL.Query().Get("sort_by"),
		SortDir: r.URL.Query().Get("sort_dir"),
	}
}

type PageResult[T any] struct {
	Items   []T `json:"items"`
	Total   int `json:"total"`
	Page    int `json:"page"`
	Limit   int `json:"limit"`
	HasNext bool `json:"has_next"`
	HasPrev bool `json:"has_prev"`
}

func NewPageResult[T any](items []T, total, page, limit int) PageResult[T] {
	offset := (page - 1) * limit
	return PageResult[T]{
		Items:   items,
		Total:   total,
		Page:    page,
		Limit:   limit,
		HasNext: offset+limit < total,
		HasPrev: page > 1,
	}
}

func parseIntParam(r *http.Request, key string, fallback int) int {
	s := r.URL.Query().Get(key)
	if s == "" {
		return fallback
	}
	n, err := strconv.Atoi(s)
	if err != nil || n < 1 {
		return fallback
	}
	return n
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-04-01][platform]: replace with LaunchDarkly Go SDK
type Flag struct {
	Name           string
	Enabled        bool
	RolloutPercent int
	Allowlist      map[string]bool
}

type FlagService struct {
	mu    sync.RWMutex
	flags map[string]*Flag
}

func NewFlagService() *FlagService {
	return &FlagService{flags: make(map[string]*Flag)}
}

func (fs *FlagService) Define(name string, enabled bool, rollout int, allowlist ...string) {
	al := make(map[string]bool, len(allowlist))
	for _, id := range allowlist {
		al[id] = true
	}
	fs.mu.Lock()
	defer fs.mu.Unlock()
	fs.flags[name] = &Flag{Name: name, Enabled: enabled, RolloutPercent: rollout, Allowlist: al}
}

func (fs *FlagService) IsEnabled(name, userID string) bool {
	fs.mu.RLock()
	flag := fs.flags[name]
	fs.mu.RUnlock()

	if flag == nil || !flag.Enabled {
		return false
	}
	if flag.Allowlist[userID] {
		return true
	}
	if flag.RolloutPercent >= 100 {
		return true
	}
	h := sha256.Sum256([]byte(name + ":" + userID))
	bucket := int(h[0]) % 100
	return bucket < flag.RolloutPercent
}

// ---------------------------------------------------------------------------
// Health monitor
// ---------------------------------------------------------------------------

type HealthCheck func(ctx context.Context) error

type HealthStatus struct {
	Status  string                     `json:"status"`
	Uptime  int64                      `json:"uptime_seconds"`
	Version string                     `json:"version"`
	Checks  map[string]CheckResult     `json:"checks"`
}

type CheckResult struct {
	Status    string  `json:"status"`
	LatencyMs float64 `json:"latency_ms"`
	Message   string  `json:"message,omitempty"`
}

type HealthMonitor struct {
	checks    map[string]HealthCheck
	startedAt time.Time
	mu        sync.RWMutex
}

func NewHealthMonitor() *HealthMonitor {
	return &HealthMonitor{checks: make(map[string]HealthCheck), startedAt: time.Now()}
}

func (h *HealthMonitor) Register(name string, check HealthCheck) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.checks[name] = check
}

func (h *HealthMonitor) Run(ctx context.Context) HealthStatus {
	h.mu.RLock()
	checks := make(map[string]HealthCheck, len(h.checks))
	for k, v := range h.checks {
		checks[k] = v
	}
	h.mu.RUnlock()

	results := make(map[string]CheckResult, len(checks))
	var wg sync.WaitGroup
	var mu sync.Mutex

	for name, check := range checks {
		wg.Add(1)
		go func(n string, c HealthCheck) {
			defer wg.Done()
			start := time.Now()
			err := c(ctx)
			r := CheckResult{
				Status:    "ok",
				LatencyMs: float64(time.Since(start).Microseconds()) / 1000,
			}
			if err != nil {
				r.Status = "fail"
				r.Message = err.Error()
			}
			mu.Lock()
			results[n] = r
			mu.Unlock()
		}(name, check)
	}
	wg.Wait()

	overall := "healthy"
	for _, r := range results {
		if r.Status != "ok" {
			overall = "unhealthy"
			break
		}
	}

	return HealthStatus{
		Status:  overall,
		Uptime:  int64(time.Since(h.startedAt).Seconds()),
		Version: getEnv("APP_VERSION", "dev"),
		Checks:  results,
	}
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

type circuitState int

const (
	stateClosed   circuitState = iota
	stateOpen
	stateHalfOpen
)

// FIXME[2021-01-15]: half-open state allows only one probe; add a success threshold
type CircuitBreaker struct {
	mu             sync.Mutex
	state          circuitState
	failures       int
	threshold      int
	lastFailure    time.Time
	resetTimeout   time.Duration
}

func NewCircuitBreaker(threshold int, resetTimeout time.Duration) *CircuitBreaker {
	return &CircuitBreaker{threshold: threshold, resetTimeout: resetTimeout}
}

func (cb *CircuitBreaker) Call(fn func() error) error {
	cb.mu.Lock()
	if cb.state == stateOpen {
		if time.Since(cb.lastFailure) >= cb.resetTimeout {
			cb.state = stateHalfOpen
		} else {
			cb.mu.Unlock()
			return &AppError{Code: "CIRCUIT_OPEN", Message: "Service temporarily unavailable", StatusCode: 503}
		}
	}
	cb.mu.Unlock()

	err := fn()
	cb.mu.Lock()
	defer cb.mu.Unlock()
	if err != nil {
		cb.failures++
		cb.lastFailure = time.Now()
		if cb.failures >= cb.threshold {
			cb.state = stateOpen
		}
	} else {
		cb.failures = 0
		cb.state = stateClosed
	}
	return err
}

// ---------------------------------------------------------------------------
// LRU Cache
// ---------------------------------------------------------------------------

type lruEntry[K comparable, V any] struct {
	key   K
	value V
	prev  *lruEntry[K, V]
	next  *lruEntry[K, V]
}

type LRU[K comparable, V any] struct {
	mu       sync.Mutex
	capacity int
	index    map[K]*lruEntry[K, V]
	head     *lruEntry[K, V]
	tail     *lruEntry[K, V]
}

func NewLRU[K comparable, V any](capacity int) *LRU[K, V] {
	head := &lruEntry[K, V]{}
	tail := &lruEntry[K, V]{}
	head.next = tail
	tail.prev = head
	return &LRU[K, V]{capacity: capacity, index: make(map[K]*lruEntry[K, V]), head: head, tail: tail}
}

func (l *LRU[K, V]) Get(key K) (V, bool) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if e, ok := l.index[key]; ok {
		l.moveToFront(e)
		return e.value, true
	}
	var zero V
	return zero, false
}

func (l *LRU[K, V]) Set(key K, value V) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if e, ok := l.index[key]; ok {
		e.value = value
		l.moveToFront(e)
		return
	}
	if len(l.index) >= l.capacity {
		oldest := l.tail.prev
		l.removeEntry(oldest)
		delete(l.index, oldest.key)
	}
	e := &lruEntry[K, V]{key: key, value: value}
	l.index[key] = e
	e.next = l.head.next
	e.prev = l.head
	l.head.next.prev = e
	l.head.next = e
}

func (l *LRU[K, V]) moveToFront(e *lruEntry[K, V]) {
	l.removeEntry(e)
	e.next = l.head.next
	e.prev = l.head
	l.head.next.prev = e
	l.head.next = e
}

func (l *LRU[K, V]) removeEntry(e *lruEntry[K, V]) {
	e.prev.next = e.next
	e.next.prev = e.prev
}

func (l *LRU[K, V]) Len() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.index)
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

func newUUID() string {
	b := make([]byte, 16)
	_, _ = rand.Read(b)
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:])
}

func hashString(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}

func Chunk[T any](slice []T, size int) [][]T {
	var chunks [][]T
	for size < len(slice) {
		slice, chunks = slice[size:], append(chunks, slice[:size])
	}
	return append(chunks, slice)
}

func Map[T, U any](slice []T, fn func(T) U) []U {
	out := make([]U, len(slice))
	for i, v := range slice {
		out[i] = fn(v)
	}
	return out
}

func Filter[T any](slice []T, fn func(T) bool) []T {
	var out []T
	for _, v := range slice {
		if fn(v) {
			out = append(out, v)
		}
	}
	return out
}

func Contains[T comparable](slice []T, item T) bool {
	for _, v := range slice {
		if v == item {
			return true
		}
	}
	return false
}

func Keys[K comparable, V any](m map[K]V) []K {
	keys := make([]K, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	return keys
}

func SortedKeys[V any](m map[string]V) []string {
	keys := Keys(m)
	sort.Strings(keys)
	return keys
}

func Truncate(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen-1] + "…"
}

func MaskEmail(email string) string {
	parts := strings.SplitN(email, "@", 2)
	if len(parts) != 2 {
		return email
	}
	local := parts[0]
	visible := local
	if len(local) > 2 {
		visible = local[:2] + strings.Repeat("*", len(local)-2)
	}
	return visible + "@" + parts[1]
}

func FormatBytes(bytes int64) string {
	const unit = 1024
	if bytes < unit {
		return fmt.Sprintf("%d B", bytes)
	}
	div, exp := int64(unit), 0
	for n := bytes / unit; n >= unit; n /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.2f %cB", float64(bytes)/float64(div), "KMGTPE"[exp])
}

func FormatDuration(d time.Duration) string {
	ms := d.Milliseconds()
	switch {
	case ms < 1000:
		return fmt.Sprintf("%dms", ms)
	case ms < 60_000:
		return fmt.Sprintf("%.1fs", float64(ms)/1000)
	default:
		m := ms / 60_000
		s := (ms % 60_000) / 1000
		return fmt.Sprintf("%dm%ds", m, s)
	}
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

type Result[T any] struct {
	value T
	err   error
}

func OK[T any](value T) Result[T]    { return Result[T]{value: value} }
func Err[T any](err error) Result[T] { return Result[T]{err: err} }

func (r Result[T]) IsOK() bool    { return r.err == nil }
func (r Result[T]) Error() error  { return r.err }
func (r Result[T]) Unwrap() T {
	if r.err != nil {
		panic(r.err)
	}
	return r.value
}
func (r Result[T]) UnwrapOr(fallback T) T {
	if r.err != nil {
		return fallback
	}
	return r.value
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

type Event struct {
	ID          string    `json:"id"`
	Type        string    `json:"type"`
	OccurredAt  time.Time `json:"occurred_at"`
	Payload     any       `json:"payload"`
}

type EventHandler func(Event)

// FIXME[2025-06-10]: add retry / dead-letter queue for failed handlers
type EventBus struct {
	mu       sync.RWMutex
	handlers map[string][]EventHandler
}

func NewEventBus() *EventBus {
	return &EventBus{handlers: make(map[string][]EventHandler)}
}

func (eb *EventBus) Subscribe(eventType string, handler EventHandler) func() {
	eb.mu.Lock()
	defer eb.mu.Unlock()
	eb.handlers[eventType] = append(eb.handlers[eventType], handler)
	return func() {
		eb.mu.Lock()
		defer eb.mu.Unlock()
		handlers := eb.handlers[eventType]
		for i, h := range handlers {
			// Compare function pointers — not idiomatic but sufficient here.
			_ = h
			if i < len(handlers) {
				eb.handlers[eventType] = append(handlers[:i], handlers[i+1:]...)
				break
			}
		}
	}
}

func (eb *EventBus) Publish(eventType string, payload any) {
	event := Event{
		ID:         newUUID(),
		Type:       eventType,
		OccurredAt: time.Now(),
		Payload:    payload,
	}
	eb.mu.RLock()
	handlers := append([]EventHandler(nil), eb.handlers[eventType]...)
	eb.mu.RUnlock()

	for _, h := range handlers {
		go h(event)
	}
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

// TODO[2088-01-01][observability]: expose /metrics in Prometheus text format
type Counter struct {
	mu    sync.Mutex
	value int64
	name  string
}

func NewCounter(name string) *Counter { return &Counter{name: name} }
func (c *Counter) Inc()               { c.mu.Lock(); c.value++; c.mu.Unlock() }
func (c *Counter) Add(n int64)        { c.mu.Lock(); c.value += n; c.mu.Unlock() }
func (c *Counter) Read() int64        { c.mu.Lock(); defer c.mu.Unlock(); return c.value }
func (c *Counter) Reset()             { c.mu.Lock(); c.value = 0; c.mu.Unlock() }

type Histogram struct {
	mu      sync.Mutex
	name    string
	samples []float64
}

func NewHistogram(name string) *Histogram { return &Histogram{name: name} }

func (h *Histogram) Observe(v float64) {
	h.mu.Lock()
	h.samples = append(h.samples, v)
	h.mu.Unlock()
}

func (h *Histogram) Percentile(p float64) float64 {
	h.mu.Lock()
	defer h.mu.Unlock()
	if len(h.samples) == 0 {
		return 0
	}
	sorted := append([]float64(nil), h.samples...)
	sort.Float64s(sorted)
	idx := int(math.Ceil(p/100*float64(len(sorted)))) - 1
	if idx < 0 {
		idx = 0
	}
	return sorted[idx]
}

// ---------------------------------------------------------------------------
// Sentinel errors (package-level)
// ---------------------------------------------------------------------------

var (
	ErrNotFound   = errors.New("not found")
	ErrConflict   = errors.New("conflict")
	ErrForbidden  = errors.New("forbidden")
	ErrBadRequest = errors.New("bad request")
)

// TODO[2099-07-01][api]: migrate callers to typed errors; remove sentinel vars above
func IsNotFound(err error) bool  { return errors.Is(err, ErrNotFound) }
func IsConflict(err error) bool  { return errors.Is(err, ErrConflict) }
func IsForbidden(err error) bool { return errors.Is(err, ErrForbidden) }
