;;; sample.lisp — fixture file for timebomb scanner tests.
;;;
;;; Annotation inventory (hardcoded dates, never relative to today):
;;;   Expired        (2018–2021): 4
;;;   Expiring-soon  (2025-06):   1
;;;   Future / OK    (2088/2099): 2

(defpackage #:sample
  (:use #:cl)
  (:export #:make-config
           #:config-production-p
           #:validate-email
           #:validate-required
           #:collect-failures
           #:make-cache
           #:cache-get
           #:cache-set
           #:cache-get-or-set
           #:make-rate-limiter
           #:rate-check
           #:paginate
           #:slugify
           #:mask-email
           #:format-bytes
           #:retry-with-backoff))

(in-package #:sample)

;;; ---------------------------------------------------------------------------
;;; Config
;;; ---------------------------------------------------------------------------

(defstruct config
  (host         "0.0.0.0"     :type string)
  (port         3000           :type integer)
  (db-url       "postgres://localhost/app" :type string)
  (jwt-secret   "change-me"   :type string)
  (jwt-expiry   3600           :type integer)
  (cache-ttl    300            :type integer)
  (rate-max     100            :type integer)
  (rate-window  60             :type integer)
  (env          "development"  :type string))

(defun make-config-from-env ()
  (make-config
   :host       (or (uiop:getenv "HOST")       "0.0.0.0")
   :port       (parse-integer (or (uiop:getenv "PORT") "3000"))
   :db-url     (or (uiop:getenv "DB_URL")     "postgres://localhost/app")
   :jwt-secret (or (uiop:getenv "JWT_SECRET") "change-me")
   :env        (or (uiop:getenv "APP_ENV")    "development")))

(defun config-production-p (cfg)
  (string= (config-env cfg) "production"))

;;; ---------------------------------------------------------------------------
;;; Conditions (errors)
;;; ---------------------------------------------------------------------------

(define-condition app-error (error)
  ((code    :initarg :code    :reader app-error-code)
   (status  :initarg :status  :reader app-error-status  :initform 500)
   (details :initarg :details :reader app-error-details :initform nil))
  (:report (lambda (c s)
             (format s "[~A] ~A" (app-error-code c) (princ-to-string c)))))

(define-condition validation-error (app-error)
  ()
  (:default-initargs :code "VALIDATION_ERROR" :status 422))

(define-condition not-found-error (app-error)
  ((resource :initarg :resource :reader not-found-resource))
  (:default-initargs :code "NOT_FOUND" :status 404)
  (:report (lambda (c s)
             (format s "~A not found" (not-found-resource c)))))

;;; ---------------------------------------------------------------------------
;;; Result type
;;; ---------------------------------------------------------------------------

(defstruct (result (:constructor %make-result))
  ok value error)

(defun ok (value)
  (%make-result :ok t :value value :error nil))

(defun err (message)
  (%make-result :ok nil :value nil :error message))

(defun result-ok-p (r)  (result-ok r))
(defun result-err-p (r) (not (result-ok r)))

(defun unwrap (r)
  (if (result-ok r)
      (result-value r)
      (error "unwrap called on Err: ~A" (result-error r))))

(defun unwrap-or (r fallback)
  (if (result-ok r) (result-value r) fallback))

(defun map-result (r f)
  (if (result-ok r) (ok (funcall f (result-value r))) r))

(defmacro try-result (&body body)
  `(handler-case (ok (progn ,@body))
     (error (e) (err (princ-to-string e)))))

;;; ---------------------------------------------------------------------------
;;; Validation
;;; ---------------------------------------------------------------------------

;; TODO[2021-07-01]: replace ad-hoc validators with a cl-json-schema validator
(defun validate-required (field value)
  (when (or (null value) (string= value ""))
    (list :field field :message "is required")))

(defun validate-email (field value)
  ;; Minimal check; not RFC 5322 compliant.
  (unless (and (stringp value)
               (find #\@ value)
               (find #\. value))
    (list :field field :message "must be a valid email address" :value value)))

(defun validate-min-length (field value min)
  (when (< (length value) min)
    (list :field field :message (format nil "must be at least ~A characters" min) :value value)))

(defun validate-max-length (field value max)
  (when (> (length value) max)
    (list :field field :message (format nil "must be at most ~A characters" max) :value value)))

(defun collect-failures (&rest checks)
  (remove nil checks))

;;; ---------------------------------------------------------------------------
;;; Cache (hash-table backed)
;;; ---------------------------------------------------------------------------

;; HACK[2018-07-01]: hash-table stand-in; wire up cl-redis before going live
(defstruct cache
  (store (make-hash-table :test 'equal) :type hash-table))

(defun cache-get (c key)
  (let* ((entry (gethash key (cache-store c)))
         (now   (get-universal-time)))
    (when (and entry (> (cdr entry) now))
      (car entry))))

(defun cache-set (c key value ttl)
  (setf (gethash key (cache-store c))
        (cons value (+ (get-universal-time) ttl))))

(defun cache-del (c key)
  (remhash key (cache-store c)))

(defun cache-get-or-set (c key ttl thunk)
  (or (cache-get c key)
      (let ((v (funcall thunk)))
        (cache-set c key v ttl)
        v)))

(defun cache-cleanup (c)
  (let ((now (get-universal-time)))
    (maphash (lambda (k e)
               (when (<= (cdr e) now)
                 (remhash k (cache-store c))))
             (cache-store c))))

;;; ---------------------------------------------------------------------------
;;; Rate limiter
;;; ---------------------------------------------------------------------------

;; FIXME[2019-12-01]: not thread-safe; wrap store access in a lock
(defstruct rate-limiter
  (window-sec  60  :type integer)
  (max-requests 100 :type integer)
  (store (make-hash-table :test 'equal) :type hash-table))

(defun rate-check (rl key)
  (let* ((now    (get-universal-time))
         (entry  (gethash key (rate-limiter-store rl)))
         (count  (if (and entry (> (cdr entry) now)) (car entry) 0))
         (rst    (if (and entry (> (cdr entry) now)) (cdr entry) (+ now (rate-limiter-window-sec rl))))
         (new-c  (1+ count))
         (max    (rate-limiter-max-requests rl)))
    (setf (gethash key (rate-limiter-store rl)) (cons new-c rst))
    (list :allowed     (<= new-c max)
          :remaining   (max 0 (- max new-c))
          :retry-after (if (<= new-c max) 0 (- rst now)))))

;;; ---------------------------------------------------------------------------
;;; Pagination
;;; ---------------------------------------------------------------------------

(defun paginate (items page-num page-size)
  (let* ((offset (max 0 (* (1- page-num) page-size)))
         (chunk  (subseq items offset (min (+ offset page-size) (length items))))
         (total  (length items)))
    (list :items    chunk
          :total    total
          :page-num page-num
          :page-size page-size
          :has-next (< (+ offset (length chunk)) total)
          :has-prev (> page-num 1))))

;;; ---------------------------------------------------------------------------
;;; Feature flags
;;; ---------------------------------------------------------------------------

;; TODO[2088-09-01][platform]: replace hash-table with remote LaunchDarkly client
(defvar *flags* (make-hash-table :test 'equal))

(defun define-flag (name enabled rollout &rest allowlist)
  (setf (gethash name *flags*)
        (list :enabled enabled :rollout rollout :allowlist allowlist)))

(defun flag-enabled-p (name &optional user-id)
  (let ((flag (gethash name *flags*)))
    (and flag
         (getf flag :enabled)
         (or (member user-id (getf flag :allowlist) :test #'equal)
             (>= (getf flag :rollout) 100)))))

;;; ---------------------------------------------------------------------------
;;; Utilities
;;; ---------------------------------------------------------------------------

(defun slugify (text)
  (string-trim "-"
    (substitute #\- #\Space
      (remove-if-not (lambda (c)
                       (or (alphanumericp c) (char= c #\Space)))
                     (string-downcase text)))))

(defun mask-email (email)
  (let ((at-pos (position #\@ email)))
    (if (null at-pos) email
        (let* ((local   (subseq email 0 at-pos))
               (domain  (subseq email (1+ at-pos)))
               (visible (subseq local 0 (min 2 (length local))))
               (stars   (make-string (max 1 (- (length local) 2)) :initial-element #\*)))
          (concatenate 'string visible stars "@" domain)))))

(defun truncate-string (s max-len &optional (suffix "…"))
  (if (<= (length s) max-len) s
      (concatenate 'string (subseq s 0 (- max-len (length suffix))) suffix)))

(defun format-bytes (bytes)
  (let ((units '("B" "KB" "MB" "GB" "TB"))
        (v (coerce bytes 'double)))
    (loop for unit in units
          for rest on (cdr units)
          while (and (>= v 1024) rest)
          do (setf v (/ v 1024))
          finally (return (format nil "~,2F ~A" v unit)))))

(defun retry-with-backoff (n delay-sec thunk)
  (loop for attempt from 1 to n
        for result = (try-result (funcall thunk))
        when (result-ok-p result) return result
        unless (= attempt n) do (sleep delay-sec)
        finally (return result)))

;; FIXME[2025-06-08]: format-duration uses integer arithmetic and truncates sub-second precision
(defun format-duration (ms)
  (cond ((< ms 1000)   (format nil "~Ams" ms))
        ((< ms 60000)  (format nil "~,1Fs" (/ ms 1000.0)))
        (t             (format nil "~Am ~As" (floor ms 60000) (floor (mod ms 60000) 1000)))))

;;; ---------------------------------------------------------------------------
;;; Metrics (simple counters)
;;; ---------------------------------------------------------------------------

(defstruct counter
  (value 0 :type integer))

(defun inc-counter (c &optional (by 1))
  (incf (counter-value c) by))

(defun read-counter (c) (counter-value c))
(defun reset-counter (c) (setf (counter-value c) 0))

;; TODO[2099-04-20][observability]: add histogram and expose via HTTP /metrics
(defvar *metrics* (make-hash-table :test 'equal))

(defun get-counter (name)
  (or (gethash name *metrics*)
      (setf (gethash name *metrics*) (make-counter))))

(defun metrics-snapshot ()
  (let ((snap '()))
    (maphash (lambda (k c) (push (cons k (counter-value c)) snap)) *metrics*)
    snap))
