<?php
// sample.php — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

declare(strict_types=1);

namespace Sample;

use InvalidArgumentException;
use RuntimeException;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

class AppException extends RuntimeException
{
    public function __construct(
        private readonly string $code,
        string $message,
        private readonly int $statusCode = 500,
        private readonly mixed $details  = null,
    ) {
        parent::__construct($message);
    }

    public function getCode(): string  { return $this->code; }
    public function getStatus(): int   { return $this->statusCode; }
    public function getDetails(): mixed { return $this->details; }
}

class ValidationException extends AppException
{
    public function __construct(string $message, mixed $details = null)
    {
        parent::__construct('VALIDATION_ERROR', $message, 422, $details);
    }
}

class NotFoundException extends AppException
{
    public function __construct(string $resource)
    {
        parent::__construct('NOT_FOUND', "{$resource} not found", 404);
    }
}

class ConflictException extends AppException
{
    public function __construct(string $message)
    {
        parent::__construct('CONFLICT', $message, 409);
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

final class Result
{
    private function __construct(
        private readonly bool  $ok,
        private readonly mixed $value,
        private readonly ?string $error,
    ) {}

    public static function ok(mixed $value): self  { return new self(true, $value, null); }
    public static function err(string $error): self { return new self(false, null, $error); }

    public function isOk(): bool    { return $this->ok; }
    public function unwrap(): mixed
    {
        if (!$this->ok) throw new RuntimeException($this->error);
        return $this->value;
    }
    public function unwrapOr(mixed $fallback): mixed { return $this->ok ? $this->value : $fallback; }
    public function map(callable $fn): self
    {
        return $this->ok ? self::ok($fn($this->value)) : $this;
    }

    public static function try(callable $fn): self
    {
        try { return self::ok($fn()); } catch (\Throwable $e) { return self::err($e->getMessage()); }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

readonly class ValidationFailure
{
    public function __construct(
        public string  $field,
        public string  $message,
        public ?string $value = null,
    ) {}
}

// TODO[2020-02-15]: replace with symfony/validator or respect/validation
class Validator
{
    private const EMAIL_RE = '/^[^\s@]+@[^\s@]+\.[^\s@]+$/';

    public static function required(string $field, mixed $value): ?ValidationFailure
    {
        if ($value === null || $value === '') {
            return new ValidationFailure($field, 'is required');
        }
        return null;
    }

    public static function email(string $field, string $value): ?ValidationFailure
    {
        if (!preg_match(self::EMAIL_RE, $value)) {
            return new ValidationFailure($field, 'must be a valid email address', $value);
        }
        return null;
    }

    public static function minLength(string $field, string $value, int $min): ?ValidationFailure
    {
        if (mb_strlen($value) < $min) {
            return new ValidationFailure($field, "must be at least {$min} characters", $value);
        }
        return null;
    }

    public static function maxLength(string $field, string $value, int $max): ?ValidationFailure
    {
        if (mb_strlen($value) > $max) {
            return new ValidationFailure($field, "must be at most {$max} characters", $value);
        }
        return null;
    }

    public static function collect(ValidationFailure|null ...$failures): array
    {
        return array_values(array_filter($failures));
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

readonly class Config
{
    public function __construct(
        public string $host,
        public int    $port,
        public string $dsn,
        public string $jwtSecret,
        public int    $jwtExpiry,
        public int    $cacheTtl,
        public int    $rateMax,
        public int    $rateWindow,
        public string $env,
    ) {}

    public static function fromEnv(): self
    {
        return new self(
            host:       $_ENV['HOST']          ?? '0.0.0.0',
            port:       (int) ($_ENV['PORT']   ?? 3000),
            dsn:        $_ENV['DATABASE_URL']  ?? 'pgsql:host=localhost;dbname=app',
            jwtSecret:  $_ENV['JWT_SECRET']    ?? 'change-me',
            jwtExpiry:  (int) ($_ENV['JWT_EXPIRY']   ?? 3600),
            cacheTtl:   (int) ($_ENV['CACHE_TTL']    ?? 300),
            rateMax:    (int) ($_ENV['RATE_MAX']      ?? 100),
            rateWindow: (int) ($_ENV['RATE_WINDOW']   ?? 60),
            env:        $_ENV['APP_ENV'] ?? 'development',
        );
    }

    public function isProduction(): bool { return $this->env === 'production'; }
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

// HACK[2018-10-01]: APCu/array cache; replace with Predis before production
class Cache
{
    private array $store = [];

    public function get(string $key): mixed
    {
        $entry = $this->store[$key] ?? null;
        if ($entry === null || $entry['expires_at'] <= time()) {
            unset($this->store[$key]);
            return null;
        }
        return $entry['value'];
    }

    public function set(string $key, mixed $value, int $ttlSeconds): void
    {
        $this->store[$key] = ['value' => $value, 'expires_at' => time() + $ttlSeconds];
    }

    public function delete(string $key): void
    {
        unset($this->store[$key]);
    }

    public function getOrSet(string $key, int $ttl, callable $fn): mixed
    {
        $cached = $this->get($key);
        if ($cached !== null) return $cached;
        $value = $fn();
        $this->set($key, $value, $ttl);
        return $value;
    }

    public function cleanup(): void
    {
        $now = time();
        foreach ($this->store as $key => $entry) {
            if ($entry['expires_at'] <= $now) unset($this->store[$key]);
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

class RateLimiter
{
    private array $store = [];

    public function __construct(
        private readonly int $windowSec,
        private readonly int $maxRequests,
    ) {}

    public function check(string $key): array
    {
        $now   = time();
        $entry = $this->store[$key] ?? null;

        if ($entry === null || $entry['reset_at'] <= $now) {
            $entry = ['count' => 0, 'reset_at' => $now + $this->windowSec];
        }

        $entry['count']++;
        $this->store[$key] = $entry;

        $remaining  = max(0, $this->maxRequests - $entry['count']);
        $allowed    = $entry['count'] <= $this->maxRequests;
        $retryAfter = $allowed ? 0 : $entry['reset_at'] - $now;

        return ['allowed' => $allowed, 'remaining' => $remaining, 'retry_after' => $retryAfter];
    }
}

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-06-15][platform]: wire to Unleash or LaunchDarkly PHP SDK
class FeatureFlagService
{
    private array $flags = [];

    public function define(string $name, bool $enabled, int $rollout = 100, array $allowlist = []): void
    {
        $this->flags[$name] = ['enabled' => $enabled, 'rollout' => $rollout, 'allowlist' => $allowlist];
    }

    public function isEnabled(string $name, ?string $userId = null): bool
    {
        $flag = $this->flags[$name] ?? null;
        if ($flag === null || !$flag['enabled']) return false;
        if ($userId !== null && in_array($userId, $flag['allowlist'], true)) return true;
        if ($flag['rollout'] >= 100) return true;
        if ($flag['rollout'] <= 0)  return false;

        $hash   = md5("{$name}:" . ($userId ?? 'anon'));
        $bucket = hexdec(substr($hash, 0, 8)) % 100;
        return $bucket < $flag['rollout'];
    }
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

// FIXME[2021-04-01]: no distributed state; breaks with multiple PHP-FPM workers
class CircuitBreaker
{
    private string $state       = 'closed';
    private int    $failures    = 0;
    private int    $lastFailure = 0;

    public function __construct(
        private readonly int $threshold,
        private readonly int $resetTimeoutSec,
    ) {}

    public function call(callable $fn): mixed
    {
        if ($this->state === 'open') {
            if (time() - $this->lastFailure >= $this->resetTimeoutSec) {
                $this->state = 'half-open';
            } else {
                throw new AppException('CIRCUIT_OPEN', 'Service temporarily unavailable', 503);
            }
        }

        try {
            $result        = $fn();
            $this->failures = 0;
            $this->state   = 'closed';
            return $result;
        } catch (\Throwable $e) {
            $this->failures++;
            $this->lastFailure = time();
            if ($this->failures >= $this->threshold) $this->state = 'open';
            throw $e;
        }
    }

    public function getState(): string { return $this->state; }
}

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

readonly class Page
{
    public bool $hasNext;
    public bool $hasPrev;

    public function __construct(
        public array $items,
        public int   $total,
        public int   $pageNum,
        public int   $pageSize,
    ) {
        $this->hasNext = ($pageNum - 1) * $pageSize + count($items) < $total;
        $this->hasPrev = $pageNum > 1;
    }

    public static function of(array $source, int $pageNum, int $pageSize): self
    {
        $offset = ($pageNum - 1) * $pageSize;
        $items  = array_slice($source, $offset, $pageSize);
        return new self($items, count($source), $pageNum, $pageSize);
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

// FIXME[2025-06-08]: slugify does not handle Unicode letters; add intl extension check
function slugify(string $text): string
{
    $text = mb_strtolower($text);
    $text = preg_replace('/[^a-z0-9\s-]/', '', $text);
    $text = preg_replace('/[\s-]+/', '-', $text);
    return trim($text, '-');
}

function maskEmail(string $email): string
{
    [$local, $domain] = explode('@', $email, 2) + [1 => null];
    if ($domain === null) return $email;
    $visible = mb_strlen($local) > 2 ? mb_substr($local, 0, 2) : mb_substr($local, 0, 1);
    $stars   = str_repeat('*', max(1, mb_strlen($local) - 2));
    return "{$visible}{$stars}@{$domain}";
}

function truncate(string $s, int $maxLen, string $suffix = '…'): string
{
    return mb_strlen($s) <= $maxLen ? $s : mb_substr($s, 0, $maxLen - mb_strlen($suffix)) . $suffix;
}

function formatBytes(int $bytes): string
{
    $units = ['B', 'KB', 'MB', 'GB', 'TB'];
    $i = 0;
    $v = (float) $bytes;
    while ($v >= 1024 && $i < count($units) - 1) { $v /= 1024; $i++; }
    return round($v, 2) . ' ' . $units[$i];
}

function formatDuration(int $ms): string
{
    if ($ms < 1000)   return "{$ms}ms";
    if ($ms < 60_000) return round($ms / 1000, 1) . 's';
    return floor($ms / 60_000) . 'm ' . floor(($ms % 60_000) / 1000) . 's';
}

function chunk(array $arr, int $size): array
{
    return array_chunk($arr, $size);
}

function groupBy(array $arr, callable $keyFn): array
{
    $result = [];
    foreach ($arr as $item) {
        $result[$keyFn($item)][] = $item;
    }
    return $result;
}
