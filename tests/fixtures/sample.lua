-- sample.lua — fixture file for timebomb scanner tests.
--
-- Annotation inventory (hardcoded dates, never relative to today):
--   Expired        (2018–2021): 4
--   Expiring-soon  (2025-06):   1
--   Future / OK    (2088/2099): 2

local M = {}

-- ---------------------------------------------------------------------------
-- Config
-- ---------------------------------------------------------------------------

local Config = {}
Config.__index = Config

function Config.from_env()
  return setmetatable({
    host       = os.getenv("HOST")       or "0.0.0.0",
    port       = tonumber(os.getenv("PORT") or "3000"),
    db_url     = os.getenv("DB_URL")     or "postgres://localhost/app",
    jwt_secret = os.getenv("JWT_SECRET") or "change-me",
    env        = os.getenv("APP_ENV")    or "development",
  }, Config)
end

function Config:is_production()
  return self.env == "production"
end

M.Config = Config

-- ---------------------------------------------------------------------------
-- Result helpers
-- ---------------------------------------------------------------------------

local function ok(value)   return { ok = true,  value = value } end
local function err(reason) return { ok = false, error = reason } end

local function unwrap(r)
  if r.ok then return r.value end
  error("unwrap on Err: " .. tostring(r.error))
end

local function unwrap_or(r, default)
  return r.ok and r.value or default
end

local function map_result(r, f)
  return r.ok and ok(f(r.value)) or r
end

M.ok, M.err, M.unwrap, M.unwrap_or, M.map_result =
  ok, err, unwrap, unwrap_or, map_result

-- ---------------------------------------------------------------------------
-- Validation
-- ---------------------------------------------------------------------------

-- TODO[2020-01-20]: replace hand-rolled validators with a JSON schema library
local function validate_required(field, value)
  if value == nil or value == "" then
    return { field = field, message = "is required" }
  end
end

local function validate_email(field, value)
  if not value or not value:match("^[^%s@]+@[^%s@]+%.[^%s@]+$") then
    return { field = field, message = "must be a valid email address", value = value }
  end
end

local function validate_min_length(field, value, min)
  if #value < min then
    return { field = field, message = "must be at least " .. min .. " characters", value = value }
  end
end

local function collect_failures(checks)
  local errs = {}
  for _, c in ipairs(checks) do
    if c ~= nil then errs[#errs + 1] = c end
  end
  return errs
end

M.validate_required  = validate_required
M.validate_email     = validate_email
M.validate_min_length = validate_min_length
M.collect_failures   = collect_failures

-- ---------------------------------------------------------------------------
-- Cache
-- ---------------------------------------------------------------------------

-- HACK[2019-04-10]: plain table cache; swap for a proper TTL eviction strategy before launch
local Cache = {}
Cache.__index = Cache

function Cache.new()
  return setmetatable({ _store = {} }, Cache)
end

function Cache:get(key)
  local e = self._store[key]
  if e and os.time() < e.expires_at then
    return e.value
  end
  self._store[key] = nil
  return nil
end

function Cache:set(key, value, ttl_sec)
  self._store[key] = { value = value, expires_at = os.time() + ttl_sec }
end

function Cache:del(key)
  self._store[key] = nil
end

function Cache:get_or_set(key, ttl_sec, fn)
  local v = self:get(key)
  if v ~= nil then return v end
  v = fn()
  self:set(key, v, ttl_sec)
  return v
end

M.Cache = Cache

-- ---------------------------------------------------------------------------
-- Rate limiter
-- ---------------------------------------------------------------------------

-- FIXME[2021-08-15]: single-process only; add a distributed counter before multi-node deploy
local RateLimiter = {}
RateLimiter.__index = RateLimiter

function RateLimiter.new(window_sec, max_requests)
  return setmetatable({
    window_sec   = window_sec,
    max_requests = max_requests,
    _store       = {},
  }, RateLimiter)
end

function RateLimiter:check(key)
  local now   = os.time()
  local entry = self._store[key]
  if not entry or entry.reset_at <= now then
    entry = { count = 0, reset_at = now + self.window_sec }
    self._store[key] = entry
  end
  entry.count = entry.count + 1
  local allowed = entry.count <= self.max_requests
  return {
    allowed     = allowed,
    remaining   = allowed and (self.max_requests - entry.count) or 0,
    retry_after = allowed and 0 or (entry.reset_at - now),
  }
end

M.RateLimiter = RateLimiter

-- ---------------------------------------------------------------------------
-- Pagination
-- ---------------------------------------------------------------------------

local function paginate(items, page_num, page_size)
  local offset = math.max(0, (page_num - 1) * page_size)
  local total  = #items
  local chunk  = {}
  for i = offset + 1, math.min(offset + page_size, total) do
    chunk[#chunk + 1] = items[i]
  end
  return {
    items     = chunk,
    total     = total,
    page_num  = page_num,
    page_size = page_size,
    has_next  = offset + #chunk < total,
    has_prev  = page_num > 1,
  }
end

M.paginate = paginate

-- ---------------------------------------------------------------------------
-- Feature flags
-- ---------------------------------------------------------------------------

-- TODO[2099-02-01][platform]: replace table-based flags with a remote LaunchDarkly client
local FlagService = {}
FlagService.__index = FlagService

function FlagService.new()
  return setmetatable({ _flags = {} }, FlagService)
end

function FlagService:define(name, enabled, rollout, allowlist)
  self._flags[name] = { enabled = enabled, rollout = rollout, allowlist = allowlist or {} }
end

function FlagService:is_enabled(name, user_id)
  local f = self._flags[name]
  if not f or not f.enabled then return false end
  if f.rollout >= 100 then return true end
  if user_id then
    for _, u in ipairs(f.allowlist) do
      if u == user_id then return true end
    end
  end
  return false
end

M.FlagService = FlagService

-- ---------------------------------------------------------------------------
-- Utilities
-- ---------------------------------------------------------------------------

local function slugify(text)
  return (text:lower()
              :gsub("[^%a%d%s%-]", "")
              :gsub("[%s%-]+", "-")
              :gsub("^%-+", "")
              :gsub("%-+$", ""))
end

local function mask_email(email)
  local at = email:find("@")
  if not at then return email end
  local local_part = email:sub(1, at - 1)
  local domain     = email:sub(at + 1)
  local visible    = local_part:sub(1, math.min(2, #local_part))
  local stars      = string.rep("*", math.max(1, #local_part - 2))
  return visible .. stars .. "@" .. domain
end

-- FIXME[2025-06-08]: format_duration does not handle negative durations
local function format_duration(ms)
  if ms < 1000   then return ms .. "ms" end
  if ms < 60000  then return string.format("%.1fs", ms / 1000) end
  return string.format("%dm %ds", math.floor(ms / 60000), math.floor((ms % 60000) / 1000))
end

local function format_bytes(bytes)
  local units = { "B", "KB", "MB", "GB", "TB" }
  local v, i  = bytes, 1
  while v >= 1024 and i < #units do v = v / 1024; i = i + 1 end
  return string.format("%.2f %s", v, units[i])
end

M.slugify        = slugify
M.mask_email     = mask_email
M.format_duration = format_duration
M.format_bytes   = format_bytes

-- ---------------------------------------------------------------------------
-- Metrics
-- ---------------------------------------------------------------------------

-- TODO[2088-08-01][observability]: expose counters via Prometheus HTTP endpoint
local Counter = {}
Counter.__index = Counter

function Counter.new(name)
  return setmetatable({ name = name, _value = 0 }, Counter)
end

function Counter:inc(by) self._value = self._value + (by or 1) end
function Counter:read()  return self._value end
function Counter:reset() self._value = 0 end

-- REMOVEME[2018-10-05]: legacy stats shim — delete once all callers migrate to Counter
local function legacy_record(_name, _value) end

M.Counter        = Counter
M.legacy_record  = legacy_record

return M
