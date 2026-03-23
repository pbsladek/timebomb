// This file contains structured timebomb annotations for use in integration tests.
// Dates are chosen to be permanently in the past or far future so tests never
// depend on the current wall-clock date.
//
// The bulk of this file is realistic-looking Rust code that produces NO annotations,
// giving the scanner's pre-filter and regex a representative workload.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

// ── Types ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Active,
    Inactive,
    Pending,
    Archived,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Active => write!(f, "active"),
            Status::Inactive => write!(f, "inactive"),
            Status::Pending => write!(f, "pending"),
            Status::Archived => write!(f, "archived"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub email: String,
    pub status: Status,
    pub metadata: HashMap<String, String>,
}

impl User {
    pub fn new(id: u64, username: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            id,
            username: username.into(),
            email: email.into(),
            status: Status::Pending,
            metadata: HashMap::new(),
        }
    }

    pub fn activate(&mut self) {
        self.status = Status::Active;
    }

    pub fn deactivate(&mut self) {
        self.status = Status::Inactive;
    }

    pub fn archive(&mut self) {
        self.status = Status::Archived;
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, Status::Active)
    }

    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }
}

impl fmt::Display for User {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "User({}, {}, {})", self.id, self.username, self.status)
    }
}

// ── Repository ────────────────────────────────────────────────────────────────

pub struct UserRepository {
    users: HashMap<u64, User>,
    next_id: u64,
}

impl UserRepository {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn create(&mut self, username: impl Into<String>, email: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let user = User::new(id, username, email);
        self.users.insert(id, user);
        id
    }

    pub fn get(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut User> {
        self.users.get_mut(&id)
    }

    pub fn delete(&mut self, id: u64) -> bool {
        self.users.remove(&id).is_some()
    }

    pub fn count(&self) -> usize {
        self.users.len()
    }

    pub fn active_users(&self) -> Vec<&User> {
        self.users.values().filter(|u| u.is_active()).collect()
    }

    pub fn find_by_username(&self, username: &str) -> Option<&User> {
        self.users.values().find(|u| u.username == username)
    }

    pub fn find_by_email(&self, email: &str) -> Option<&User> {
        self.users.values().find(|u| u.email == email)
    }
}

impl Default for UserRepository {
    fn default() -> Self {
        Self::new()
    }
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub max_connections: u32,
    pub timeout_seconds: u64,
    pub debug_mode: bool,
    pub allowed_origins: Vec<String>,
    pub feature_flags: HashMap<String, bool>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/app".into()),
            max_connections: std::env::var("MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            timeout_seconds: std::env::var("TIMEOUT_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            debug_mode: std::env::var("DEBUG").map(|v| v == "1").unwrap_or(false),
            allowed_origins: std::env::var("ALLOWED_ORIGINS")
                .map(|v| v.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            feature_flags: HashMap::new(),
        })
    }

    pub fn is_feature_enabled(&self, name: &str) -> bool {
        self.feature_flags.get(name).copied().unwrap_or(false)
    }

    pub fn set_feature(&mut self, name: impl Into<String>, enabled: bool) {
        self.feature_flags.insert(name.into(), enabled);
    }
}

// ── Error types ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AppError {
    NotFound { id: u64 },
    Unauthorized { user_id: u64 },
    ValidationError(String),
    DatabaseError(String),
    ConfigError(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NotFound { id } => write!(f, "resource not found: {}", id),
            AppError::Unauthorized { user_id } => write!(f, "unauthorized: {}", user_id),
            AppError::ValidationError(msg) => write!(f, "validation error: {}", msg),
            AppError::DatabaseError(msg) => write!(f, "database error: {}", msg),
            AppError::ConfigError(msg) => write!(f, "config error: {}", msg),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ── Service layer ─────────────────────────────────────────────────────────────

pub struct UserService {
    repo: UserRepository,
    config: AppConfig,
}

impl UserService {
    pub fn new(config: AppConfig) -> Self {
        Self {
            repo: UserRepository::new(),
            config,
        }
    }

    pub fn register(&mut self, username: &str, email: &str) -> AppResult<u64> {
        if username.is_empty() {
            return Err(AppError::ValidationError("username cannot be empty".into()));
        }
        if !email.contains('@') {
            return Err(AppError::ValidationError("invalid email address".into()));
        }
        if self.repo.find_by_username(username).is_some() {
            return Err(AppError::ValidationError("username already taken".into()));
        }
        if self.repo.find_by_email(email).is_some() {
            return Err(AppError::ValidationError("email already registered".into()));
        }
        Ok(self.repo.create(username, email))
    }

    pub fn activate_user(&mut self, id: u64) -> AppResult<()> {
        let user = self
            .repo
            .get_mut(id)
            .ok_or(AppError::NotFound { id })?;
        user.activate();
        Ok(())
    }

    pub fn deactivate_user(&mut self, id: u64) -> AppResult<()> {
        let user = self
            .repo
            .get_mut(id)
            .ok_or(AppError::NotFound { id })?;
        user.deactivate();
        Ok(())
    }

    pub fn get_user(&self, id: u64) -> AppResult<&User> {
        self.repo.get(id).ok_or(AppError::NotFound { id })
    }

    pub fn list_active(&self) -> Vec<&User> {
        self.repo.active_users()
    }

    pub fn user_count(&self) -> usize {
        self.repo.count()
    }
}

// ── File utilities ────────────────────────────────────────────────────────────

pub fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    Ok(content.lines().map(str::to_string).collect())
}

pub fn write_lines(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let content = lines.join("\n");
    std::fs::write(path, content)
}

pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

pub fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    target.strip_prefix(base).ok().map(Path::to_path_buf)
}

pub fn file_extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|e| e.to_str())
}

pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

// ── String utilities ──────────────────────────────────────────────────────────

pub fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        None => s,
        Some((idx, _)) => &s[..idx],
    }
}

pub fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

// ── Math utilities ────────────────────────────────────────────────────────────

pub fn clamp<T: PartialOrd>(val: T, min: T, max: T) -> T {
    if val < min {
        min
    } else if val > max {
        max
    } else {
        val
    }
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn median(values: &mut Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

pub fn std_dev(values: &[f64]) -> Option<f64> {
    let m = mean(values)?;
    let variance = values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / values.len() as f64;
    Some(variance.sqrt())
}

// ── Cache ─────────────────────────────────────────────────────────────────────

pub struct Cache<K, V> {
    store: HashMap<K, V>,
    max_size: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> Cache<K, V> {
    pub fn new(max_size: usize) -> Self {
        Self {
            store: HashMap::new(),
            max_size,
        }
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.store.get(key)
    }

    pub fn insert(&mut self, key: K, value: V) -> bool {
        if self.store.len() >= self.max_size && !self.store.contains_key(&key) {
            return false;
        }
        self.store.insert(key, value);
        true
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.store.remove(key)
    }

    pub fn clear(&mut self) {
        self.store.clear();
    }

    pub fn len(&self) -> usize {
        self.store.len()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

pub struct Pipeline<T> {
    steps: Vec<Box<dyn Fn(T) -> Option<T>>>,
}

impl<T: 'static> Pipeline<T> {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn add_step<F: Fn(T) -> Option<T> + 'static>(mut self, step: F) -> Self {
        self.steps.push(Box::new(step));
        self
    }

    pub fn run(&self, input: T) -> Option<T> {
        let mut current = Some(input);
        for step in &self.steps {
            current = current.and_then(|val| step(val));
        }
        current
    }
}

impl<T: 'static> Default for Pipeline<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    // ── Expired annotations (date in the past) ────────────────────────────────

    // TODO[2020-01-01]: remove legacy authentication module
    let _legacy = true;

    // FIXME[2019-06-15]: upstream bug workaround, revert after upgrade
    let _workaround = 42;

    // HACK[2018-03-10]: temporary patch for prod incident #4471
    let _patch = "hack";

    // TEMP[2020-12-31]: remove after Q4 migration completes
    let _temp = vec![1, 2, 3];

    // REMOVEME[2021-05-01]: feature flag for experiment A/B, experiment ended
    let _flag = false;

    // TODO[2020-01-01][alice]: remove after alice's team finishes refactor
    let _owned = "owned";

    // ── Expiring-soon annotations (within a typical warn window) ──────────────
    // These use dates far enough in the future to be "ok" but tests that need
    // "expiring soon" should inject a `today` close to these dates.

    // TODO[2025-06-10]: rotate API keys before deadline
    let _keys = "rotate";

    // FIXME[2025-06-08]: temporary disable of rate limiting, re-enable after deploy
    let _rate_limit = true;

    // ── Future annotations (far future — always OK) ───────────────────────────

    // TODO[2099-01-01]: revisit this algorithm when hardware improves
    let _algo = 0;

    // FIXME[2099-12-31]: long-term tech debt, tracked in issue #9999
    let _debt = "future";

    // HACK[2088-07-04]: can be removed once the platform team ships new API
    let _platform = ();

    // TODO[2099-01-01][bob]: bob will handle this in the next major version
    let _bobs_work = "pending";

    // ── Non-matching annotations (should be ignored by scanner) ──────────────

    // TODO: this is a plain TODO with no date — must NOT be matched
    // FIXME: another plain one
    // NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
    // TODO [2020-01-01]: space between tag and bracket — must NOT be matched

    // ── Application setup ─────────────────────────────────────────────────────

    let config = AppConfig {
        database_url: "postgres://localhost/app".into(),
        max_connections: 20,
        timeout_seconds: 30,
        debug_mode: false,
        allowed_origins: vec!["https://example.com".into()],
        feature_flags: HashMap::new(),
    };

    let mut service = UserService::new(config);

    let id1 = service.register("alice", "alice@example.com").unwrap();
    let id2 = service.register("bob", "bob@example.com").unwrap();
    let id3 = service.register("carol", "carol@example.com").unwrap();

    service.activate_user(id1).unwrap();
    service.activate_user(id2).unwrap();

    println!("active users: {}", service.list_active().len());
    println!("total users: {}", service.user_count());

    let user = service.get_user(id3).unwrap();
    println!("user: {}", user);

    // String utilities
    println!("{}", slugify("Hello World! This is a test."));
    println!("{}", capitalize("hello world"));
    println!("words: {}", word_count("the quick brown fox jumps over the lazy dog"));
    println!("truncated: {}", truncate("this is a long string", 10));

    // Math utilities
    let mut values = vec![3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
    println!("median: {:?}", median(&mut values));
    println!("mean: {:?}", mean(&values));
    println!("std_dev: {:?}", std_dev(&values));
    println!("lerp: {}", lerp(0.0, 10.0, 0.5));
    println!("clamp: {}", clamp(15, 0, 10));

    // Cache
    let mut cache: Cache<String, u64> = Cache::new(100);
    cache.insert("key1".into(), 42);
    cache.insert("key2".into(), 99);
    println!("cache size: {}", cache.len());
    println!("cache[key1]: {:?}", cache.get(&"key1".to_string()));

    // Pipeline
    let pipeline: Pipeline<i32> = Pipeline::new()
        .add_step(|x| if x > 0 { Some(x * 2) } else { None })
        .add_step(|x| Some(x + 1))
        .add_step(|x| if x < 1000 { Some(x) } else { None });

    println!("pipeline result: {:?}", pipeline.run(5));
    println!("pipeline rejected: {:?}", pipeline.run(-1));

    // File utilities (paths only, no actual I/O in this fixture)
    let base = Path::new("/home/user/project");
    let target = Path::new("/home/user/project/src/main.rs");
    println!("relative: {:?}", relative_path(base, target));
    println!("extension: {:?}", file_extension(target));
    println!("hidden: {}", is_hidden(Path::new(".gitignore")));

    println!("fixture file");
}

// ── Additional module-level helpers ───────────────────────────────────────────

/// Batch process a collection, returning results and errors separately.
pub fn partition_results<T, E>(
    results: impl IntoIterator<Item = Result<T, E>>,
) -> (Vec<T>, Vec<E>) {
    let mut ok = Vec::new();
    let mut err = Vec::new();
    for r in results {
        match r {
            Ok(v) => ok.push(v),
            Err(e) => err.push(e),
        }
    }
    (ok, err)
}

/// Chunk an iterator into fixed-size batches.
pub fn chunks<T>(items: Vec<T>, size: usize) -> Vec<Vec<T>> {
    if size == 0 {
        return vec![];
    }
    items
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(size)
        .map(|c| c.to_vec())
        .collect()
}

/// Retry a fallible operation up to `attempts` times.
pub fn retry<T, E, F: Fn() -> Result<T, E>>(attempts: usize, f: F) -> Result<T, E> {
    let mut last_err = None;
    for _ in 0..attempts {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap())
}

/// Simple event bus.
pub struct EventBus<E> {
    handlers: Vec<Box<dyn Fn(&E)>>,
}

impl<E: 'static> EventBus<E> {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    pub fn subscribe<F: Fn(&E) + 'static>(&mut self, handler: F) {
        self.handlers.push(Box::new(handler));
    }

    pub fn publish(&self, event: &E) {
        for handler in &self.handlers {
            handler(event);
        }
    }
}

impl<E: 'static> Default for EventBus<E> {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limiter using token bucket algorithm (simplified).
pub struct RateLimiter {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
}

impl RateLimiter {
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
        }
    }

    pub fn try_consume(&mut self, tokens: f64) -> bool {
        if self.tokens >= tokens {
            self.tokens -= tokens;
            true
        } else {
            false
        }
    }

    pub fn refill(&mut self, elapsed_seconds: f64) {
        self.tokens = (self.tokens + self.refill_rate * elapsed_seconds).min(self.max_tokens);
    }
}

/// Bloom filter stub (for demonstration purposes).
pub struct BloomFilter {
    bits: Vec<bool>,
    hash_count: usize,
}

impl BloomFilter {
    pub fn new(size: usize, hash_count: usize) -> Self {
        Self {
            bits: vec![false; size],
            hash_count,
        }
    }

    fn hash(&self, item: &str, seed: usize) -> usize {
        let mut h: usize = seed.wrapping_mul(0x517cc1b727220a95);
        for b in item.bytes() {
            h = h.wrapping_mul(31).wrapping_add(b as usize);
        }
        h % self.bits.len()
    }

    pub fn insert(&mut self, item: &str) {
        for i in 0..self.hash_count {
            let idx = self.hash(item, i);
            self.bits[idx] = true;
        }
    }

    pub fn might_contain(&self, item: &str) -> bool {
        (0..self.hash_count).all(|i| self.bits[self.hash(item, i)])
    }
}

/// Priority queue using a binary heap (simplified min-heap).
pub struct MinHeap<T: PartialOrd> {
    data: Vec<T>,
}

impl<T: PartialOrd> MinHeap<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn push(&mut self, val: T) {
        self.data.push(val);
        self.sift_up(self.data.len() - 1);
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.data.is_empty() {
            return None;
        }
        let n = self.data.len() - 1;
        self.data.swap(0, n);
        let val = self.data.pop();
        if !self.data.is_empty() {
            self.sift_down(0);
        }
        val
    }

    pub fn peek(&self) -> Option<&T> {
        self.data.first()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn sift_up(&mut self, mut i: usize) {
        while i > 0 {
            let parent = (i - 1) / 2;
            if self.data[i] < self.data[parent] {
                self.data.swap(i, parent);
                i = parent;
            } else {
                break;
            }
        }
    }

    fn sift_down(&mut self, mut i: usize) {
        let n = self.data.len();
        loop {
            let left = 2 * i + 1;
            let right = 2 * i + 2;
            let mut smallest = i;
            if left < n && self.data[left] < self.data[smallest] {
                smallest = left;
            }
            if right < n && self.data[right] < self.data[smallest] {
                smallest = right;
            }
            if smallest == i {
                break;
            }
            self.data.swap(i, smallest);
            i = smallest;
        }
    }
}

impl<T: PartialOrd> Default for MinHeap<T> {
    fn default() -> Self {
        Self::new()
    }
}
