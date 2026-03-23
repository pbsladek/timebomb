%% sample.erl — fixture file for timebomb scanner tests.
%%
%% Annotation inventory (hardcoded dates, never relative to today):
%%   Expired        (2018–2021): 4
%%   Expiring-soon  (2025-06):   1
%%   Future / OK    (2088/2099): 2

-module(sample).
-export([
    make_config/0, production/1,
    ok/1, err/1, unwrap/1, unwrap_or/2,
    validate_required/2, validate_email/2, collect_failures/1,
    cache_new/0, cache_get/2, cache_set/4, cache_get_or_set/4,
    rate_limiter_new/2, rate_check/2,
    paginate/3,
    flag_service_new/0, define_flag/5, flag_enabled/3,
    slugify/1, mask_email/1, format_bytes/1, format_duration/1,
    retry/2
]).

%% ---------------------------------------------------------------------------
%% Config
%% ---------------------------------------------------------------------------

-record(config, {
    host       = "0.0.0.0",
    port       = 3000,
    db_url     = "postgres://localhost/app",
    jwt_secret = "change-me",
    env        = "development"
}).

make_config() ->
    #config{
        host       = os:getenv("HOST",       "0.0.0.0"),
        port       = list_to_integer(os:getenv("PORT", "3000")),
        db_url     = os:getenv("DB_URL",     "postgres://localhost/app"),
        jwt_secret = os:getenv("JWT_SECRET", "change-me"),
        env        = os:getenv("APP_ENV",    "development")
    }.

production(#config{env = Env}) -> Env =:= "production".

%% ---------------------------------------------------------------------------
%% Result helpers
%% ---------------------------------------------------------------------------

ok(Value)   -> {ok, Value}.
err(Reason) -> {error, Reason}.

unwrap({ok, Value})    -> Value;
unwrap({error, Reason}) -> error({unwrap_on_err, Reason}).

unwrap_or({ok, Value}, _Default)  -> Value;
unwrap_or({error, _}, Default)    -> Default.

%% ---------------------------------------------------------------------------
%% Validation
%% ---------------------------------------------------------------------------

%% TODO[2020-11-01]: replace with erval or a schema validation library
validate_required(Field, Value) when Value =:= undefined; Value =:= "" ->
    {error, #{field => Field, message => "is required"}};
validate_required(_Field, _Value) -> ok.

validate_email(Field, Value) ->
    Pattern = "^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$",
    case re:run(Value, Pattern) of
        {match, _} -> ok;
        nomatch    -> {error, #{field => Field, message => "must be a valid email address", value => Value}}
    end.

collect_failures(Checks) ->
    lists:filter(fun({error, _}) -> true; (_) -> false end, Checks).

%% ---------------------------------------------------------------------------
%% Cache (ETS-backed)
%% ---------------------------------------------------------------------------

%% HACK[2019-09-01]: process dictionary fallback; wire up Mnesia or Redis before launch
cache_new() ->
    ets:new(cache, [set, public, named_table]).

cache_get(Table, Key) ->
    Now = erlang:system_time(second),
    case ets:lookup(Table, Key) of
        [{Key, Value, ExpiresAt}] when ExpiresAt > Now -> {ok, Value};
        _ -> miss
    end.

cache_set(Table, Key, Value, TtlSec) ->
    ExpiresAt = erlang:system_time(second) + TtlSec,
    ets:insert(Table, {Key, Value, ExpiresAt}),
    ok.

cache_get_or_set(Table, Key, TtlSec, Fun) ->
    case cache_get(Table, Key) of
        {ok, Value} -> Value;
        miss ->
            Value = Fun(),
            cache_set(Table, Key, Value, TtlSec),
            Value
    end.

%% ---------------------------------------------------------------------------
%% Rate limiter
%% ---------------------------------------------------------------------------

%% FIXME[2021-07-15]: store is a plain map — not safe across nodes; add distributed coordination
-record(rate_limiter, {window_sec, max_requests, store = #{}}).

rate_limiter_new(WindowSec, MaxRequests) ->
    #rate_limiter{window_sec = WindowSec, max_requests = MaxRequests}.

rate_check(#rate_limiter{} = RL, Key) ->
    Now   = erlang:system_time(second),
    Store = RL#rate_limiter.store,
    Entry = maps:get(Key, Store, undefined),

    {Count, ResetAt} =
        case Entry of
            undefined                       -> {0, Now + RL#rate_limiter.window_sec};
            {C, R} when R > Now             -> {C, R};
            _                               -> {0, Now + RL#rate_limiter.window_sec}
        end,

    NewCount = Count + 1,
    NewStore = maps:put(Key, {NewCount, ResetAt}, Store),
    NewRL    = RL#rate_limiter{store = NewStore},
    Allowed  = NewCount =< RL#rate_limiter.max_requests,

    Result = #{
        allowed     => Allowed,
        remaining   => max(0, RL#rate_limiter.max_requests - NewCount),
        retry_after => case Allowed of true -> 0; false -> ResetAt - Now end
    },
    {Result, NewRL}.

%% ---------------------------------------------------------------------------
%% Pagination
%% ---------------------------------------------------------------------------

paginate(Items, PageNum, PageSize) ->
    Offset = max(0, (PageNum - 1) * PageSize),
    Chunk  = lists:sublist(lists:nthtail(min(Offset, length(Items)), Items), PageSize),
    Total  = length(Items),
    #{
        items     => Chunk,
        total     => Total,
        page_num  => PageNum,
        page_size => PageSize,
        has_next  => Offset + length(Chunk) < Total,
        has_prev  => PageNum > 1
    }.

%% ---------------------------------------------------------------------------
%% Feature flags
%% ---------------------------------------------------------------------------

%% TODO[2099-03-01][platform]: replace map-based flags with a remote LaunchDarkly client
flag_service_new() -> #{}.

define_flag(Flags, Name, Enabled, Rollout, Allowlist) ->
    maps:put(Name, #{enabled => Enabled, rollout => Rollout, allowlist => Allowlist}, Flags).

flag_enabled(Flags, Name, UserId) ->
    case maps:get(Name, Flags, undefined) of
        undefined -> false;
        Flag ->
            maps:get(enabled, Flag, false) andalso
            (lists:member(UserId, maps:get(allowlist, Flag, [])) orelse
             maps:get(rollout, Flag, 0) >= 100)
    end.

%% ---------------------------------------------------------------------------
%% Utilities
%% ---------------------------------------------------------------------------

slugify(Text) ->
    Lower   = string:lowercase(Text),
    NoSpec  = re:replace(Lower, "[^a-z0-9\\s-]", "", [global, {return, list}]),
    Dashed  = re:replace(NoSpec, "[\\s-]+", "-", [global, {return, list}]),
    string:trim(Dashed, both, "-").

mask_email(Email) ->
    case string:split(Email, "@") of
        [Local, Domain] ->
            Visible = string:slice(Local, 0, min(2, string:length(Local))),
            Stars   = lists:duplicate(max(1, string:length(Local) - 2), $*),
            Visible ++ Stars ++ "@" ++ Domain;
        _ ->
            Email
    end.

format_bytes(Bytes) ->
    Units = ["B", "KB", "MB", "GB", "TB"],
    format_bytes_loop(Bytes * 1.0, Units).

format_bytes_loop(V, [Unit]) ->
    io_lib:format("~.2f ~s", [V, Unit]);
format_bytes_loop(V, [Unit | Rest]) when V >= 1024 ->
    format_bytes_loop(V / 1024, Rest);
format_bytes_loop(V, [Unit | _]) ->
    io_lib:format("~.2f ~s", [V, Unit]).

%% FIXME[2025-06-10]: format_duration uses integer division and loses sub-second precision
format_duration(Ms) when Ms < 1000  -> io_lib:format("~Bms", [Ms]);
format_duration(Ms) when Ms < 60000 -> io_lib:format("~.1fs", [Ms / 1000.0]);
format_duration(Ms) ->
    io_lib:format("~Bm ~Bs", [Ms div 60000, (Ms rem 60000) div 1000]).

retry(_N, _Fun) when _N =< 0 -> {error, no_attempts};
retry(N, Fun) ->
    case Fun() of
        {ok, _} = Ok -> Ok;
        Err when N =:= 1 -> Err;
        _Err -> retry(N - 1, Fun)
    end.

%% ---------------------------------------------------------------------------
%% Metrics
%% ---------------------------------------------------------------------------

%% TODO[2088-12-01][observability]: expose metrics via Prometheus HTTP endpoint
-record(counter, {name, value = 0}).

counter_new(Name) -> #counter{name = Name}.
counter_inc(#counter{} = C, By) -> C#counter{value = C#counter.value + By}.
counter_read(#counter{value = V}) -> V.
counter_reset(#counter{} = C) -> C#counter{value = 0}.
