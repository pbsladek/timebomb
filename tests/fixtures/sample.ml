(* sample.ml — fixture file for timebomb scanner tests.

   Annotation inventory (hardcoded dates, never relative to today):
     Expired        (2018-2021): 4
     Expiring-soon  (2025-06):   1
     Future / OK    (2088/2099): 2
*)

(* ---------------------------------------------------------------------------
   Config
   --------------------------------------------------------------------------- *)

type config = {
  host       : string;
  port       : int;
  db_url     : string;
  jwt_secret : string;
  env        : string;
}

let getenv_or key fallback =
  match Sys.getenv_opt key with Some v -> v | None -> fallback

let config_from_env () = {
  host       = getenv_or "HOST"       "0.0.0.0";
  port       = int_of_string (getenv_or "PORT" "3000");
  db_url     = getenv_or "DB_URL"     "postgres://localhost/app";
  jwt_secret = getenv_or "JWT_SECRET" "change-me";
  env        = getenv_or "APP_ENV"    "development";
}

let is_production cfg = cfg.env = "production"

(* ---------------------------------------------------------------------------
   Result helpers
   --------------------------------------------------------------------------- *)

(* REMOVEME[2018-03-20]: compatibility alias kept for downstream consumers — drop after 2.0 ships *)
let ok v  = Ok v
let err e = Error e

let unwrap = function
  | Ok v    -> v
  | Error e -> failwith ("unwrap on Error: " ^ e)

let unwrap_or default = function
  | Ok v   -> v
  | Error _ -> default

let map_result f = function
  | Ok v    -> Ok (f v)
  | Error e -> Error e

(* ---------------------------------------------------------------------------
   Validation
   --------------------------------------------------------------------------- *)

(* TODO[2020-09-01]: replace with a ppx-based schema validation library *)
type validation_failure = { field : string; message : string }

let validate_required field value =
  if String.length (String.trim value) = 0
  then Some { field; message = "is required" }
  else None

let validate_email field value =
  let re = Str.regexp {|^[^ \t@]+@[^ \t@]+\.[^ \t@]+$|} in
  if Str.string_match re value 0
  then None
  else Some { field; message = "must be a valid email address" }

let validate_min_length field value min =
  if String.length value >= min then None
  else Some { field; message = Printf.sprintf "must be at least %d characters" min }

let collect_failures checks =
  List.filter_map Fun.id checks

(* ---------------------------------------------------------------------------
   Cache (Hashtbl-backed)
   --------------------------------------------------------------------------- *)

(* HACK[2019-11-15]: Hashtbl with no eviction; wire up a proper TTL store before launch *)
type 'v cache_entry = { value : 'v; expires_at : float }

type ('k, 'v) cache = {
  tbl  : ('k, 'v cache_entry) Hashtbl.t;
  mu   : Mutex.t;
}

let cache_create () = { tbl = Hashtbl.create 64; mu = Mutex.create () }

let cache_get c key =
  Mutex.lock c.mu;
  let result =
    match Hashtbl.find_opt c.tbl key with
    | Some e when e.expires_at > Unix.gettimeofday () -> Some e.value
    | Some _ -> Hashtbl.remove c.tbl key; None
    | None   -> None
  in
  Mutex.unlock c.mu;
  result

let cache_set c key value ttl_sec =
  Mutex.lock c.mu;
  Hashtbl.replace c.tbl key { value; expires_at = Unix.gettimeofday () +. float_of_int ttl_sec };
  Mutex.unlock c.mu

let cache_del c key =
  Mutex.lock c.mu;
  Hashtbl.remove c.tbl key;
  Mutex.unlock c.mu

let cache_get_or_set c key ttl_sec f =
  match cache_get c key with
  | Some v -> v
  | None   ->
    let v = f () in
    cache_set c key v ttl_sec;
    v

(* ---------------------------------------------------------------------------
   Rate limiter
   --------------------------------------------------------------------------- *)

(* FIXME[2021-01-10]: no distributed coordination; add Redis token bucket for multi-node *)
type rate_result = {
  allowed     : bool;
  remaining   : int;
  retry_after : int;
}

type rate_slot = { mutable count : int; mutable reset_at : float }

type rate_limiter = {
  window_sec   : int;
  max_requests : int;
  store        : (string, rate_slot) Hashtbl.t;
  mu           : Mutex.t;
}

let rate_limiter_create window_sec max_requests =
  { window_sec; max_requests; store = Hashtbl.create 16; mu = Mutex.create () }

let rate_check rl key =
  Mutex.lock rl.mu;
  let now = Unix.gettimeofday () in
  let slot =
    match Hashtbl.find_opt rl.store key with
    | Some s when s.reset_at > now -> s
    | _ ->
      let s = { count = 0; reset_at = now +. float_of_int rl.window_sec } in
      Hashtbl.replace rl.store key s;
      s
  in
  slot.count <- slot.count + 1;
  let count   = slot.count in
  let allowed = count <= rl.max_requests in
  let after   = if allowed then 0 else int_of_float (slot.reset_at -. now) in
  Mutex.unlock rl.mu;
  { allowed; remaining = (if allowed then rl.max_requests - count else 0); retry_after = after }

(* ---------------------------------------------------------------------------
   Pagination
   --------------------------------------------------------------------------- *)

type 'a page = {
  items     : 'a list;
  total     : int;
  page_num  : int;
  page_size : int;
  has_next  : bool;
  has_prev  : bool;
}

let paginate items page_num page_size =
  let total  = List.length items in
  let offset = max 0 ((page_num - 1) * page_size) in
  let dropped = if offset >= total then [] else
    let rec drop n = function [] -> [] | _ :: t -> if n = 0 then t else drop (n-1) t in
    drop offset items
  in
  let rec take n = function
    | []     -> []
    | x :: t -> if n = 0 then [] else x :: take (n-1) t
  in
  let chunk = take page_size dropped in
  { items     = chunk;
    total;
    page_num;
    page_size;
    has_next  = offset + List.length chunk < total;
    has_prev  = page_num > 1 }

(* ---------------------------------------------------------------------------
   Feature flags
   --------------------------------------------------------------------------- *)

(* TODO[2099-07-01][platform]: replace Hashtbl flags with a remote LaunchDarkly client *)
type flag = { enabled : bool; rollout : int; allowlist : string list }

type flag_service = {
  flags : (string, flag) Hashtbl.t;
  mu    : Mutex.t;
}

let flag_service_create () = { flags = Hashtbl.create 8; mu = Mutex.create () }

let define_flag svc name enabled rollout allowlist =
  Mutex.lock svc.mu;
  Hashtbl.replace svc.flags name { enabled; rollout; allowlist };
  Mutex.unlock svc.mu

let flag_enabled svc name user_id =
  Mutex.lock svc.mu;
  let result =
    match Hashtbl.find_opt svc.flags name with
    | None   -> false
    | Some f ->
      f.enabled &&
      (f.rollout >= 100 ||
       (match user_id with Some u -> List.mem u f.allowlist | None -> false))
  in
  Mutex.unlock svc.mu;
  result

(* ---------------------------------------------------------------------------
   Utilities
   --------------------------------------------------------------------------- *)

let slugify text =
  let lower = String.lowercase_ascii text in
  let buf   = Buffer.create (String.length lower) in
  let last_dash = ref false in
  String.iter (fun c ->
    if (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') then begin
      Buffer.add_char buf c; last_dash := false
    end else if not !last_dash && Buffer.length buf > 0 then begin
      Buffer.add_char buf '-'; last_dash := true
    end
  ) lower;
  let s = Buffer.contents buf in
  let len = String.length s in
  if len > 0 && s.[len-1] = '-' then String.sub s 0 (len-1) else s

let mask_email email =
  match String.index_opt email '@' with
  | None    -> email
  | Some at ->
    let local  = String.sub email 0 at in
    let domain = String.sub email (at + 1) (String.length email - at - 1) in
    let vis    = String.sub local 0 (min 2 (String.length local)) in
    let stars  = String.make (max 1 (String.length local - 2)) '*' in
    vis ^ stars ^ "@" ^ domain

(* TODO[2025-06-10]: formatDuration does not handle sub-millisecond precision *)
let format_duration ms =
  if ms < 1000        then Printf.sprintf "%dms" ms
  else if ms < 60000  then Printf.sprintf "%.1fs" (float_of_int ms /. 1000.)
  else Printf.sprintf "%dm %ds" (ms / 60000) ((ms mod 60000) / 1000)

let format_bytes bytes =
  let units = [| "B"; "KB"; "MB"; "GB"; "TB" |] in
  let v = ref (float_of_int bytes) in
  let i = ref 0 in
  while !v >= 1024. && !i < 4 do v := !v /. 1024.; incr i done;
  Printf.sprintf "%.2f %s" !v units.(!i)

(* ---------------------------------------------------------------------------
   Metrics
   --------------------------------------------------------------------------- *)

(* TODO[2088-05-01][observability]: expose counters via a Prometheus HTTP handler *)
type counter = { name : string; mutable value : int; mu : Mutex.t }

let counter_create name = { name; value = 0; mu = Mutex.create () }
let counter_inc c by = Mutex.lock c.mu; c.value <- c.value + by; Mutex.unlock c.mu
let counter_read c = Mutex.lock c.mu; let v = c.value in Mutex.unlock c.mu; v
let counter_reset c = Mutex.lock c.mu; c.value <- 0; Mutex.unlock c.mu
