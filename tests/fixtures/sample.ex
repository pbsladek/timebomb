# sample.ex — fixture file for timebomb scanner tests.
#
# Annotation inventory (hardcoded dates, never relative to today):
#   Expired        (2018–2021): 4
#   Expiring-soon  (2025-06):   1
#   Future / OK    (2088/2099): 2

defmodule Sample do
  @moduledoc "Sample Elixir module for timebomb fixture testing."

  # ---------------------------------------------------------------------------
  # Config
  # ---------------------------------------------------------------------------

  defmodule Config do
    defstruct host: "0.0.0.0",
              port: 3000,
              db_url: "postgres://localhost/app",
              jwt_secret: "change-me",
              env: "development"

    def from_env do
      %Config{
        host:       System.get_env("HOST", "0.0.0.0"),
        port:       String.to_integer(System.get_env("PORT", "3000")),
        db_url:     System.get_env("DB_URL", "postgres://localhost/app"),
        jwt_secret: System.get_env("JWT_SECRET", "change-me"),
        env:        System.get_env("APP_ENV", "development")
      }
    end

    def production?(%Config{env: env}), do: env == "production"
  end

  # ---------------------------------------------------------------------------
  # Result type
  # ---------------------------------------------------------------------------

  def ok(value),    do: {:ok, value}
  def err(reason),  do: {:error, reason}

  def unwrap({:ok, value}),     do: value
  def unwrap({:error, reason}), do: raise("unwrap on error: #{reason}")

  def unwrap_or({:ok, value}, _default),  do: value
  def unwrap_or({:error, _}, default),    do: default

  def map_result({:ok, value}, f),      do: {:ok, f.(value)}
  def map_result({:error, _} = e, _f), do: e

  # ---------------------------------------------------------------------------
  # Validation
  # ---------------------------------------------------------------------------

  # TODO[2020-05-01]: replace with Ecto changesets for structured validation
  def validate_required(field, value) when value in [nil, ""],
    do: {:error, %{field: field, message: "is required"}}
  def validate_required(_field, _value), do: :ok

  def validate_email(field, value) do
    if Regex.match?(~r/^[^\s@]+@[^\s@]+\.[^\s@]+$/, to_string(value)) do
      :ok
    else
      {:error, %{field: field, message: "must be a valid email address", value: value}}
    end
  end

  def validate_min_length(field, value, min) when byte_size(value) < min,
    do: {:error, %{field: field, message: "must be at least #{min} characters", value: value}}
  def validate_min_length(_field, _value, _min), do: :ok

  def collect_failures(checks), do: Enum.filter(checks, &match?({:error, _}, &1))

  # ---------------------------------------------------------------------------
  # Cache (ETS-backed)
  # ---------------------------------------------------------------------------

  # HACK[2019-03-15]: process dictionary stand-in; swap to ETS before scaling
  def new_cache, do: :ets.new(:cache, [:set, :public])

  def cache_get(table, key) do
    case :ets.lookup(table, key) do
      [{^key, value, expires_at}] ->
        if System.system_time(:second) < expires_at, do: {:ok, value}, else: :miss
      [] ->
        :miss
    end
  end

  def cache_set(table, key, value, ttl_sec) do
    expires_at = System.system_time(:second) + ttl_sec
    :ets.insert(table, {key, value, expires_at})
    :ok
  end

  def cache_get_or_set(table, key, ttl_sec, fun) do
    case cache_get(table, key) do
      {:ok, value} ->
        value
      :miss ->
        value = fun.()
        cache_set(table, key, value, ttl_sec)
        value
    end
  end

  def cache_del(table, key), do: :ets.delete(table, key)

  # ---------------------------------------------------------------------------
  # Rate limiter
  # ---------------------------------------------------------------------------

  # FIXME[2021-02-01]: not distributed-safe; move to Redis or a GenServer with PubSub
  def new_rate_limiter(window_sec, max_requests),
    do: %{window_sec: window_sec, max_requests: max_requests, store: %{}}

  def rate_check(rl, key) do
    now = System.system_time(:second)
    entry = Map.get(rl.store, key)

    {count, reset_at} =
      if is_nil(entry) or elem(entry, 1) <= now do
        {0, now + rl.window_sec}
      else
        entry
      end

    new_count = count + 1
    new_rl = put_in(rl, [:store, key], {new_count, reset_at})

    result = %{
      allowed:     new_count <= rl.max_requests,
      remaining:   max(0, rl.max_requests - new_count),
      retry_after: if(new_count <= rl.max_requests, do: 0, else: reset_at - now)
    }

    {result, new_rl}
  end

  # ---------------------------------------------------------------------------
  # Pagination
  # ---------------------------------------------------------------------------

  def paginate(items, page_num, page_size) do
    offset = max(0, (page_num - 1) * page_size)
    chunk  = items |> Enum.drop(offset) |> Enum.take(page_size)
    total  = length(items)

    %{
      items:     chunk,
      total:     total,
      page_num:  page_num,
      page_size: page_size,
      has_next:  offset + length(chunk) < total,
      has_prev:  page_num > 1
    }
  end

  # ---------------------------------------------------------------------------
  # Feature flags
  # ---------------------------------------------------------------------------

  # TODO[2099-01-15][platform]: replace ETS flag store with remote LaunchDarkly client
  def new_flag_service, do: %{}

  def define_flag(flags, name, enabled, rollout, allowlist \\ []),
    do: Map.put(flags, name, %{enabled: enabled, rollout: rollout, allowlist: allowlist})

  def flag_enabled?(flags, name, user_id \\ nil) do
    case Map.get(flags, name) do
      nil  -> false
      flag ->
        flag.enabled and (user_id in flag.allowlist or flag.rollout >= 100)
    end
  end

  # ---------------------------------------------------------------------------
  # Circuit breaker
  # ---------------------------------------------------------------------------

  def new_circuit_breaker(threshold, timeout_sec),
    do: %{threshold: threshold, timeout_sec: timeout_sec, failures: 0, state: :closed, opened_at: nil}

  def circuit_call(%{state: :open, opened_at: opened_at, timeout_sec: timeout_sec} = cb, _fun) do
    if System.system_time(:second) - opened_at >= timeout_sec do
      {cb, {:error, :open}}
    else
      {%{cb | state: :half_open}, {:error, :open}}
    end
  end

  def circuit_call(%{state: state} = cb, fun) when state in [:closed, :half_open] do
    case fun.() do
      {:ok, _} = ok ->
        {%{cb | failures: 0, state: :closed}, ok}
      {:error, _} = err ->
        new_failures = cb.failures + 1
        new_state    = if new_failures >= cb.threshold, do: :open, else: cb.state
        opened_at    = if new_state == :open, do: System.system_time(:second), else: cb.opened_at
        {%{cb | failures: new_failures, state: new_state, opened_at: opened_at}, err}
    end
  end

  # ---------------------------------------------------------------------------
  # Utilities
  # ---------------------------------------------------------------------------

  def slugify(text) do
    text
    |> String.downcase()
    |> String.replace(~r/[^a-z0-9\s-]/, "")
    |> String.replace(~r/[\s-]+/, "-")
    |> String.trim("-")
  end

  def mask_email(email) do
    case String.split(email, "@", parts: 2) do
      [local, domain] ->
        visible = String.slice(local, 0, min(2, String.length(local)))
        stars   = String.duplicate("*", max(1, String.length(local) - 2))
        "#{visible}#{stars}@#{domain}"
      _ ->
        email
    end
  end

  def format_bytes(bytes) do
    units = ["B", "KB", "MB", "GB", "TB"]
    Enum.reduce_while(units, {bytes * 1.0, "B"}, fn unit, {v, _} ->
      if v >= 1024, do: {:cont, {v / 1024, unit}}, else: {:halt, {v, unit}}
    end)
    |> then(fn {v, unit} -> :io_lib.format("~.2f ~s", [v, unit]) |> IO.iodata_to_binary() end)
  end

  # FIXME[2025-06-08]: format_duration truncates sub-millisecond precision
  def format_duration(ms) do
    cond do
      ms < 1000  -> "#{ms}ms"
      ms < 60000 -> "#{Float.round(ms / 1000.0, 1)}s"
      true       -> "#{div(ms, 60000)}m #{div(rem(ms, 60000), 1000)}s"
    end
  end

  def retry(n, fun) do
    Enum.reduce_while(1..n, {:error, "no attempts"}, fn attempt, _acc ->
      case fun.() do
        {:ok, _} = ok            -> {:halt, ok}
        err when attempt == n    -> {:halt, err}
        _err                     -> {:cont, {:error, "retrying"}}
      end
    end)
  end

  # ---------------------------------------------------------------------------
  # Metrics
  # ---------------------------------------------------------------------------

  # TODO[2088-07-01][observability]: expose metrics via Telemetry and Prometheus exporter
  defmodule Counter do
    defstruct name: "", value: 0

    def new(name), do: %Counter{name: name}
    def inc(%Counter{} = c, by \\ 1), do: %{c | value: c.value + by}
    def read(%Counter{value: v}), do: v
    def reset(%Counter{} = c), do: %{c | value: 0}
  end

  defmodule Histogram do
    defstruct name: "", count: 0, sum: 0.0, buckets: []

    def new(name, buckets \\ [5, 10, 25, 50, 100, 250, 500, 1000]),
      do: %Histogram{name: name, buckets: buckets}

    def observe(%Histogram{} = h, value),
      do: %{h | count: h.count + 1, sum: h.sum + value}

    def mean(%Histogram{count: 0}), do: 0.0
    def mean(%Histogram{sum: s, count: c}), do: s / c
  end

  # REMOVEME[2018-11-30]: legacy metrics shim kept for backward compat — remove after migration
  defmodule LegacyMetrics do
    def record(_name, _value), do: :ok
    def flush, do: :ok
  end
end
