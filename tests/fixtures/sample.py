# This file contains structured timebomb annotations for use in integration tests.
# Dates are chosen to be permanently in the past or far future so tests never
# depend on the current wall-clock date.

# FIXME[2019-03-15]: monkey-patch for upstream library bug, remove after upgrade
import os
import sys
import json
import time
import math
import hashlib
import logging
import threading
import functools
import itertools
import collections
import contextlib
import heapq
import re
from typing import Any, Callable, Dict, List, Optional, Tuple, Union, Iterator, Generator
from dataclasses import dataclass, field
from pathlib import Path
from enum import Enum, auto

logger = logging.getLogger(__name__)

# ── Expired annotations (date in the past) ────────────────────────────────

# TODO[2020-01-01]: remove compatibility shim for Python 2
legacy_compat = True

_env = os.environ.get("LEGACY", "")


# HACK[2018-11-20]: work around broken CSV parser, replace with stdlib
def parse_csv_hacky(line):
    return line.split(",")


# TEMP[2020-06-30]: temporary feature flag for rollout
ENABLE_NEW_DASHBOARD = False


# REMOVEME[2021-02-28]: dead code left over from old billing system
def old_billing_logic():
    pass


# TODO[2020-01-01][carol]: carol to remove after data migration completes
MIGRATION_DONE = False

# ── Expiring-soon annotations (dates used by tests injecting a close `today`) ──
# Tests that need "expiring soon" status should inject today = 2025-06-01 or similar.

# TODO[2025-06-10]: rotate service account credentials before expiry
SERVICE_ACCOUNT_KEY = "placeholder"

# FIXME[2025-06-08]: disable debug logging before next release
DEBUG_LOGGING = True

# ── Future annotations (far future — always OK) ───────────────────────────


# TODO[2099-01-01]: migrate to async IO once team is trained
def sync_fetch(url):
    return url


# FIXME[2099-12-31]: long-term tech debt, tracked in issue #1234
TECH_DEBT = "acknowledged"


# HACK[2088-04-01]: workaround for platform limitation, revisit in next decade
def platform_workaround():
    return 42


# TODO[2099-01-01][dave]: dave's team owns this cleanup task
OWNED_CLEANUP = None

# ── Non-matching annotations (must be ignored by scanner) ────────────────

# TODO: plain todo with no date bracket — must NOT be matched
# FIXME: another undecorated one — must NOT be matched
# NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
# TODO [2020-01-01]: space between tag and bracket — must NOT be matched

# =============================================================================
# Domain models
# =============================================================================


class UserRole(Enum):
    ADMIN = auto()
    EDITOR = auto()
    VIEWER = auto()
    GUEST = auto()


class AccountStatus(Enum):
    ACTIVE = auto()
    SUSPENDED = auto()
    PENDING = auto()
    DELETED = auto()


@dataclass
class Address:
    street: str
    city: str
    state: str
    country: str
    postal_code: str

    def formatted(self) -> str:
        return f"{self.street}, {self.city}, {self.state} {self.postal_code}, {self.country}"


@dataclass
class UserProfile:
    bio: Optional[str]
    avatar_url: Optional[str]
    website: Optional[str]
    location: Optional[str]
    timezone: str = "UTC"
    language: str = "en"


@dataclass
class User:
    id: int
    username: str
    email: str
    role: UserRole
    status: AccountStatus
    profile: Optional[UserProfile] = None
    address: Optional[Address] = None
    created_at: float = field(default_factory=time.time)
    updated_at: float = field(default_factory=time.time)
    tags: List[str] = field(default_factory=list)
    metadata: Dict[str, Any] = field(default_factory=dict)

    def is_active(self) -> bool:
        return self.status == AccountStatus.ACTIVE

    def is_admin(self) -> bool:
        return self.role == UserRole.ADMIN

    def display_name(self) -> str:
        if self.profile and self.profile.bio:
            return f"{self.username} ({self.profile.bio[:20]})"
        return self.username

    def age_days(self) -> float:
        return (time.time() - self.created_at) / 86400.0

    def to_dict(self) -> Dict[str, Any]:
        return {
            "id": self.id,
            "username": self.username,
            "email": self.email,
            "role": self.role.name,
            "status": self.status.name,
            "created_at": self.created_at,
        }


# =============================================================================
# Repository pattern
# =============================================================================


class UserNotFoundError(Exception):
    def __init__(self, user_id: int):
        super().__init__(f"User {user_id} not found")
        self.user_id = user_id


class DuplicateUserError(Exception):
    def __init__(self, email: str):
        super().__init__(f"User with email {email!r} already exists")
        self.email = email


class UserRepository:
    """In-memory user repository with indexing by id and email."""

    def __init__(self):
        self._by_id: Dict[int, User] = {}
        self._by_email: Dict[str, int] = {}
        self._lock = threading.RLock()
        self._next_id = 1

    def add(self, user: User) -> User:
        with self._lock:
            if user.email in self._by_email:
                raise DuplicateUserError(user.email)
            user.id = self._next_id
            self._next_id += 1
            self._by_id[user.id] = user
            self._by_email[user.email] = user.id
            return user

    def get(self, user_id: int) -> User:
        with self._lock:
            try:
                return self._by_id[user_id]
            except KeyError:
                raise UserNotFoundError(user_id)

    def find_by_email(self, email: str) -> Optional[User]:
        with self._lock:
            uid = self._by_email.get(email)
            if uid is None:
                return None
            return self._by_id.get(uid)

    def update(self, user: User) -> User:
        with self._lock:
            if user.id not in self._by_id:
                raise UserNotFoundError(user.id)
            user.updated_at = time.time()
            self._by_id[user.id] = user
            return user

    def delete(self, user_id: int) -> None:
        with self._lock:
            user = self._by_id.pop(user_id, None)
            if user is None:
                raise UserNotFoundError(user_id)
            self._by_email.pop(user.email, None)

    def list_all(self) -> List[User]:
        with self._lock:
            return list(self._by_id.values())

    def list_by_role(self, role: UserRole) -> List[User]:
        with self._lock:
            return [u for u in self._by_id.values() if u.role == role]

    def list_by_status(self, status: AccountStatus) -> List[User]:
        with self._lock:
            return [u for u in self._by_id.values() if u.status == status]

    def count(self) -> int:
        with self._lock:
            return len(self._by_id)

    def search(self, query: str) -> List[User]:
        q = query.lower()
        with self._lock:
            return [
                u
                for u in self._by_id.values()
                if q in u.username.lower() or q in u.email.lower()
            ]


# =============================================================================
# Cache implementation
# =============================================================================


class CacheEntry:
    __slots__ = ("value", "expires_at")

    def __init__(self, value: Any, ttl: float):
        self.value = value
        self.expires_at = time.time() + ttl

    def is_expired(self) -> bool:
        return time.time() > self.expires_at


class LRUCache:
    """Thread-safe LRU cache with TTL support."""

    def __init__(self, capacity: int, default_ttl: float = 300.0):
        self._capacity = capacity
        self._default_ttl = default_ttl
        self._data: "collections.OrderedDict[str, CacheEntry]" = collections.OrderedDict()
        self._lock = threading.RLock()
        self._hits = 0
        self._misses = 0
        self._evictions = 0

    def get(self, key: str) -> Optional[Any]:
        with self._lock:
            entry = self._data.get(key)
            if entry is None:
                self._misses += 1
                return None
            if entry.is_expired():
                del self._data[key]
                self._misses += 1
                return None
            self._data.move_to_end(key)
            self._hits += 1
            return entry.value

    def set(self, key: str, value: Any, ttl: Optional[float] = None) -> None:
        effective_ttl = ttl if ttl is not None else self._default_ttl
        with self._lock:
            if key in self._data:
                self._data.move_to_end(key)
            self._data[key] = CacheEntry(value, effective_ttl)
            if len(self._data) > self._capacity:
                self._data.popitem(last=False)
                self._evictions += 1

    def delete(self, key: str) -> bool:
        with self._lock:
            return self._data.pop(key, None) is not None

    def clear(self) -> None:
        with self._lock:
            self._data.clear()

    def hit_rate(self) -> float:
        total = self._hits + self._misses
        return self._hits / total if total > 0 else 0.0

    def stats(self) -> Dict[str, Any]:
        with self._lock:
            return {
                "size": len(self._data),
                "capacity": self._capacity,
                "hits": self._hits,
                "misses": self._misses,
                "evictions": self._evictions,
                "hit_rate": self.hit_rate(),
            }


# =============================================================================
# Event system
# =============================================================================


@dataclass
class Event:
    name: str
    payload: Any
    timestamp: float = field(default_factory=time.time)
    source: Optional[str] = None


EventHandler = Callable[["Event"], None]


class EventBus:
    """Simple synchronous pub/sub event bus."""

    def __init__(self):
        self._handlers: Dict[str, List[Any]] = collections.defaultdict(list)
        self._lock = threading.RLock()
        self._published = 0
        self._errors = 0

    def subscribe(self, event_name: str, handler) -> None:
        with self._lock:
            self._handlers[event_name].append(handler)

    def unsubscribe(self, event_name: str, handler) -> None:
        with self._lock:
            handlers = self._handlers.get(event_name, [])
            try:
                handlers.remove(handler)
            except ValueError:
                pass

    def publish(self, event: Event) -> int:
        with self._lock:
            handlers = list(self._handlers.get(event.name, []))
        dispatched = 0
        for handler in handlers:
            try:
                handler(event)
                dispatched += 1
            except Exception as exc:
                self._errors += 1
                logger.error("Handler error for event %s: %s", event.name, exc)
        self._published += 1
        return dispatched

    def stats(self) -> Dict[str, int]:
        with self._lock:
            return {
                "published": self._published,
                "errors": self._errors,
                "subscriptions": sum(len(v) for v in self._handlers.values()),
            }


# =============================================================================
# Rate limiter (token bucket)
# =============================================================================


class RateLimiter:
    """Token-bucket rate limiter."""

    def __init__(self, rate: float, burst: int):
        self._rate = rate
        self._burst = burst
        self._tokens: float = float(burst)
        self._last_refill = time.monotonic()
        self._lock = threading.Lock()

    def _refill(self) -> None:
        now = time.monotonic()
        elapsed = now - self._last_refill
        self._tokens = min(float(self._burst), self._tokens + elapsed * self._rate)
        self._last_refill = now

    def allow(self, cost: float = 1.0) -> bool:
        with self._lock:
            self._refill()
            if self._tokens >= cost:
                self._tokens -= cost
                return True
            return False

    def wait_and_allow(self, cost: float = 1.0, timeout: float = 5.0) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.allow(cost):
                return True
            time.sleep(0.001)
        return False


# =============================================================================
# Pipeline / functional utilities
# =============================================================================


def chunks(lst: List[Any], size: int) -> Generator[List[Any], None, None]:
    """Yield successive chunks of `size` from `lst`."""
    for i in range(0, len(lst), size):
        yield lst[i : i + size]


def flatten(nested: List[List[Any]]) -> List[Any]:
    return list(itertools.chain.from_iterable(nested))


def partition(pred, iterable) -> Tuple[List[Any], List[Any]]:
    """Split an iterable into two lists: those matching pred and those not."""
    yes, no = [], []
    for item in iterable:
        (yes if pred(item) else no).append(item)
    return yes, no


def retry(fn, attempts: int = 3, delay: float = 0.1, exceptions=(Exception,)):
    last_exc: Exception = RuntimeError("retry called with attempts=0")
    for attempt in range(attempts):
        try:
            return fn()
        except exceptions as exc:
            last_exc = exc
            if attempt < attempts - 1:
                time.sleep(delay * (2**attempt))
    raise last_exc


def deep_merge(base: Dict, override: Dict) -> Dict:
    """Recursively merge two dicts; override wins on conflicts."""
    result = dict(base)
    for k, v in override.items():
        if k in result and isinstance(result[k], dict) and isinstance(v, dict):
            result[k] = deep_merge(result[k], v)
        else:
            result[k] = v
    return result


def stable_hash(obj: Any) -> str:
    raw = json.dumps(obj, sort_keys=True, default=str)
    return hashlib.sha256(raw.encode()).hexdigest()[:16]


# =============================================================================
# Configuration management
# =============================================================================


@dataclass
class DatabaseConfig:
    host: str = "localhost"
    port: int = 5432
    name: str = "app"
    user: str = "app"
    password: str = ""
    pool_size: int = 10
    max_overflow: int = 5
    connect_timeout: float = 5.0

    @property
    def dsn(self) -> str:
        return f"postgresql://{self.user}:{self.password}@{self.host}:{self.port}/{self.name}"


@dataclass
class CacheConfig:
    backend: str = "memory"
    capacity: int = 1000
    default_ttl: float = 300.0
    redis_url: Optional[str] = None


@dataclass
class AppConfig:
    debug: bool = False
    log_level: str = "INFO"
    secret_key: str = ""
    allowed_hosts: List[str] = field(default_factory=list)
    database: DatabaseConfig = field(default_factory=DatabaseConfig)
    cache: CacheConfig = field(default_factory=CacheConfig)
    max_page_size: int = 100
    default_page_size: int = 20
    request_timeout: float = 30.0
    cors_origins: List[str] = field(default_factory=list)

    @classmethod
    def from_env(cls) -> "AppConfig":
        cfg = cls()
        cfg.debug = os.environ.get("DEBUG", "").lower() in ("1", "true", "yes")
        cfg.log_level = os.environ.get("LOG_LEVEL", "INFO").upper()
        cfg.secret_key = os.environ.get("SECRET_KEY", "")
        if hosts := os.environ.get("ALLOWED_HOSTS"):
            cfg.allowed_hosts = [h.strip() for h in hosts.split(",")]
        cfg.database.host = os.environ.get("DB_HOST", "localhost")
        cfg.database.port = int(os.environ.get("DB_PORT", "5432"))
        cfg.database.name = os.environ.get("DB_NAME", "app")
        cfg.database.user = os.environ.get("DB_USER", "app")
        cfg.database.password = os.environ.get("DB_PASSWORD", "")
        return cfg


# =============================================================================
# Service layer
# =============================================================================


class UserService:
    """Application service coordinating user business logic."""

    def __init__(
        self,
        repo: UserRepository,
        cache: LRUCache,
        bus: EventBus,
        config: AppConfig,
    ):
        self._repo = repo
        self._cache = cache
        self._bus = bus
        self._config = config

    def _cache_key(self, user_id: int) -> str:
        return f"user:{user_id}"

    def get_user(self, user_id: int) -> User:
        key = self._cache_key(user_id)
        cached = self._cache.get(key)
        if cached is not None:
            return cached
        user = self._repo.get(user_id)
        self._cache.set(key, user, ttl=60.0)
        return user

    def create_user(
        self,
        username: str,
        email: str,
        role: UserRole = UserRole.VIEWER,
    ) -> User:
        user = User(
            id=0,
            username=username,
            email=email,
            role=role,
            status=AccountStatus.PENDING,
        )
        created = self._repo.add(user)
        self._bus.publish(Event("user.created", {"user_id": created.id}))
        return created

    def activate_user(self, user_id: int) -> User:
        user = self._repo.get(user_id)
        user.status = AccountStatus.ACTIVE
        updated = self._repo.update(user)
        self._cache.delete(self._cache_key(user_id))
        self._bus.publish(Event("user.activated", {"user_id": user_id}))
        return updated

    def suspend_user(self, user_id: int, reason: str = "") -> User:
        user = self._repo.get(user_id)
        user.status = AccountStatus.SUSPENDED
        user.metadata["suspension_reason"] = reason
        updated = self._repo.update(user)
        self._cache.delete(self._cache_key(user_id))
        self._bus.publish(Event("user.suspended", {"user_id": user_id, "reason": reason}))
        return updated

    def delete_user(self, user_id: int) -> None:
        self._repo.delete(user_id)
        self._cache.delete(self._cache_key(user_id))
        self._bus.publish(Event("user.deleted", {"user_id": user_id}))

    def list_active_admins(self) -> List[User]:
        return [
            u
            for u in self._repo.list_by_role(UserRole.ADMIN)
            if u.status == AccountStatus.ACTIVE
        ]

    def bulk_activate(self, user_ids: List[int]) -> Tuple[int, int]:
        success = failure = 0
        for uid in user_ids:
            try:
                self.activate_user(uid)
                success += 1
            except Exception:
                failure += 1
        return success, failure


# =============================================================================
# Pipeline / processing
# =============================================================================


class PipelineStage:
    """Abstract pipeline stage."""

    def process(self, item: Any) -> Any:
        raise NotImplementedError

    def __or__(self, other: "PipelineStage") -> "CompositePipeline":
        return CompositePipeline([self, other])


class CompositePipeline(PipelineStage):
    def __init__(self, stages: List[PipelineStage]):
        self._stages = stages

    def process(self, item: Any) -> Any:
        for stage in self._stages:
            item = stage.process(item)
        return item

    def __or__(self, other: "PipelineStage") -> "CompositePipeline":
        return CompositePipeline(self._stages + [other])


class FilterStage(PipelineStage):
    def __init__(self, predicate):
        self._predicate = predicate

    def process(self, item: Any) -> Any:
        if self._predicate(item):
            return item
        return None


class TransformStage(PipelineStage):
    def __init__(self, transform):
        self._transform = transform

    def process(self, item: Any) -> Any:
        return self._transform(item)


class ValidateStage(PipelineStage):
    def __init__(self, schema: Dict[str, type]):
        self._schema = schema

    def process(self, item: Dict) -> Dict:
        for key, expected_type in self._schema.items():
            if key not in item:
                raise ValueError(f"Missing required field: {key!r}")
            if not isinstance(item[key], expected_type):
                raise TypeError(f"Field {key!r} must be {expected_type.__name__}")
        return item


# =============================================================================
# Statistics utilities
# =============================================================================


def mean(values: List[float]) -> float:
    if not values:
        return 0.0
    return sum(values) / len(values)


def variance(values: List[float]) -> float:
    if len(values) < 2:
        return 0.0
    m = mean(values)
    return sum((v - m) ** 2 for v in values) / (len(values) - 1)


def std_dev(values: List[float]) -> float:
    return math.sqrt(variance(values))


def percentile(values: List[float], p: float) -> float:
    """Compute the p-th percentile (0–100) of sorted values."""
    if not values:
        return 0.0
    sorted_vals = sorted(values)
    idx = (p / 100.0) * (len(sorted_vals) - 1)
    lo = int(idx)
    hi = min(lo + 1, len(sorted_vals) - 1)
    frac = idx - lo
    return sorted_vals[lo] * (1 - frac) + sorted_vals[hi] * frac


def histogram(values: List[float], bins: int = 10) -> List[Tuple[float, float, int]]:
    """Return list of (lo, hi, count) tuples."""
    if not values:
        return []
    lo, hi = min(values), max(values)
    if lo == hi:
        return [(lo, hi, len(values))]
    width = (hi - lo) / bins
    result = []
    for i in range(bins):
        bin_lo = lo + i * width
        bin_hi = lo + (i + 1) * width
        count = sum(1 for v in values if bin_lo <= v < bin_hi)
        result.append((bin_lo, bin_hi, count))
    return result


# =============================================================================
# File utilities
# =============================================================================


def read_json(path: Union[str, Path]) -> Any:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def write_json(path: Union[str, Path], data: Any, indent: int = 2) -> None:
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=indent, ensure_ascii=False)


def read_lines(path: Union[str, Path]) -> List[str]:
    with open(path, encoding="utf-8") as f:
        return f.readlines()


def write_lines(path: Union[str, Path], lines: List[str]) -> None:
    with open(path, "w", encoding="utf-8") as f:
        f.writelines(lines)


def atomic_write(path: Union[str, Path], content: str) -> None:
    """Write content to a temp file then rename — avoids partial writes."""
    p = Path(path)
    tmp = p.with_suffix(".tmp")
    try:
        tmp.write_text(content, encoding="utf-8")
        tmp.rename(p)
    except Exception:
        tmp.unlink(missing_ok=True)
        raise


def ensure_dir(path: Union[str, Path]) -> Path:
    p = Path(path)
    p.mkdir(parents=True, exist_ok=True)
    return p


def file_size_bytes(path: Union[str, Path]) -> int:
    return os.path.getsize(path)


def glob_files(root: Union[str, Path], pattern: str) -> List[Path]:
    return sorted(Path(root).rglob(pattern))


# =============================================================================
# String utilities
# =============================================================================


def truncate(s: str, max_len: int, ellipsis: str = "…") -> str:
    if len(s) <= max_len:
        return s
    return s[: max_len - len(ellipsis)] + ellipsis


def slugify(s: str) -> str:
    s = s.lower().strip()
    s = re.sub(r"[^\w\s-]", "", s)
    s = re.sub(r"[\s_-]+", "-", s)
    return s.strip("-")


def camel_to_snake(s: str) -> str:
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", s)
    s = re.sub(r"([a-z\d])([A-Z])", r"\1_\2", s)
    return s.lower()


def snake_to_camel(s: str) -> str:
    parts = s.split("_")
    return parts[0] + "".join(p.capitalize() for p in parts[1:])


def wrap_text(text: str, width: int = 80) -> List[str]:
    words = text.split()
    lines: List[str] = []
    current: List[str] = []
    length = 0
    for word in words:
        if length + len(word) + (1 if current else 0) > width:
            lines.append(" ".join(current))
            current = [word]
            length = len(word)
        else:
            current.append(word)
            length += len(word) + (1 if len(current) > 1 else 0)
    if current:
        lines.append(" ".join(current))
    return lines


# =============================================================================
# Bloom filter (space-efficient probabilistic set membership)
# =============================================================================


class BloomFilter:
    """Simple counting Bloom filter using multiple hash functions."""

    def __init__(self, capacity: int = 10_000, error_rate: float = 0.01):
        self._capacity = capacity
        self._error_rate = error_rate
        # Optimal bit array size
        m = int(-capacity * math.log(error_rate) / (math.log(2) ** 2))
        self._bits = bytearray(m)
        self._size = m
        # Optimal number of hash functions
        self._k = max(1, int((m / capacity) * math.log(2)))
        self._count = 0

    def _hashes(self, item: str) -> List[int]:
        h1 = int(hashlib.md5(item.encode()).hexdigest(), 16)
        h2 = int(hashlib.sha1(item.encode()).hexdigest(), 16)
        return [(h1 + i * h2) % self._size for i in range(self._k)]

    def add(self, item: str) -> None:
        for idx in self._hashes(item):
            self._bits[idx] = 1
        self._count += 1

    def __contains__(self, item: str) -> bool:
        return all(self._bits[idx] for idx in self._hashes(item))

    def __len__(self) -> int:
        return self._count

    @property
    def estimated_fpr(self) -> float:
        filled = sum(self._bits)
        return (filled / self._size) ** self._k


# =============================================================================
# Priority queue (min-heap)
# =============================================================================


@dataclass(order=True)
class PrioritizedItem:
    priority: float
    item: Any = field(compare=False)


class MinHeap:
    def __init__(self):
        self._heap: List[PrioritizedItem] = []

    def push(self, item: Any, priority: float) -> None:
        heapq.heappush(self._heap, PrioritizedItem(priority, item))

    def pop(self) -> Tuple[Any, float]:
        entry = heapq.heappop(self._heap)
        return entry.item, entry.priority

    def peek(self) -> Tuple[Any, float]:
        entry = self._heap[0]
        return entry.item, entry.priority

    def __len__(self) -> int:
        return len(self._heap)

    def __bool__(self) -> bool:
        return bool(self._heap)


# =============================================================================
# Context managers
# =============================================================================


@contextlib.contextmanager
def timed(label: str = "") -> Iterator[Dict[str, float]]:
    """Context manager that records elapsed time in a dict."""
    info: Dict[str, float] = {}
    start = time.perf_counter()
    try:
        yield info
    finally:
        info["elapsed"] = time.perf_counter() - start
        if label:
            logger.debug("%s: %.4fs", label, info["elapsed"])


@contextlib.contextmanager
def suppress_output() -> Iterator[None]:
    """Redirect stdout/stderr to /dev/null within the block."""
    with open(os.devnull, "w") as devnull:
        old_stdout, old_stderr = sys.stdout, sys.stderr
        sys.stdout = devnull
        sys.stderr = devnull
        try:
            yield
        finally:
            sys.stdout = old_stdout
            sys.stderr = old_stderr


# =============================================================================
# Decorators
# =============================================================================


def memoize(fn):
    cache: Dict[Any, Any] = {}

    @functools.wraps(fn)
    def wrapper(*args):
        if args not in cache:
            cache[args] = fn(*args)
        return cache[args]

    wrapper.cache = cache  # type: ignore[attr-defined]
    return wrapper


def singleton(cls):
    instances: Dict[type, Any] = {}

    @functools.wraps(cls)
    def get_instance(*args, **kwargs):
        if cls not in instances:
            instances[cls] = cls(*args, **kwargs)
        return instances[cls]

    return get_instance


def validate_types(**type_map):
    """Decorator that validates argument types at call time."""

    def decorator(fn):
        @functools.wraps(fn)
        def wrapper(*args, **kwargs):
            bound = fn.__code__.co_varnames
            for i, (name, expected) in enumerate(type_map.items()):
                idx = list(bound).index(name) if name in bound else -1
                if idx >= 0 and idx < len(args):
                    if not isinstance(args[idx], expected):
                        raise TypeError(
                            f"Argument {name!r} must be {expected.__name__}, "
                            f"got {type(args[idx]).__name__}"
                        )
            return fn(*args, **kwargs)

        return wrapper

    return decorator


# =============================================================================
# Graph algorithms
# =============================================================================


class DirectedGraph:
    """Adjacency-list directed graph."""

    def __init__(self):
        self._adj: Dict[str, List[str]] = collections.defaultdict(list)
        self._nodes: set = set()

    def add_node(self, node: str) -> None:
        self._nodes.add(node)

    def add_edge(self, src: str, dst: str) -> None:
        self._nodes.add(src)
        self._nodes.add(dst)
        self._adj[src].append(dst)

    def neighbors(self, node: str) -> List[str]:
        return self._adj.get(node, [])

    def bfs(self, start: str) -> List[str]:
        visited: set = set()
        queue = collections.deque([start])
        order: List[str] = []
        while queue:
            node = queue.popleft()
            if node in visited:
                continue
            visited.add(node)
            order.append(node)
            queue.extend(self._adj.get(node, []))
        return order

    def dfs(self, start: str) -> List[str]:
        visited: set = set()
        order: List[str] = []

        def _dfs(node: str) -> None:
            if node in visited:
                return
            visited.add(node)
            order.append(node)
            for neighbor in self._adj.get(node, []):
                _dfs(neighbor)

        _dfs(start)
        return order

    def topological_sort(self) -> List[str]:
        in_degree: Dict[str, int] = {n: 0 for n in self._nodes}
        for src in self._adj:
            for dst in self._adj[src]:
                in_degree[dst] = in_degree.get(dst, 0) + 1
        queue = collections.deque([n for n, d in in_degree.items() if d == 0])
        result: List[str] = []
        while queue:
            node = queue.popleft()
            result.append(node)
            for neighbor in self._adj.get(node, []):
                in_degree[neighbor] -= 1
                if in_degree[neighbor] == 0:
                    queue.append(neighbor)
        return result

    def has_cycle(self) -> bool:
        return len(self.topological_sort()) != len(self._nodes)


# =============================================================================
# String search (KMP)
# =============================================================================


def kmp_search(text: str, pattern: str) -> List[int]:
    """Return all start indices where pattern occurs in text."""
    if not pattern:
        return list(range(len(text) + 1))
    # Build failure function
    failure = [0] * len(pattern)
    j = 0
    for i in range(1, len(pattern)):
        while j > 0 and pattern[i] != pattern[j]:
            j = failure[j - 1]
        if pattern[i] == pattern[j]:
            j += 1
        failure[i] = j
    # Search
    results: List[int] = []
    j = 0
    for i, ch in enumerate(text):
        while j > 0 and ch != pattern[j]:
            j = failure[j - 1]
        if ch == pattern[j]:
            j += 1
        if j == len(pattern):
            results.append(i - len(pattern) + 1)
            j = failure[j - 1]
    return results


# =============================================================================
# Minimal HTTP request helpers (no external deps)
# =============================================================================


def build_query_string(params: Dict[str, Any]) -> str:
    from urllib.parse import urlencode

    return urlencode({k: str(v) for k, v in params.items()})


def parse_headers(raw: str) -> Dict[str, str]:
    headers: Dict[str, str] = {}
    for line in raw.strip().splitlines():
        if ":" in line:
            key, _, value = line.partition(":")
            headers[key.strip().lower()] = value.strip()
    return headers


def format_json_response(data: Any, status: int = 200) -> Dict[str, Any]:
    return {
        "status": status,
        "body": json.dumps(data),
        "headers": {"Content-Type": "application/json"},
    }


# =============================================================================
# Main entry point
# =============================================================================


def main() -> int:
    cfg = AppConfig.from_env()
    logging.basicConfig(level=getattr(logging, cfg.log_level, logging.INFO))

    repo = UserRepository()
    cache = LRUCache(capacity=cfg.cache.capacity, default_ttl=cfg.cache.default_ttl)
    bus = EventBus()
    service = UserService(repo, cache, bus, cfg)

    # Seed a few users
    for i in range(10):
        service.create_user(
            username=f"user_{i}",
            email=f"user_{i}@example.com",
            role=UserRole.VIEWER if i % 3 != 0 else UserRole.ADMIN,
        )
        if i % 2 == 0:
            service.activate_user(i + 1)

    active_admins = service.list_active_admins()
    logger.info("Active admins: %d", len(active_admins))

    bf = BloomFilter(capacity=1000)
    for user in repo.list_all():
        bf.add(user.email)
    logger.info("Bloom filter FPR estimate: %.4f", bf.estimated_fpr)

    heap: MinHeap = MinHeap()
    for i, user in enumerate(repo.list_all()):
        heap.push(user, float(i))
    while heap:
        item, priority = heap.pop()
        logger.debug("Processed user %s (priority %.1f)", item.username, priority)

    values = [float(i) for i in range(100)]
    logger.info(
        "Stats: mean=%.2f std=%.2f p95=%.2f",
        mean(values),
        std_dev(values),
        percentile(values, 95),
    )

    graph = DirectedGraph()
    for node in ["a", "b", "c", "d", "e"]:
        graph.add_node(node)
    for src, dst in [("a", "b"), ("b", "c"), ("c", "d"), ("a", "d"), ("d", "e")]:
        graph.add_edge(src, dst)
    logger.info("Topological order: %s", graph.topological_sort())

    with timed("cache stats") as t:
        stats = cache.stats()
    logger.info("Cache stats (%.4fs): %s", t["elapsed"], stats)

    return 0


if __name__ == "__main__":
    sys.exit(main())
