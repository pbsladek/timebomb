# sample.rb — fixture file for timebomb scanner tests.
#
# Annotation inventory (hardcoded dates, never relative to today):
#   Expired        (2018–2021): 6
#   Expiring-soon  (2025-06):   2
#   Future / OK    (2088/2099): 4

require "json"
require "logger"
require "digest"
require "securerandom"
require "time"
require "uri"
require "net/http"

# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

module Errors
  class AppError < StandardError
    attr_reader :code, :status, :details

    def initialize(code, message, status: 500, details: nil)
      super(message)
      @code    = code
      @status  = status
      @details = details
    end
  end

  class ValidationError < AppError
    def initialize(message, details = nil)
      super("VALIDATION_ERROR", message, status: 422, details: details)
    end
  end

  class AuthError < AppError
    def initialize(message = "Unauthorized")
      super("AUTH_ERROR", message, status: 401)
    end
  end

  class ForbiddenError < AppError
    def initialize(message = "Forbidden")
      super("FORBIDDEN", message, status: 403)
    end
  end

  class NotFoundError < AppError
    def initialize(resource)
      super("NOT_FOUND", "#{resource} not found", status: 404)
    end
  end

  class ConflictError < AppError
    def initialize(message)
      super("CONFLICT", message, status: 409)
    end
  end

  class RateLimitError < AppError
    attr_reader :retry_after

    def initialize(retry_after)
      super("RATE_LIMITED", "Too many requests", status: 429)
      @retry_after = retry_after
    end
  end
end

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

module Validation
  ValidationFailure = Struct.new(:field, :message, :value)

  def self.present?(field, value)
    return ValidationFailure.new(field, "is required") if value.nil? || value.to_s.strip.empty?
    nil
  end

  def self.min_length(field, value, min)
    return nil unless value.is_a?(String)
    return ValidationFailure.new(field, "must be at least #{min} characters", value) if value.length < min
    nil
  end

  def self.max_length(field, value, max)
    return nil unless value.is_a?(String)
    return ValidationFailure.new(field, "must be at most #{max} characters", value) if value.length > max
    nil
  end

  # TODO[2020-04-01]: replace with a proper RFC 5322 email validator
  EMAIL_RE = /\A[^\s@]+@[^\s@]+\.[^\s@]+\z/

  def self.email(field, value)
    return nil unless value.is_a?(String)
    return ValidationFailure.new(field, "must be a valid email address", value) unless value.match?(EMAIL_RE)
    nil
  end

  UUID_RE = /\A[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/i

  def self.uuid(field, value)
    return nil unless value.is_a?(String)
    return ValidationFailure.new(field, "must be a valid UUID v4", value) unless value.match?(UUID_RE)
    nil
  end

  def self.run(checks)
    checks.compact
  end
end

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

module Config
  DEFAULT = {
    host:              ENV.fetch("HOST", "0.0.0.0"),
    port:              ENV.fetch("PORT", "3000").to_i,
    env:               ENV.fetch("RACK_ENV", "development"),
    db_host:           ENV.fetch("DB_HOST", "localhost"),
    db_port:           ENV.fetch("DB_PORT", "5432").to_i,
    db_name:           ENV.fetch("DB_NAME", "app"),
    db_user:           ENV.fetch("DB_USER", "postgres"),
    db_password:       ENV.fetch("DB_PASSWORD", ""),
    db_pool_min:       ENV.fetch("DB_POOL_MIN", "2").to_i,
    db_pool_max:       ENV.fetch("DB_POOL_MAX", "10").to_i,
    redis_host:        ENV.fetch("REDIS_HOST", "localhost"),
    redis_port:        ENV.fetch("REDIS_PORT", "6379").to_i,
    cache_ttl:         ENV.fetch("CACHE_TTL", "300").to_i,
    jwt_secret:        ENV.fetch("JWT_SECRET", "change-me"),
    jwt_expiry:        ENV.fetch("JWT_EXPIRY", "3600").to_i,
    bcrypt_cost:       ENV.fetch("BCRYPT_COST", "12").to_i,
    rate_window_sec:   ENV.fetch("RATE_WINDOW_SEC", "60").to_i,
    rate_max_requests: ENV.fetch("RATE_MAX", "100").to_i,
    log_level:         ENV.fetch("LOG_LEVEL", "info"),
  }.freeze

  def self.get(key) = DEFAULT.fetch(key)
  def self.production? = DEFAULT[:env] == "production"
  def self.development? = DEFAULT[:env] == "development"
end

# ---------------------------------------------------------------------------
# Logger
# ---------------------------------------------------------------------------

module AppLogger
  LEVELS = %w[debug info warn error].freeze

  def self.build(out: $stdout)
    logger = ::Logger.new(out)
    logger.formatter = proc do |severity, time, _prog, msg|
      entry = { level: severity.downcase, time: time.iso8601, message: msg }
      "#{JSON.generate(entry)}\n"
    end
    logger.level = ::Logger.const_get(Config.get(:log_level).upcase)
    logger
  end

  LOG = build

  def self.debug(msg, **meta) = LOG.debug(annotate(msg, meta))
  def self.info(msg, **meta)  = LOG.info(annotate(msg, meta))
  def self.warn(msg, **meta)  = LOG.warn(annotate(msg, meta))
  def self.error(msg, **meta) = LOG.error(annotate(msg, meta))

  def self.annotate(msg, meta)
    meta.empty? ? msg : "#{msg} #{meta.map { |k, v| "#{k}=#{v.inspect}" }.join(" ")}"
  end
end

# ---------------------------------------------------------------------------
# In-process cache
# ---------------------------------------------------------------------------

# HACK[2018-09-15]: in-memory store used as Redis stand-in; replace before launch
class SimpleCache
  Entry = Struct.new(:value, :expires_at)

  def initialize
    @store = {}
    @mutex = Mutex.new
  end

  def get(key)
    @mutex.synchronize do
      entry = @store[key]
      return nil if entry.nil?
      return nil if entry.expires_at <= Time.now.to_f
      entry.value
    end
  end

  def set(key, value, ttl:)
    @mutex.synchronize do
      @store[key] = Entry.new(value, Time.now.to_f + ttl)
    end
    value
  end

  def delete(key)
    @mutex.synchronize { @store.delete(key) }
  end

  def get_or_set(key, ttl:, &block)
    cached = get(key)
    return cached unless cached.nil?
    value = block.call
    set(key, value, ttl: ttl)
    value
  end

  def flush_pattern(pattern)
    re = Regexp.new("\\A" + Regexp.escape(pattern).gsub("\\*", ".*") + "\\z")
    @mutex.synchronize { @store.delete_if { |k, _| k.match?(re) } }
  end

  def cleanup
    now = Time.now.to_f
    @mutex.synchronize { @store.delete_if { |_, e| e.expires_at <= now } }
  end
end

CACHE = SimpleCache.new

# ---------------------------------------------------------------------------
# Rate limiter
# ---------------------------------------------------------------------------

class RateLimiter
  Entry = Struct.new(:count, :reset_at)

  def initialize(window_sec:, max_requests:)
    @window_sec   = window_sec
    @max_requests = max_requests
    @store        = {}
    @mutex        = Mutex.new
  end

  def check(key)
    now = Time.now.to_f
    @mutex.synchronize do
      entry = @store[key]
      if entry.nil? || entry.reset_at <= now
        entry = Entry.new(0, now + @window_sec)
        @store[key] = entry
      end
      entry.count += 1
      remaining  = [@max_requests - entry.count, 0].max
      allowed    = entry.count <= @max_requests
      retry_after = allowed ? 0 : (entry.reset_at - now).ceil
      { allowed: allowed, remaining: remaining, retry_after: retry_after }
    end
  end

  def cleanup
    now = Time.now.to_f
    @mutex.synchronize { @store.delete_if { |_, e| e.reset_at <= now } }
  end
end

# ---------------------------------------------------------------------------
# HTTP client
# ---------------------------------------------------------------------------

class HttpClient
  DEFAULT_TIMEOUT = 10

  def initialize(base_url:, timeout: DEFAULT_TIMEOUT, headers: {})
    @base_uri = URI.parse(base_url)
    @timeout  = timeout
    @headers  = { "Content-Type" => "application/json", "Accept" => "application/json" }.merge(headers)
  end

  def get(path, params: {})
    uri = build_uri(path, params)
    req = Net::HTTP::Get.new(uri)
    execute(req, uri)
  end

  def post(path, body:)
    uri = build_uri(path)
    req = Net::HTTP::Post.new(uri)
    req.body = JSON.generate(body)
    execute(req, uri)
  end

  def put(path, body:)
    uri = build_uri(path)
    req = Net::HTTP::Put.new(uri)
    req.body = JSON.generate(body)
    execute(req, uri)
  end

  def delete(path)
    uri = build_uri(path)
    req = Net::HTTP::Delete.new(uri)
    execute(req, uri)
  end

  private

  def build_uri(path, params = {})
    uri = URI.join(@base_uri.to_s, path)
    uri.query = URI.encode_www_form(params) unless params.empty?
    uri
  end

  def execute(req, uri)
    @headers.each { |k, v| req[k] = v }
    Net::HTTP.start(uri.host, uri.port, use_ssl: uri.scheme == "https",
                    read_timeout: @timeout, open_timeout: @timeout) do |http|
      response = http.request(req)
      parse_response(response)
    end
  end

  def parse_response(response)
    body = response.body.to_s
    parsed = body.empty? ? nil : JSON.parse(body, symbolize_names: true)
    { status: response.code.to_i, body: parsed, headers: response.to_hash }
  rescue JSON::ParserError
    { status: response.code.to_i, body: body, headers: response.to_hash }
  end
end

# ---------------------------------------------------------------------------
# Repository base
# ---------------------------------------------------------------------------

# TODO[2019-11-01]: replace Struct-based rows with proper ORM (Sequel or ActiveRecord)
class BaseRepository
  def initialize(db:, table:, cache: CACHE, cache_ttl: 300)
    @db        = db
    @table     = table
    @cache     = cache
    @cache_ttl = cache_ttl
  end

  def find_by_id(id)
    @cache.get_or_set("#{@table}:#{id}", ttl: @cache_ttl) do
      query_one("SELECT * FROM #{@table} WHERE id = $1 AND deleted_at IS NULL", id)
    end
  end

  def find_all(page: 1, limit: 20, order: "created_at DESC")
    offset = (page - 1) * limit
    rows   = query_many("SELECT * FROM #{@table} WHERE deleted_at IS NULL ORDER BY #{order} LIMIT $1 OFFSET $2", limit, offset)
    count  = query_one("SELECT COUNT(*) AS total FROM #{@table} WHERE deleted_at IS NULL")
    total  = count[:total].to_i
    {
      items:    rows,
      total:    total,
      page:     page,
      limit:    limit,
      has_next: offset + limit < total,
      has_prev: page > 1,
    }
  end

  def delete(id)
    exec_query("UPDATE #{@table} SET deleted_at = NOW() WHERE id = $1", id)
    @cache.delete("#{@table}:#{id}")
  end

  private

  def query_one(sql, *params)
    result = @db.exec_params(sql, params)
    result.first&.transform_keys(&:to_sym)
  end

  def query_many(sql, *params)
    result = @db.exec_params(sql, params)
    result.map { |row| row.transform_keys(&:to_sym) }
  end

  def exec_query(sql, *params)
    @db.exec_params(sql, params)
  end
end

# ---------------------------------------------------------------------------
# Domain: Users
# ---------------------------------------------------------------------------

class UserRepository < BaseRepository
  def initialize(db:, cache: CACHE)
    super(db: db, table: "users", cache: cache, cache_ttl: 60)
  end

  def find_by_email(email)
    query_one("SELECT * FROM users WHERE email = $1 AND deleted_at IS NULL", email.downcase)
  end

  def update_last_login(id)
    exec_query("UPDATE users SET last_login_at = NOW(), updated_at = NOW() WHERE id = $1", id)
    @cache.delete("users:#{id}")
  end

  def count_by_role(role)
    row = query_one("SELECT COUNT(*) AS count FROM users WHERE role = $1 AND deleted_at IS NULL", role)
    row[:count].to_i
  end

  def create(email:, password_hash:, display_name:, role: "user")
    id = SecureRandom.uuid
    now = Time.now.iso8601
    exec_query(
      "INSERT INTO users (id, email, password_hash, display_name, role, email_verified, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
      id, email.downcase, password_hash, display_name, role, false, now, now
    )
    find_by_id(id)
  end
end

# ---------------------------------------------------------------------------
# Domain: Sessions
# ---------------------------------------------------------------------------

class SessionRepository < BaseRepository
  def initialize(db:, cache: CACHE)
    super(db: db, table: "sessions", cache: cache, cache_ttl: 30)
  end

  def find_by_token(token)
    query_one("SELECT * FROM sessions WHERE token = $1 AND expires_at > NOW()", token)
  end

  def find_by_refresh_token(refresh_token)
    query_one("SELECT * FROM sessions WHERE refresh_token = $1 AND expires_at > NOW()", refresh_token)
  end

  # FIXME[2025-06-08]: move to a background job via Sidekiq
  def delete_expired
    result = exec_query("DELETE FROM sessions WHERE expires_at <= NOW()")
    result.cmd_tuples
  end

  def delete_all_for_user(user_id)
    exec_query("DELETE FROM sessions WHERE user_id = $1", user_id)
    @cache.flush_pattern("sessions:*")
  end

  def create(user_id:, user_agent: nil, ip_address: nil, expiry_sec: 3600)
    id            = SecureRandom.uuid
    token         = SecureRandom.urlsafe_base64(48)
    refresh_token = SecureRandom.urlsafe_base64(48)
    expires_at    = (Time.now + expiry_sec).iso8601
    exec_query(
      "INSERT INTO sessions (id, user_id, token, refresh_token, user_agent, ip_address, expires_at, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
      id, user_id, token, refresh_token, user_agent, ip_address, expires_at, Time.now.iso8601
    )
    { id: id, token: token, refresh_token: refresh_token, expires_at: expires_at }
  end
end

# ---------------------------------------------------------------------------
# Auth service
# ---------------------------------------------------------------------------

class AuthService
  def initialize(users:, sessions:, jwt_secret:, bcrypt_cost: 12)
    @users       = users
    @sessions    = sessions
    @jwt_secret  = jwt_secret
    @bcrypt_cost = bcrypt_cost
  end

  def register(email:, password:, display_name:, role: "user")
    failures = Validation.run([
      Validation.present?("email", email),
      Validation.email("email", email),
      Validation.present?("password", password),
      Validation.min_length("password", password.to_s, 8),
      Validation.max_length("password", password.to_s, 128),
      Validation.present?("display_name", display_name),
      Validation.min_length("display_name", display_name.to_s, 2),
    ])

    raise Errors::ValidationError.new("Invalid input", failures) if failures.any?
    raise Errors::ConflictError.new("Email already registered") if @users.find_by_email(email)

    hash = hash_password(password)
    user = @users.create(email: email, password_hash: hash, display_name: display_name, role: role)
    AppLogger.info("user registered", user_id: user[:id], email: user[:email])
    user
  end

  def login(email:, password:, user_agent: nil, ip_address: nil)
    user = @users.find_by_email(email)
    unless user && verify_password(password, user[:password_hash])
      raise Errors::AuthError.new("Invalid email or password")
    end
    raise Errors::AuthError.new("Account is deactivated") if user[:deleted_at]

    session = @sessions.create(user_id: user[:id], user_agent: user_agent, ip_address: ip_address)
    @users.update_last_login(user[:id])
    AppLogger.info("user logged in", user_id: user[:id])
    session
  end

  def logout(token)
    session = @sessions.find_by_token(token)
    return unless session

    @sessions.delete(session[:id])
    AppLogger.info("user logged out", user_id: session[:user_id])
  end

  def verify(token)
    session = @sessions.find_by_token(token)
    raise Errors::AuthError unless session

    user = @users.find_by_id(session[:user_id])
    raise Errors::AuthError.new("Account not found") if user.nil? || user[:deleted_at]

    { id: user[:id], email: user[:email], role: user[:role], session_id: session[:id] }
  end

  private

  def hash_password(password)
    # Stub: real impl uses bcrypt gem
    Digest::SHA256.hexdigest("#{password}:#{@bcrypt_cost}")
  end

  def verify_password(password, hash)
    hash_password(password) == hash
  end
end

# ---------------------------------------------------------------------------
# Feature flags
# ---------------------------------------------------------------------------

# TODO[2099-01-01][platform]: wire to LaunchDarkly or Flipper
class FeatureFlagService
  Flag = Struct.new(:name, :enabled, :rollout_percent, :allowlist, keyword_init: true)

  def initialize
    @flags = {}
  end

  def define(name:, enabled: true, rollout_percent: 100, allowlist: [])
    @flags[name.to_s] = Flag.new(
      name: name.to_s, enabled: enabled,
      rollout_percent: rollout_percent, allowlist: allowlist.map(&:to_s)
    )
    self
  end

  def enabled?(name, user_id: nil)
    flag = @flags[name.to_s]
    return false if flag.nil? || !flag.enabled
    return true if user_id && flag.allowlist.include?(user_id.to_s)
    return true if flag.rollout_percent >= 100
    return false if flag.rollout_percent <= 0

    bucket = Digest::MD5.hexdigest("#{name}:#{user_id || "anon"}").to_i(16) % 100
    bucket < flag.rollout_percent
  end
end

# ---------------------------------------------------------------------------
# Job queue (in-process stub)
# ---------------------------------------------------------------------------

# TEMP[2021-03-10]: replace with Sidekiq before scaling beyond one process
class InProcessQueue
  Job = Struct.new(:id, :type, :payload, :attempts, :max_attempts, :run_at, keyword_init: true)

  def initialize
    @handlers = {}
    @queue    = []
    @mutex    = Mutex.new
    @running  = false
  end

  def register(type, &handler)
    @handlers[type.to_s] = handler
  end

  def enqueue(type, payload, delay_sec: 0)
    job = Job.new(
      id: SecureRandom.uuid, type: type.to_s, payload: payload,
      attempts: 0, max_attempts: 3, run_at: Time.now + delay_sec
    )
    @mutex.synchronize { @queue << job }
    job
  end

  def start(interval_sec: 0.1)
    return if @running
    @running = true
    @thread  = Thread.new { loop { tick; sleep interval_sec } }
  end

  def stop
    @running = false
    @thread&.kill
  end

  private

  def tick
    now = Time.now
    ready = nil
    @mutex.synchronize do
      ready = @queue.select { |j| j.run_at <= now }
      @queue -= ready
    end

    ready.each do |job|
      handler = @handlers[job.type]
      next unless handler

      job.attempts += 1
      begin
        handler.call(job)
      rescue => e
        AppLogger.error("job failed", job_id: job.id, type: job.type, error: e.message)
        if job.attempts < job.max_attempts
          job.run_at = Time.now + (2**job.attempts)
          @mutex.synchronize { @queue << job }
        end
      end
    end
  end
end

# ---------------------------------------------------------------------------
# Health monitor
# ---------------------------------------------------------------------------

class HealthMonitor
  Check = Struct.new(:name, :proc)

  def initialize
    @checks     = []
    @started_at = Time.now
  end

  def register(name, &block)
    @checks << Check.new(name.to_s, block)
    self
  end

  def run
    results = {}
    @checks.each do |check|
      t0 = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      begin
        ok = check.proc.call
        results[check.name] = { status: ok ? "ok" : "fail", latency_ms: elapsed_ms(t0) }
      rescue => e
        results[check.name] = { status: "fail", message: e.message, latency_ms: elapsed_ms(t0) }
      end
    end

    overall = results.values.all? { |r| r[:status] == "ok" } ? "healthy" : "unhealthy"
    {
      status:  overall,
      uptime:  (Time.now - @started_at).to_i,
      checks:  results,
      version: ENV.fetch("APP_VERSION", "dev"),
    }
  end

  private

  def elapsed_ms(t0)
    ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - t0) * 1000).round(2)
  end
end

# ---------------------------------------------------------------------------
# Utilities
# ---------------------------------------------------------------------------

module Utils
  # FIXME[2020-08-01]: memoize is not thread-safe; use a Mutex or concurrent-ruby
  def self.memoize(cache = {}, &block)
    ->(key) { cache[key] ||= block.call(key) }
  end

  def self.deep_merge(base, override)
    base.merge(override) do |_key, old_val, new_val|
      old_val.is_a?(Hash) && new_val.is_a?(Hash) ? deep_merge(old_val, new_val) : new_val
    end
  end

  def self.retry_with_backoff(attempts: 3, base_delay: 1.0, &block)
    last_error = nil
    attempts.times do |i|
      return block.call
    rescue => e
      last_error = e
      sleep(base_delay * (2**i)) if i < attempts - 1
    end
    raise last_error
  end

  def self.chunk(arr, size)
    arr.each_slice(size).to_a
  end

  def self.group_by_key(arr, &key_fn)
    arr.group_by(&key_fn)
  end

  def self.deep_symbolize(obj)
    case obj
    when Hash  then obj.transform_keys(&:to_sym).transform_values { |v| deep_symbolize(v) }
    when Array then obj.map { |v| deep_symbolize(v) }
    else obj
    end
  end

  def self.blank?(val)
    val.nil? || (val.respond_to?(:empty?) && val.empty?)
  end

  def self.present?(val) = !blank?(val)

  def self.truncate(str, max_len, suffix: "…")
    return str if str.length <= max_len
    str[0, max_len - suffix.length] + suffix
  end

  def self.slugify(text)
    text.downcase.gsub(/[^\w\s-]/, "").gsub(/[\s_-]+/, "-").gsub(/\A-+|-+\z/, "")
  end

  def self.mask_email(email)
    local, domain = email.split("@", 2)
    return email unless local && domain
    visible = local.length > 2 ? local[0, 2] : local[0, 1]
    "#{visible}#{"*" * [local.length - 2, 1].max}@#{domain}"
  end

  def self.format_bytes(bytes)
    units = %w[B KB MB GB TB]
    i = (Math.log(bytes) / Math.log(1024)).floor rescue 0
    i = [i, units.length - 1].min
    "#{"%.2f" % (bytes.to_f / 1024**i)} #{units[i]}"
  end

  def self.format_duration_ms(ms)
    return "#{ms}ms"     if ms < 1_000
    return "#{"%.1f" % (ms / 1_000.0)}s" if ms < 60_000
    m = (ms / 60_000).to_i
    s = ((ms % 60_000) / 1_000).to_i
    "#{m}m #{s}s"
  end
end

# ---------------------------------------------------------------------------
# LRU Cache
# ---------------------------------------------------------------------------

class LRUCache
  def initialize(capacity)
    @capacity = capacity
    @store    = {}
  end

  def get(key)
    return unless @store.key?(key)
    value = @store.delete(key)
    @store[key] = value
    value
  end

  def set(key, value)
    @store.delete(key) if @store.key?(key)
    @store.shift if @store.size >= @capacity
    @store[key] = value
  end

  def delete(key) = @store.delete(key)
  def include?(key) = @store.key?(key)
  def size = @store.size
  def clear = @store.clear
end

# ---------------------------------------------------------------------------
# Observable
# ---------------------------------------------------------------------------

class Observable
  def initialize(initial)
    @value     = initial
    @observers = []
  end

  def value = @value

  def set(new_value)
    return if new_value == @value
    prev   = @value
    @value = new_value
    @observers.each { |obs| obs.call(new_value, prev) }
  end

  def update(&block)
    set(block.call(@value))
  end

  def subscribe(&observer)
    @observers << observer
    -> { @observers.delete(observer) }
  end

  def map(&transform)
    derived = Observable.new(transform.call(@value))
    subscribe { |v| derived.set(transform.call(v)) }
    derived
  end
end

# ---------------------------------------------------------------------------
# Circuit breaker
# ---------------------------------------------------------------------------

# TODO[2088-06-15][platform]: emit open/close events to metrics pipeline
class CircuitBreaker
  STATES = %i[closed open half_open].freeze

  def initialize(threshold:, reset_timeout_sec:)
    @threshold         = threshold
    @reset_timeout_sec = reset_timeout_sec
    @state             = :closed
    @failures          = 0
    @last_failure_at   = nil
    @mutex             = Mutex.new
  end

  def call(&block)
    @mutex.synchronize do
      if @state == :open
        elapsed = Time.now.to_f - @last_failure_at.to_f
        if elapsed >= @reset_timeout_sec
          @state = :half_open
        else
          raise Errors::AppError.new("CIRCUIT_OPEN", "Service temporarily unavailable", status: 503)
        end
      end
    end

    begin
      result = block.call
      on_success
      result
    rescue => e
      on_failure
      raise
    end
  end

  def state
    @mutex.synchronize { @state }
  end

  def reset
    @mutex.synchronize { @state = :closed; @failures = 0 }
  end

  private

  def on_success
    @mutex.synchronize { @failures = 0; @state = :closed }
  end

  def on_failure
    @mutex.synchronize do
      @failures       += 1
      @last_failure_at = Time.now
      @state           = :open if @failures >= @threshold
    end
  end
end

# ---------------------------------------------------------------------------
# Result type
# ---------------------------------------------------------------------------

module Result
  Success = Struct.new(:value) do
    def ok?    = true
    def error? = false
    def map(&block) = Success.new(block.call(value))
    def flat_map(&block) = block.call(value)
    def unwrap = value
    def unwrap_or(_fallback) = value
  end

  Failure = Struct.new(:error) do
    def ok?    = false
    def error? = true
    def map(&_block) = self
    def flat_map(&_block) = self
    def unwrap = raise error
    def unwrap_or(fallback) = fallback
  end

  def self.ok(value)    = Success.new(value)
  def self.err(error)   = Failure.new(error)

  def self.try(&block)
    ok(block.call)
  rescue => e
    err(e)
  end
end

# REMOVEME[2021-06-30]: legacy alias kept for compatibility with v1 internal API
module Outcome
  Success = Result::Success
  Failure = Result::Failure
end

# ---------------------------------------------------------------------------
# Metrics
# ---------------------------------------------------------------------------

class Counter
  def initialize(name, labels = {})
    @name   = name
    @labels = labels
    @value  = 0
    @mutex  = Mutex.new
  end

  def inc(by = 1) = @mutex.synchronize { @value += by }
  def reset       = @mutex.synchronize { @value = 0 }
  def read        = @mutex.synchronize { @value }
end

class Histogram
  # TODO[2099-09-01][observability]: push to Prometheus exporter
  DEFAULT_BUCKETS = [5, 10, 25, 50, 100, 250, 500, 1000].freeze

  def initialize(name, buckets: DEFAULT_BUCKETS)
    @name    = name
    @buckets = buckets
    @samples = []
    @mutex   = Mutex.new
  end

  def observe(value) = @mutex.synchronize { @samples << value }
  def reset          = @mutex.synchronize { @samples.clear }

  def percentile(p)
    @mutex.synchronize do
      return 0 if @samples.empty?
      sorted = @samples.sort
      idx    = [(p / 100.0 * sorted.length).ceil - 1, 0].max
      sorted[idx]
    end
  end

  def p50 = percentile(50)
  def p95 = percentile(95)
  def p99 = percentile(99)
end

class MetricsRegistry
  def initialize
    @counters   = {}
    @histograms = {}
    @mutex      = Mutex.new
  end

  def counter(name, labels = {})
    key = "#{name}:#{labels.sort.to_h}"
    @mutex.synchronize { @counters[key] ||= Counter.new(name, labels) }
  end

  def histogram(name, buckets: Histogram::DEFAULT_BUCKETS)
    @mutex.synchronize { @histograms[name] ||= Histogram.new(name, buckets: buckets) }
  end

  def snapshot
    @mutex.synchronize do
      counters   = @counters.transform_values(&:read)
      histograms = @histograms.transform_values { |h| { p50: h.p50, p95: h.p95, p99: h.p99 } }
      { counters: counters, histograms: histograms }
    end
  end
end

METRICS = MetricsRegistry.new
