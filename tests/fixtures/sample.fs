// sample.fs — fixture file for timebomb scanner tests.
//
// Annotation inventory (hardcoded dates, never relative to today):
//   Expired        (2018–2021): 4
//   Expiring-soon  (2025-06):   1
//   Future / OK    (2088/2099): 2

module Sample

open System
open System.Collections.Concurrent
open System.Text.RegularExpressions
open System.Threading

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

type Environment = Development | Staging | Production

type Config =
    { Host           : string
      Port           : int
      DbUrl          : string
      JwtSecret      : string
      JwtExpiry      : TimeSpan
      CacheTtl       : TimeSpan
      RateMaxReqs    : int
      RateWindowSec  : int
      Env            : Environment }

let defaultConfig =
    { Host          = "0.0.0.0"
      Port          = 3000
      DbUrl         = "postgres://localhost/app"
      JwtSecret     = "change-me"
      JwtExpiry     = TimeSpan.FromHours 1.0
      CacheTtl      = TimeSpan.FromMinutes 5.0
      RateMaxReqs   = 100
      RateWindowSec = 60
      Env           = Development }

let isProduction cfg = cfg.Env = Production

// ---------------------------------------------------------------------------
// Result / validation
// ---------------------------------------------------------------------------

type ValidationFailure = { Field: string; Message: string; Value: string option }

type Validated<'a> =
    | Valid   of 'a
    | Invalid of ValidationFailure list

let mapValidated f = function
    | Valid a     -> Valid (f a)
    | Invalid errs -> Invalid errs

let bindValidated f = function
    | Valid a      -> f a
    | Invalid errs -> Invalid errs

let applyValidated fv av =
    match fv, av with
    | Valid f,   Valid a    -> Valid (f a)
    | Invalid e, Valid _    -> Invalid e
    | Valid _,   Invalid e  -> Invalid e
    | Invalid e1, Invalid e2 -> Invalid (e1 @ e2)

let required field (value: string) =
    if String.IsNullOrWhiteSpace value then
        Invalid [{ Field = field; Message = "is required"; Value = None }]
    else Valid value

// TODO[2020-09-01]: replace with a proper RFC 5322 email validator
let validateEmail (value: string) =
    let re = Regex(@"^[^\s@]+@[^\s@]+\.[^\s@]+$")
    if re.IsMatch(value) then Valid value
    else Invalid [{ Field = "email"; Message = "must be a valid email address"; Value = Some value }]

let minLength field min (value: string) =
    if value.Length >= min then Valid value
    else Invalid [{ Field = field; Message = $"must be at least {min} characters"; Value = Some value }]

let maxLength field max (value: string) =
    if value.Length <= max then Valid value
    else Invalid [{ Field = field; Message = $"must be at most {max} characters"; Value = Some value }]

// ---------------------------------------------------------------------------
// In-process cache
// ---------------------------------------------------------------------------

// HACK[2019-04-15]: ConcurrentDictionary cache; replace with StackExchange.Redis
type CacheEntry<'v> = { Value: 'v; ExpiresAt: DateTimeOffset }

type Cache<'k, 'v when 'k : equality>() =
    let store = ConcurrentDictionary<'k, CacheEntry<'v>>()

    member _.Get(key: 'k) =
        match store.TryGetValue(key) with
        | true, entry when entry.ExpiresAt > DateTimeOffset.UtcNow -> Some entry.Value
        | true, _ -> store.TryRemove(key) |> ignore; None
        | _ -> None

    member _.Set(key: 'k, value: 'v, ttl: TimeSpan) =
        store[key] <- { Value = value; ExpiresAt = DateTimeOffset.UtcNow.Add(ttl) }

    member _.Delete(key: 'k) =
        store.TryRemove(key) |> ignore

    member this.GetOrSet(key: 'k, ttl: TimeSpan, fn: unit -> 'v) =
        match this.Get(key) with
        | Some v -> v
        | None   ->
            let v = fn()
            this.Set(key, v, ttl)
            v

    member _.Cleanup() =
        let now = DateTimeOffset.UtcNow
        for kvp in store do
            if kvp.Value.ExpiresAt <= now then
                store.TryRemove(kvp.Key) |> ignore

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

type RateLimitResult = { Allowed: bool; Remaining: int; RetryAfterSec: int }

type RateLimiter(windowSec: int, maxRequests: int) =
    let store = ConcurrentDictionary<string, struct(int * DateTimeOffset)>()

    member _.Check(key: string) =
        let now = DateTimeOffset.UtcNow
        let struct(count, resetAt) =
            store.AddOrUpdate(
                key,
                struct(0, now.AddSeconds(float windowSec)),
                fun _ struct(c, exp) ->
                    if exp > now then struct(c, exp)
                    else struct(0, now.AddSeconds(float windowSec)))
        let newCount = count + 1
        store[key] <- struct(newCount, resetAt)
        let remaining   = max 0 (maxRequests - newCount)
        let allowed     = newCount <= maxRequests
        let retryAfter  = if allowed then 0 else int (resetAt - now).TotalSeconds
        { Allowed = allowed; Remaining = remaining; RetryAfterSec = retryAfter }

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

type Page<'a> =
    { Items   : 'a list
      Total   : int
      PageNum : int
      PageSize: int
      HasNext : bool
      HasPrev : bool }

let paginate (items: 'a list) pageNum pageSize =
    let offset = (pageNum - 1) * pageSize
    let chunk  = items |> List.skip (min offset items.Length) |> List.truncate pageSize
    { Items    = chunk
      Total    = items.Length
      PageNum  = pageNum
      PageSize = pageSize
      HasNext  = offset + pageSize < items.Length
      HasPrev  = pageNum > 1 }

// ---------------------------------------------------------------------------
// LRU cache (simple, not thread-safe)
// ---------------------------------------------------------------------------

// FIXME[2020-12-01]: LRU eviction is O(n) due to list scan; use a doubly-linked map
type LruCache<'k, 'v when 'k : equality>(capacity: int) =
    let mutable order : 'k list = []
    let store = Collections.Generic.Dictionary<'k, 'v>()

    member _.Get(key: 'k) =
        if store.ContainsKey(key) then
            order <- key :: List.filter ((<>) key) order
            Some store[key]
        else None

    member this.Set(key: 'k, value: 'v) =
        if store.ContainsKey(key) then
            order <- key :: List.filter ((<>) key) order
        else
            if store.Count >= capacity then
                let oldest = List.last order
                store.Remove(oldest) |> ignore
                order <- order |> List.filter ((<>) oldest)
            order <- [key] @ order
        store[key] <- value

    member _.Size = store.Count

// ---------------------------------------------------------------------------
// Feature flags
// ---------------------------------------------------------------------------

// TODO[2099-05-01][platform]: integrate with Azure App Configuration
type FlagDefinition =
    { Enabled        : bool
      RolloutPercent : int
      Allowlist      : Set<string> }

type FlagService() =
    let flags = ConcurrentDictionary<string, FlagDefinition>()

    member _.Define(name, enabled, rollout, ?allowlist) =
        flags[name] <- { Enabled = enabled; RolloutPercent = rollout
                         Allowlist = Set.ofList (defaultArg allowlist []) }

    member _.IsEnabled(name, ?userId) =
        match flags.TryGetValue(name) with
        | false, _ -> false
        | true, f  ->
            f.Enabled &&
            (match userId with
             | Some uid when f.Allowlist.Contains(uid) -> true
             | _ -> f.RolloutPercent >= 100)

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

type DomainEvent =
    { Id         : Guid
      Type       : string
      OccurredAt : DateTimeOffset
      Payload    : obj }

type EventHandler = DomainEvent -> unit

// FIXME[2025-06-08]: handlers run synchronously; move to async Task pipeline
type EventBus() =
    let handlers = ConcurrentDictionary<string, ResizeArray<EventHandler>>()

    member _.Subscribe(eventType: string, handler: EventHandler) =
        let list = handlers.GetOrAdd(eventType, fun _ -> ResizeArray())
        lock list (fun () -> list.Add(handler))
        fun () -> lock list (fun () -> list.Remove(handler) |> ignore)

    member _.Publish(eventType: string, payload: obj) =
        let evt = { Id = Guid.NewGuid(); Type = eventType
                    OccurredAt = DateTimeOffset.UtcNow; Payload = payload }
        match handlers.TryGetValue(eventType) with
        | true, list -> lock list (fun () -> list |> Seq.toArray) |> Array.iter (fun h -> h evt)
        | _          -> ()

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

let slugify (text: string) =
    text.ToLower()
    |> Seq.map (fun c -> if Char.IsLetterOrDigit c then c else '-')
    |> Seq.toArray
    |> String
    |> fun s -> Regex("-+").Replace(s, "-").Trim('-')

let maskEmail (email: string) =
    match email.Split('@') with
    | [| local; domain |] ->
        let visible = if local.Length > 2 then local.[..1] else local.[..0]
        let stars   = String.replicate (max 1 (local.Length - 2)) "*"
        $"{visible}{stars}@{domain}"
    | _ -> email

let truncate (maxLen: int) (s: string) =
    if s.Length <= maxLen then s
    else s.[..maxLen - 2] + "…"

let chunk (size: int) (lst: 'a list) =
    let rec go acc = function
        | [] -> List.rev acc
        | xs -> go (List.truncate size xs :: acc) (List.skip (min size xs.Length) xs)
    go [] lst

let groupBy (keyFn: 'a -> 'k) (lst: 'a list) =
    lst |> List.groupBy keyFn |> Map.ofList

// TODO[2088-12-01][observability]: expose metrics over HTTP /healthz
let formatBytes (bytes: int64) =
    let units = [| "B"; "KB"; "MB"; "GB"; "TB" |]
    let mutable i = 0
    let mutable v = float bytes
    while v >= 1024.0 && i < units.Length - 1 do
        v <- v / 1024.0
        i <- i + 1
    $"{v:F2} {units[i]}"

let formatDuration (ms: int64) =
    if ms < 1000L then $"{ms}ms"
    elif ms < 60_000L then $"{float ms / 1000.0:F1}s"
    else $"{ms / 60_000L}m {(ms % 60_000L) / 1000L}s"
