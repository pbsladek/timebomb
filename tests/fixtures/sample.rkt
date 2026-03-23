#lang racket/base
;; sample.rkt — fixture file for timebomb scanner tests.
;;
;; Annotation inventory (hardcoded dates, never relative to today):
;;   Expired        (2018–2021): 4
;;   Expiring-soon  (2025-06):   1
;;   Future / OK    (2088/2099): 2

(require racket/string
         racket/list
         racket/match
         racket/contract
         racket/hash)

(provide make-config config-production?
         ok err ok? err? unwrap unwrap-or
         validate-required validate-email collect-failures
         make-cache cache-get cache-set! cache-get-or-set!
         make-rate-limiter rate-check!
         paginate make-page
         slugify mask-email format-bytes format-duration
         retry)

;; ---------------------------------------------------------------------------
;; Config
;; ---------------------------------------------------------------------------

(struct config
  (host port db-url jwt-secret jwt-expiry cache-ttl rate-max rate-window env)
  #:transparent)

(define (getenv* key fallback)
  (or (getenv key) fallback))

(define (make-config)
  (config
   (getenv* "HOST"       "0.0.0.0")
   (string->number (getenv* "PORT" "3000"))
   (getenv* "DB_URL"     "postgres://localhost/app")
   (getenv* "JWT_SECRET" "change-me")
   (string->number (getenv* "JWT_EXPIRY"  "3600"))
   (string->number (getenv* "CACHE_TTL"   "300"))
   (string->number (getenv* "RATE_MAX"    "100"))
   (string->number (getenv* "RATE_WINDOW" "60"))
   (getenv* "APP_ENV" "development")))

(define (config-production? cfg)
  (string=? (config-env cfg) "production"))

;; ---------------------------------------------------------------------------
;; Result type
;; ---------------------------------------------------------------------------

(struct result-ok  (value) #:transparent)
(struct result-err (message) #:transparent)

(define (ok value)  (result-ok value))
(define (err msg)   (result-err msg))
(define (ok? r)     (result-ok? r))
(define (err? r)    (result-err? r))

(define (unwrap r)
  (match r
    [(result-ok v)  v]
    [(result-err m) (error "unwrap on Err" m)]))

(define (unwrap-or r fallback)
  (if (ok? r) (result-ok-value r) fallback))

(define (map-result r f)
  (if (ok? r) (ok (f (result-ok-value r))) r))

(define-syntax try-result
  (syntax-rules ()
    [(_ body ...)
     (with-handlers ([exn:fail? (lambda (e) (err (exn-message e)))])
       (ok (begin body ...)))]))

;; ---------------------------------------------------------------------------
;; Validation
;; ---------------------------------------------------------------------------

;; TODO[2020-08-15]: replace with an algebraic validation library
(define email-pattern #px"^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$")

(define (validate-required field value)
  (if (or (not value) (string=? value ""))
      (hash 'field field 'message "is required")
      #f))

(define (validate-email field value)
  (if (and (string? value) (regexp-match email-pattern value))
      #f
      (hash 'field field 'message "must be a valid email address" 'value value)))

(define (validate-min-length field value n)
  (if (>= (string-length value) n)
      #f
      (hash 'field field 'message (format "must be at least ~a characters" n) 'value value)))

(define (validate-max-length field value n)
  (if (<= (string-length value) n)
      #f
      (hash 'field field 'message (format "must be at most ~a characters" n) 'value value)))

(define (collect-failures . checks)
  (filter values checks))

;; ---------------------------------------------------------------------------
;; Cache (hash-table backed)
;; ---------------------------------------------------------------------------

;; HACK[2019-05-01]: in-process hash store; replace with redis before scaling
(define (make-cache) (make-hash))

(define (cache-get cache key)
  (define entry (hash-ref cache key #f))
  (define now   (current-seconds))
  (cond
    [(not entry) #f]
    [(> (cdr entry) now) (car entry)]
    [else (hash-remove! cache key) #f]))

(define (cache-set! cache key value ttl)
  (hash-set! cache key (cons value (+ (current-seconds) ttl))))

(define (cache-del! cache key)
  (hash-remove! cache key))

(define (cache-get-or-set! cache key ttl thunk)
  (or (cache-get cache key)
      (let ([v (thunk)])
        (cache-set! cache key v ttl)
        v)))

(define (cache-cleanup! cache)
  (define now (current-seconds))
  (for ([k (in-list (hash-keys cache))])
    (define entry (hash-ref cache k #f))
    (when (and entry (<= (cdr entry) now))
      (hash-remove! cache k))))

;; ---------------------------------------------------------------------------
;; Rate limiter
;; ---------------------------------------------------------------------------

;; FIXME[2020-04-10]: store is not thread-safe; add a semaphore before using with threads
(struct rate-limiter (window-sec max-requests store) #:mutable #:transparent)

(define (make-rate-limiter window-sec max-requests)
  (rate-limiter window-sec max-requests (make-hash)))

(define (rate-check! rl key)
  (define now   (current-seconds))
  (define store (rate-limiter-store rl))
  (define entry (hash-ref store key #f))
  (define-values (count rst)
    (cond
      [(not entry)           (values 0 (+ now (rate-limiter-window-sec rl)))]
      [(> (cdr entry) now)   (values (car entry) (cdr entry))]
      [else                  (values 0 (+ now (rate-limiter-window-sec rl)))]))
  (define new-count (add1 count))
  (hash-set! store key (cons new-count rst))
  (hash 'allowed     (<= new-count (rate-limiter-max-requests rl))
        'remaining   (max 0 (- (rate-limiter-max-requests rl) new-count))
        'retry-after (if (<= new-count (rate-limiter-max-requests rl)) 0 (- rst now))))

;; ---------------------------------------------------------------------------
;; Pagination
;; ---------------------------------------------------------------------------

(struct page (items total page-num page-size has-next has-prev) #:transparent)

(define (make-page items total page-num page-size)
  (define offset (max 0 (* (sub1 page-num) page-size)))
  (page items total page-num page-size
        (< (+ offset (length items)) total)
        (> page-num 1)))

(define (paginate items page-num page-size)
  (define offset (max 0 (* (sub1 page-num) page-size)))
  (define chunk  (take (drop items offset) (min page-size (- (length items) offset))))
  (make-page chunk (length items) page-num page-size))

;; ---------------------------------------------------------------------------
;; Feature flags
;; ---------------------------------------------------------------------------

;; TODO[2099-07-15][platform]: plug in a remote feature flag service
(define *flags* (make-hash))

(define (define-flag! name enabled rollout . allowlist)
  (hash-set! *flags* name (hash 'enabled enabled 'rollout rollout 'allowlist allowlist)))

(define (flag-enabled? name [user-id #f])
  (define flag (hash-ref *flags* name #f))
  (and flag
       (hash-ref flag 'enabled #f)
       (or (and user-id (member user-id (hash-ref flag 'allowlist '())))
           (>= (hash-ref flag 'rollout 0) 100))))

;; ---------------------------------------------------------------------------
;; Utilities
;; ---------------------------------------------------------------------------

(define (slugify text)
  (string-trim
   (regexp-replace* #px"[^a-z0-9]+" (string-downcase text) "-")
   "-"))

(define (mask-email email)
  (define at-pos (for/first ([i (in-range (string-length email))]
                             #:when (char=? (string-ref email i) #\@))
                   i))
  (if (not at-pos) email
      (let* ([local   (substring email 0 at-pos)]
             [domain  (substring email (add1 at-pos))]
             [visible (substring local 0 (min 2 (string-length local)))]
             [stars   (make-string (max 1 (- (string-length local) 2)) #\*)])
        (string-append visible stars "@" domain))))

(define (truncate-string s max-len [suffix "…"])
  (if (<= (string-length s) max-len) s
      (string-append (substring s 0 (- max-len (string-length suffix))) suffix)))

(define (format-bytes bytes)
  (define units '("B" "KB" "MB" "GB" "TB"))
  (let loop ([v (exact->inexact bytes)] [us units])
    (if (or (< v 1024) (null? (cdr us)))
        (format "~,2F ~a" v (car us))
        (loop (/ v 1024) (cdr us)))))

;; FIXME[2025-06-10]: format-duration does not handle negative durations
(define (format-duration ms)
  (cond
    [(< ms 1000)   (format "~ams" ms)]
    [(< ms 60000)  (format "~,1Fs" (/ ms 1000.0))]
    [else          (format "~am ~as" (quotient ms 60000) (quotient (remainder ms 60000) 1000))]))

(define (chunk-list lst n)
  (if (null? lst) '()
      (cons (take lst (min n (length lst)))
            (chunk-list (drop lst (min n (length lst))) n))))

(define (group-by-key f lst)
  (foldl (lambda (item acc)
           (define k (f item))
           (hash-set acc k (append (hash-ref acc k '()) (list item))))
         (hash) lst))

(define (retry n thunk)
  (let loop ([attempts n])
    (define result (try-result (thunk)))
    (cond
      [(ok? result)     result]
      [(= attempts 1)   result]
      [else             (loop (sub1 attempts))])))

;; TODO[2088-11-01][observability]: add contract-based instrumentation for all public fns
(define/contract (safe-divide a b)
  (-> number? (and/c number? (not/c zero?)) number?)
  (/ a b))
