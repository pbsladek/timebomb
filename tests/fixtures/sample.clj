;; sample.clj — fixture file for timebomb scanner tests.
;;
;; Annotation inventory (hardcoded dates, never relative to today):
;;   Expired        (2018–2021): 4
;;   Expiring-soon  (2025-06):   1
;;   Future / OK    (2088/2099): 2

(ns sample.core
  (:require [clojure.string  :as str]
            [clojure.set     :as set]
            [clojure.edn     :as edn])
  (:import  [java.time Instant Duration]
            [java.util UUID]
            [java.security MessageDigest]))

;; ---------------------------------------------------------------------------
;; Config
;; ---------------------------------------------------------------------------

(defn getenv [key fallback]
  (or (System/getenv key) fallback))

(def default-config
  {:host           (getenv "HOST"        "0.0.0.0")
   :port           (Integer/parseInt (getenv "PORT" "3000"))
   :db-url         (getenv "DB_URL"      "jdbc:postgresql://localhost/app")
   :jwt-secret     (getenv "JWT_SECRET"  "change-me")
   :jwt-expiry     (Integer/parseInt (getenv "JWT_EXPIRY"  "3600"))
   :cache-ttl      (Integer/parseInt (getenv "CACHE_TTL"   "300"))
   :rate-max       (Integer/parseInt (getenv "RATE_MAX"    "100"))
   :rate-window    (Integer/parseInt (getenv "RATE_WINDOW" "60"))
   :env            (getenv "APP_ENV" "development")})

(defn production? [cfg] (= "production" (:env cfg)))

;; ---------------------------------------------------------------------------
;; Errors
;; ---------------------------------------------------------------------------

(defn app-error
  ([code message] (app-error code message 500 nil))
  ([code message status] (app-error code message status nil))
  ([code message status details]
   (ex-info message {:code code :status status :details details})))

(defn validation-error [message details]
  (app-error "VALIDATION_ERROR" message 422 details))

(defn not-found-error [resource]
  (app-error "NOT_FOUND" (str resource " not found") 404))

(defn conflict-error [message]
  (app-error "CONFLICT" message 409))

(defn error-data [e] (ex-data e))
(defn error-code [e] (:code (ex-data e)))

;; ---------------------------------------------------------------------------
;; Result helpers
;; ---------------------------------------------------------------------------

(defn ok  [value] {:ok true  :value value})
(defn err [msg]   {:ok false :error msg})

(defn ok?  [r] (:ok r))
(defn err? [r] (not (:ok r)))

(defn unwrap [r]
  (if (:ok r) (:value r) (throw (ex-info (:error r) {}))))

(defn unwrap-or [r fallback]
  (if (:ok r) (:value r) fallback))

(defn map-result [r f]
  (if (:ok r) (ok (f (:value r))) r))

(defmacro try-result [& body]
  `(try (ok (do ~@body))
        (catch Exception e# (err (.getMessage e#)))))

;; ---------------------------------------------------------------------------
;; Validation
;; ---------------------------------------------------------------------------

;; TODO[2020-10-01]: replace with malli or spec-based schema validation
(def email-re #"^[^\s@]+@[^\s@]+\.[^\s@]+$")

(defn validate-required [field value]
  (when (or (nil? value) (str/blank? (str value)))
    {:field field :message "is required"}))

(defn validate-email [field value]
  (when (not (re-matches email-re (str value)))
    {:field field :message "must be a valid email address" :value value}))

(defn validate-min-length [field value n]
  (when (< (count value) n)
    {:field field :message (str "must be at least " n " characters") :value value}))

(defn validate-max-length [field value n]
  (when (> (count value) n)
    {:field field :message (str "must be at most " n " characters") :value value}))

(defn collect-failures [& checks]
  (filterv some? checks))

;; ---------------------------------------------------------------------------
;; Cache (atom-backed)
;; ---------------------------------------------------------------------------

;; HACK[2019-01-15]: atom-backed store; swap for carmine (Redis) before launch
(defn new-cache []
  (atom {}))

(defn cache-get [cache key]
  (let [entry (get @cache key)
        now   (System/currentTimeMillis)]
    (when (and entry (> (:expires-at entry) now))
      (:value entry))))

(defn cache-set! [cache key value ttl-sec]
  (swap! cache assoc key
         {:value      value
          :expires-at (+ (System/currentTimeMillis) (* ttl-sec 1000))}))

(defn cache-del! [cache key]
  (swap! cache dissoc key))

(defn cache-get-or-set! [cache key ttl-sec f]
  (or (cache-get cache key)
      (let [v (f)]
        (cache-set! cache key v ttl-sec)
        v)))

(defn cache-cleanup! [cache]
  (let [now (System/currentTimeMillis)]
    (swap! cache #(into {} (filter (fn [[_ e]] (> (:expires-at e) now)) %)))))

;; ---------------------------------------------------------------------------
;; Rate limiter
;; ---------------------------------------------------------------------------

(defn new-rate-limiter [window-sec max-requests]
  (atom {:window-sec window-sec :max-requests max-requests :store {}}))

(defn rate-check! [rl-atom key]
  (let [now         (quot (System/currentTimeMillis) 1000)
        {:keys [window-sec max-requests]} @rl-atom
        entry       (get-in @rl-atom [:store key])
        [count rst] (if (or (nil? entry) (<= (:reset-at entry) now))
                      [0 (+ now window-sec)]
                      [(:count entry) (:reset-at entry)])
        new-count   (inc count)]
    (swap! rl-atom assoc-in [:store key] {:count new-count :reset-at rst})
    {:allowed     (<= new-count max-requests)
     :remaining   (max 0 (- max-requests new-count))
     :retry-after (if (<= new-count max-requests) 0 (- rst now))}))

;; ---------------------------------------------------------------------------
;; Pagination
;; ---------------------------------------------------------------------------

(defn paginate [items page-num page-size]
  (let [offset (-> page-num dec (* page-size))
        chunk  (->> items (drop offset) (take page-size) vec)
        total  (count items)]
    {:items    chunk
     :total    total
     :page-num page-num
     :page-size page-size
     :has-next (< (+ offset (count chunk)) total)
     :has-prev (> page-num 1)}))

;; ---------------------------------------------------------------------------
;; Feature flags
;; ---------------------------------------------------------------------------

;; TODO[2099-10-01][platform]: replace atom with remote flag service client
(defn new-flag-service []
  (atom {}))

(defn define-flag! [flags name enabled rollout & allowlist]
  (swap! flags assoc name {:enabled enabled :rollout rollout :allowlist (set allowlist)}))

(defn flag-enabled? [flags name user-id]
  (let [flag (get @flags name)]
    (boolean
     (and flag
          (:enabled flag)
          (or (contains? (:allowlist flag) user-id)
              (>= (:rollout flag) 100)
              (let [hash   (-> (MessageDigest/getInstance "MD5")
                               (.digest (.getBytes (str name ":" (or user-id "anon")))))
                    bucket (mod (Math/abs (aget hash 0)) 100)]
                (< bucket (:rollout flag))))))))

;; ---------------------------------------------------------------------------
;; Event bus
;; ---------------------------------------------------------------------------

;; FIXME[2021-06-01]: handlers run in calling thread; add core.async dispatch
(defn new-event-bus []
  (atom {}))

(defn subscribe! [bus event-type handler]
  (swap! bus update event-type (fnil conj []) handler)
  (fn [] (swap! bus update event-type #(filterv (partial not= handler) %))))

(defn publish! [bus event-type payload]
  (let [event {:id         (str (UUID/randomUUID))
               :type       event-type
               :occurred-at (str (Instant/now))
               :payload    payload}]
    (doseq [handler (get @bus event-type [])]
      (try (handler event)
           (catch Exception e
             (println "event handler error:" (.getMessage e)))))
    event))

;; ---------------------------------------------------------------------------
;; Utilities
;; ---------------------------------------------------------------------------

(defn slugify [text]
  (-> text
      str/lower-case
      (str/replace #"[^a-z0-9\s-]" "")
      (str/replace #"[\s-]+" "-")
      (str/trim)))

(defn mask-email [email]
  (let [[local domain] (str/split email #"@" 2)]
    (if (nil? domain) email
        (let [visible (subs local 0 (min 2 (count local)))
              stars   (apply str (repeat (max 1 (- (count local) 2)) "*"))]
          (str visible stars "@" domain)))))

(defn truncate [s max-len]
  (if (<= (count s) max-len) s
      (str (subs s 0 (- max-len 1)) "…")))

(defn chunk-seq [n coll]
  (partition n n nil coll))

(defn group-by-key [f coll]
  (reduce (fn [acc item]
            (update acc (f item) (fnil conj []) item))
          {} coll))

(defn format-bytes [bytes]
  (let [units ["B" "KB" "MB" "GB" "TB"]]
    (loop [v (double bytes) i 0]
      (if (or (< v 1024) (= i (dec (count units))))
        (format "%.2f %s" v (nth units i))
        (recur (/ v 1024) (inc i))))))

(defn format-duration [ms]
  (cond
    (< ms 1000)   (str ms "ms")
    (< ms 60000)  (format "%.1fs" (/ ms 1000.0))
    :else         (str (quot ms 60000) "m " (quot (rem ms 60000) 1000) "s")))

(defn retry [n f]
  (loop [attempts n]
    (let [result (try-result (f))]
      (if (or (ok? result) (zero? (dec attempts)))
        result
        (recur (dec attempts))))))

;; FIXME[2025-06-10]: memoize is not safe for functions with side effects
(defn memoize-ttl [ttl-ms f]
  (let [cache (atom {})]
    (fn [& args]
      (let [now   (System/currentTimeMillis)
            entry (get @cache args)]
        (if (and entry (< (- now (:at entry)) ttl-ms))
          (:value entry)
          (let [v (apply f args)]
            (swap! cache assoc args {:value v :at now})
            v))))))
