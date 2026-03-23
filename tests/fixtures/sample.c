/* sample.c — fixture file for timebomb scanner tests.
 *
 * Annotation inventory (hardcoded dates, never relative to today):
 *   Expired        (2018–2021): 4
 *   Expiring-soon  (2025-06):   1
 *   Future / OK    (2088/2099): 2
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>
#include <time.h>
#include <ctype.h>

/* ---------------------------------------------------------------------------
 * Config
 * ---------------------------------------------------------------------------*/

typedef struct {
    const char *host;
    int         port;
    const char *db_url;
    const char *jwt_secret;
    const char *env;
} Config;

static const char *getenv_or(const char *key, const char *fallback) {
    const char *val = getenv(key);
    return val ? val : fallback;
}

Config config_from_env(void) {
    Config cfg;
    cfg.host       = getenv_or("HOST",       "0.0.0.0");
    cfg.port       = atoi(getenv_or("PORT",  "3000"));
    cfg.db_url     = getenv_or("DB_URL",     "postgres://localhost/app");
    cfg.jwt_secret = getenv_or("JWT_SECRET", "change-me");
    cfg.env        = getenv_or("APP_ENV",    "development");
    return cfg;
}

bool config_is_production(const Config *cfg) {
    return strcmp(cfg->env, "production") == 0;
}

/* ---------------------------------------------------------------------------
 * Result type
 * ---------------------------------------------------------------------------*/

typedef enum { RESULT_OK, RESULT_ERR } ResultTag;

typedef struct {
    ResultTag tag;
    union {
        void       *value;
        const char *error;
    };
} Result;

Result result_ok(void *value) {
    return (Result){ .tag = RESULT_OK, .value = value };
}

Result result_err(const char *error) {
    return (Result){ .tag = RESULT_ERR, .error = error };
}

bool result_is_ok(Result r)  { return r.tag == RESULT_OK; }
bool result_is_err(Result r) { return r.tag == RESULT_ERR; }

/* ---------------------------------------------------------------------------
 * Validation
 * ---------------------------------------------------------------------------*/

/* TODO[2019-01-15]: replace with a proper schema-validation library once one stabilises */
bool validate_required(const char *value) {
    return value != NULL && strlen(value) > 0;
}

bool validate_email(const char *value) {
    if (!value) return false;
    const char *at = strchr(value, '@');
    if (!at || at == value) return false;
    const char *dot = strchr(at + 1, '.');
    return dot && dot > at + 1 && *(dot + 1) != '\0';
}

bool validate_min_length(const char *value, size_t min) {
    return value && strlen(value) >= min;
}

/* ---------------------------------------------------------------------------
 * Cache (open-addressing hash table)
 * ---------------------------------------------------------------------------*/

#define CACHE_BUCKETS 256

typedef struct CacheEntry {
    char           *key;
    void           *value;
    time_t          expires_at;
    struct CacheEntry *next;
} CacheEntry;

typedef struct {
    CacheEntry *buckets[CACHE_BUCKETS];
} Cache;

/* HACK[2020-07-01]: linear chaining without resizing; replace before any load testing */
Cache *cache_new(void) {
    Cache *c = calloc(1, sizeof(Cache));
    return c;
}

static unsigned cache_hash(const char *key) {
    unsigned h = 5381;
    while (*key) h = (h << 5) + h + (unsigned char)*key++;
    return h % CACHE_BUCKETS;
}

void *cache_get(Cache *c, const char *key) {
    unsigned idx = cache_hash(key);
    time_t now = time(NULL);
    for (CacheEntry *e = c->buckets[idx]; e; e = e->next) {
        if (strcmp(e->key, key) == 0) {
            return e->expires_at > now ? e->value : NULL;
        }
    }
    return NULL;
}

void cache_set(Cache *c, const char *key, void *value, int ttl_sec) {
    unsigned idx = cache_hash(key);
    for (CacheEntry *e = c->buckets[idx]; e; e = e->next) {
        if (strcmp(e->key, key) == 0) {
            e->value      = value;
            e->expires_at = time(NULL) + ttl_sec;
            return;
        }
    }
    CacheEntry *e = malloc(sizeof(CacheEntry));
    e->key        = strdup(key);
    e->value      = value;
    e->expires_at = time(NULL) + ttl_sec;
    e->next       = c->buckets[idx];
    c->buckets[idx] = e;
}

void cache_free(Cache *c) {
    for (int i = 0; i < CACHE_BUCKETS; i++) {
        CacheEntry *e = c->buckets[i];
        while (e) {
            CacheEntry *next = e->next;
            free(e->key);
            free(e);
            e = next;
        }
    }
    free(c);
}

/* ---------------------------------------------------------------------------
 * Rate limiter
 * ---------------------------------------------------------------------------*/

/* FIXME[2018-12-01]: single-process only; add shared-memory or Redis for multi-process */
typedef struct {
    int    window_sec;
    int    max_requests;
    int    count;
    time_t reset_at;
} RateLimiter;

RateLimiter rate_limiter_new(int window_sec, int max_requests) {
    return (RateLimiter){ window_sec, max_requests, 0, 0 };
}

typedef struct {
    bool allowed;
    int  remaining;
    int  retry_after;
} RateResult;

RateResult rate_check(RateLimiter *rl, const char *key) {
    (void)key;
    time_t now = time(NULL);
    if (now >= rl->reset_at) {
        rl->count    = 0;
        rl->reset_at = now + rl->window_sec;
    }
    rl->count++;
    bool allowed = rl->count <= rl->max_requests;
    return (RateResult){
        .allowed     = allowed,
        .remaining   = allowed ? rl->max_requests - rl->count : 0,
        .retry_after = allowed ? 0 : (int)(rl->reset_at - now)
    };
}

/* ---------------------------------------------------------------------------
 * Pagination
 * ---------------------------------------------------------------------------*/

typedef struct {
    void  **items;
    size_t  count;
    size_t  total;
    int     page_num;
    int     page_size;
    bool    has_next;
    bool    has_prev;
} Page;

Page paginate(void **items, size_t total, int page_num, int page_size) {
    size_t offset = (size_t)((page_num - 1) * page_size);
    if (offset > total) offset = total;
    size_t count = (size_t)page_size;
    if (offset + count > total) count = total - offset;
    return (Page){
        .items     = items + offset,
        .count     = count,
        .total     = total,
        .page_num  = page_num,
        .page_size = page_size,
        .has_next  = offset + count < total,
        .has_prev  = page_num > 1
    };
}

/* ---------------------------------------------------------------------------
 * Utilities
 * ---------------------------------------------------------------------------*/

void slugify(const char *src, char *dst, size_t dst_size) {
    size_t j = 0;
    bool   last_dash = false;
    for (size_t i = 0; src[i] && j + 1 < dst_size; i++) {
        char c = (char)tolower((unsigned char)src[i]);
        if (isalnum((unsigned char)c)) {
            dst[j++]  = c;
            last_dash = false;
        } else if (!last_dash && j > 0) {
            dst[j++]  = '-';
            last_dash = true;
        }
    }
    if (j > 0 && dst[j - 1] == '-') j--;
    dst[j] = '\0';
}

/* FIXME[2025-06-08]: format_duration does not handle negative values */
void format_duration(long ms, char *buf, size_t buf_size) {
    if (ms < 1000)
        snprintf(buf, buf_size, "%ldms", ms);
    else if (ms < 60000)
        snprintf(buf, buf_size, "%.1fs", ms / 1000.0);
    else
        snprintf(buf, buf_size, "%ldm %lds", ms / 60000, (ms % 60000) / 1000);
}

void format_bytes(long bytes, char *buf, size_t buf_size) {
    const char *units[] = { "B", "KB", "MB", "GB", "TB" };
    double v = (double)bytes;
    int    i = 0;
    while (v >= 1024 && i < 4) { v /= 1024; i++; }
    snprintf(buf, buf_size, "%.2f %s", v, units[i]);
}

/* ---------------------------------------------------------------------------
 * Feature flags
 * ---------------------------------------------------------------------------*/

/* TODO[2099-06-01][platform]: replace compile-time flags with a remote config service */
typedef struct {
    const char *name;
    bool        enabled;
    int         rollout;
} FeatureFlag;

bool flag_enabled(const FeatureFlag *flag, int user_bucket) {
    if (!flag || !flag->enabled) return false;
    return flag->rollout >= 100 || user_bucket < flag->rollout;
}

/* ---------------------------------------------------------------------------
 * Metrics
 * ---------------------------------------------------------------------------*/

/* TODO[2088-04-01][observability]: wire up Prometheus client once one exists for C */
typedef struct {
    const char *name;
    long        value;
} Counter;

Counter counter_new(const char *name) {
    return (Counter){ name, 0 };
}

void counter_inc(Counter *c, long by) { c->value += by; }
long counter_read(const Counter *c)   { return c->value; }
void counter_reset(Counter *c)        { c->value = 0; }
