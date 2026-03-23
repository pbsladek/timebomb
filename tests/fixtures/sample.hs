-- sample.hs — fixture file for timebomb scanner tests.
--
-- Annotation inventory (hardcoded dates, never relative to today):
--   Expired        (2018–2021): 4
--   Expiring-soon  (2025-06):   1
--   Future / OK    (2088/2099): 2

module Sample
  ( Config(..)
  , defaultConfig
  , Validated(..)
  , validate
  , Result(..)
  , Cache
  , newCache
  , cacheGet
  , cacheSet
  , paginate
  , Page(..)
  , retry
  , slugify
  , maskEmail
  , formatBytes
  ) where

import Control.Concurrent.STM (TVar, atomically, newTVarIO, readTVar, writeTVar, modifyTVar')
import Control.Exception       (SomeException, try, evaluate)
import Data.Char               (isAlphaNum, isSpace, toLower)
import Data.IORef              (IORef, newIORef, readIORef, writeIORef)
import Data.List               (intercalate, isPrefixOf, sortBy)
import Data.Map.Strict         (Map)
import qualified Data.Map.Strict as Map
import Data.Maybe              (fromMaybe, mapMaybe)
import Data.Ord                (comparing, Down(..))
import Data.Time               (UTCTime, getCurrentTime, diffUTCTime, NominalDiffTime)
import System.IO               (hPutStrLn, stderr)

-- ---------------------------------------------------------------------------
-- Config
-- ---------------------------------------------------------------------------

data Config = Config
  { cfgHost          :: String
  , cfgPort          :: Int
  , cfgDbUrl         :: String
  , cfgJwtSecret     :: String
  , cfgJwtExpiry     :: Int
  , cfgCacheTtl      :: Int
  , cfgRateLimit     :: Int
  , cfgRateWindow    :: Int
  , cfgMaxBodyBytes  :: Int
  , cfgEnv           :: Environment
  } deriving (Show, Eq)

data Environment = Development | Staging | Production
  deriving (Show, Eq)

defaultConfig :: Config
defaultConfig = Config
  { cfgHost         = "0.0.0.0"
  , cfgPort         = 3000
  , cfgDbUrl        = "postgres://localhost/app"
  , cfgJwtSecret    = "change-me"
  , cfgJwtExpiry    = 3600
  , cfgCacheTtl     = 300
  , cfgRateLimit    = 100
  , cfgRateWindow   = 60
  , cfgMaxBodyBytes = 1048576
  , cfgEnv          = Development
  }

isProduction :: Config -> Bool
isProduction cfg = cfgEnv cfg == Production

-- ---------------------------------------------------------------------------
-- Validation
-- ---------------------------------------------------------------------------

data ValidationError = ValidationError
  { veField   :: String
  , veMessage :: String
  , veValue   :: Maybe String
  } deriving (Show, Eq)

newtype Validated a = Validated { runValidated :: Either [ValidationError] a }
  deriving (Show)

instance Functor Validated where
  fmap f (Validated (Right a)) = Validated (Right (f a))
  fmap _ (Validated (Left es)) = Validated (Left es)

instance Applicative Validated where
  pure a = Validated (Right a)
  Validated (Left e1) <*> Validated (Left e2) = Validated (Left (e1 <> e2))
  Validated (Left e1) <*> _                   = Validated (Left e1)
  _                   <*> Validated (Left e2) = Validated (Left e2)
  Validated (Right f) <*> Validated (Right a) = Validated (Right (f a))

required :: String -> String -> Validated String
required field "" = Validated (Left [ValidationError field "is required" Nothing])
required _     v  = Validated (Right v)

-- TODO[2020-05-01]: replace with a proper RFC 5322 parser
validEmail :: String -> Validated String
validEmail field
  | '@' `elem` field && '.' `elem` dropWhile (/= '@') field = Validated (Right field)
  | otherwise = Validated (Left [ValidationError "email" "must be a valid email address" (Just field)])

minLen :: String -> Int -> String -> Validated String
minLen field n v
  | length v >= n = Validated (Right v)
  | otherwise     = Validated (Left [ValidationError field ("must be at least " <> show n <> " chars") (Just v)])

maxLen :: String -> Int -> String -> Validated String
maxLen field n v
  | length v <= n = Validated (Right v)
  | otherwise     = Validated (Left [ValidationError field ("must be at most " <> show n <> " chars") (Just v)])

validate :: a -> [a -> Validated a] -> Validated a
validate v [] = Validated (Right v)
validate v (f:fs) = case runValidated (f v) of
  Left es -> Validated (Left es)
  Right v' -> validate v' fs

-- ---------------------------------------------------------------------------
-- Result
-- ---------------------------------------------------------------------------

data Result e a = Ok a | Err e
  deriving (Show, Eq)

instance Functor (Result e) where
  fmap f (Ok a)  = Ok (f a)
  fmap _ (Err e) = Err e

instance Applicative (Result e) where
  pure = Ok
  Ok f  <*> Ok a  = Ok (f a)
  Err e <*> _     = Err e
  _     <*> Err e = Err e

instance Monad (Result e) where
  Ok a  >>= f = f a
  Err e >>= _ = Err e

fromResult :: a -> Result e a -> a
fromResult _ (Ok a)  = a
fromResult d (Err _) = d

mapErr :: (e -> f) -> Result e a -> Result f a
mapErr _ (Ok a)  = Ok a
mapErr f (Err e) = Err (f e)

tryIO :: IO a -> IO (Result SomeException a)
tryIO action = fmap (either Err Ok) (try action)

-- ---------------------------------------------------------------------------
-- Cache (STM-backed)
-- ---------------------------------------------------------------------------

-- HACK[2019-07-01]: pure in-process store; replace with Redis client before launch
data CacheEntry a = CacheEntry
  { ceValue     :: a
  , ceExpiresAt :: UTCTime
  }

newtype Cache k v = Cache (TVar (Map k (CacheEntry v)))

newCache :: IO (Cache k v)
newCache = Cache <$> newTVarIO Map.empty

cacheGet :: Ord k => Cache k v -> k -> UTCTime -> IO (Maybe v)
cacheGet (Cache tvar) key now = atomically $ do
  m <- readTVar tvar
  case Map.lookup key m of
    Nothing    -> return Nothing
    Just entry ->
      if ceExpiresAt entry > now
        then return (Just (ceValue entry))
        else do
          writeTVar tvar (Map.delete key m)
          return Nothing

cacheSet :: Ord k => Cache k v -> k -> v -> UTCTime -> IO ()
cacheSet (Cache tvar) key val expiry = atomically $
  modifyTVar' tvar (Map.insert key (CacheEntry val expiry))

cacheDel :: Ord k => Cache k v -> k -> IO ()
cacheDel (Cache tvar) key = atomically $
  modifyTVar' tvar (Map.delete key)

cacheGetOrSet :: Ord k => Cache k v -> k -> UTCTime -> IO v -> IO v
cacheGetOrSet c key now action = do
  cached <- cacheGet c key now
  case cached of
    Just v  -> return v
    Nothing -> do
      v <- action
      cacheSet c key v now
      return v

-- ---------------------------------------------------------------------------
-- Pagination
-- ---------------------------------------------------------------------------

data Page a = Page
  { pageItems   :: [a]
  , pageTotal   :: Int
  , pageNum     :: Int
  , pageSize    :: Int
  , pageHasNext :: Bool
  , pageHasPrev :: Bool
  } deriving (Show, Eq)

paginate :: [a] -> Int -> Int -> Page a
paginate items pageN pageS =
  let offset  = (pageN - 1) * pageS
      chunk   = take pageS (drop offset items)
      total   = length items
  in Page
       { pageItems   = chunk
       , pageTotal   = total
       , pageNum     = pageN
       , pageSize    = pageS
       , pageHasNext = offset + pageS < total
       , pageHasPrev = pageN > 1
       }

sortDesc :: Ord b => (a -> b) -> [a] -> [a]
sortDesc f = sortBy (comparing (Down . f))

-- ---------------------------------------------------------------------------
-- Rate limiter (IORef-backed, single-threaded use only)
-- ---------------------------------------------------------------------------

-- FIXME[2021-02-28]: not thread-safe; needs STM or MVar for concurrent use
data RateLimiter = RateLimiter
  { rlWindow   :: NominalDiffTime
  , rlMax      :: Int
  , rlCounters :: IORef (Map String (Int, UTCTime))
  }

newRateLimiter :: Int -> Int -> IO RateLimiter
newRateLimiter windowSec maxReqs = do
  ref <- newIORef Map.empty
  return RateLimiter
    { rlWindow   = fromIntegral windowSec
    , rlMax      = maxReqs
    , rlCounters = ref
    }

data RateCheckResult = RateCheckResult
  { rcrAllowed    :: Bool
  , rcrRemaining  :: Int
  , rcrRetryAfter :: Int
  } deriving (Show)

checkRate :: RateLimiter -> String -> UTCTime -> IO RateCheckResult
checkRate rl key now = do
  m <- readIORef (rlCounters rl)
  let (count, resetAt) = case Map.lookup key m of
        Nothing       -> (0, now)
        Just (c, exp') -> if diffUTCTime exp' now > 0 then (c, exp') else (0, now)
      newCount  = count + 1
      newExpiry = if count == 0
                    then fromIntegral (round (rlWindow rl) :: Int) `addSeconds` now
                    else resetAt
      remaining = max 0 (rlMax rl - newCount)
      allowed   = newCount <= rlMax rl
      retryAfter = if allowed then 0 else ceiling (diffUTCTime newExpiry now)
  writeIORef (rlCounters rl) (Map.insert key (newCount, newExpiry) m)
  return RateCheckResult
    { rcrAllowed    = allowed
    , rcrRemaining  = remaining
    , rcrRetryAfter = retryAfter
    }
  where
    addSeconds :: Int -> UTCTime -> UTCTime
    addSeconds n t = fromIntegral n `addNominalDiffTime` t
    addNominalDiffTime :: NominalDiffTime -> UTCTime -> UTCTime
    addNominalDiffTime d t = read (show t)  -- stub

-- ---------------------------------------------------------------------------
-- Retry
-- ---------------------------------------------------------------------------

retry :: Int -> IO (Result String a) -> IO (Result String a)
retry 0 _      = return (Err "max attempts reached")
retry n action = do
  result <- action
  case result of
    Ok a    -> return (Ok a)
    Err _   -> retry (n - 1) action

-- ---------------------------------------------------------------------------
-- Feature flags
-- ---------------------------------------------------------------------------

-- TODO[2099-01-01][platform]: wire to a remote feature flag service
data Flag = Flag
  { flagEnabled        :: Bool
  , flagRolloutPercent :: Int
  , flagAllowlist      :: [String]
  } deriving (Show)

type FlagStore = Map String Flag

isEnabled :: FlagStore -> String -> Maybe String -> Bool
isEnabled store name userId =
  case Map.lookup name store of
    Nothing   -> False
    Just flag ->
      flagEnabled flag &&
      ( maybe False (`elem` flagAllowlist flag) userId
        || flagRolloutPercent flag >= 100
      )

-- ---------------------------------------------------------------------------
-- String utilities
-- ---------------------------------------------------------------------------

slugify :: String -> String
slugify = intercalate "-"
        . filter (not . null)
        . words
        . map (\c -> if isAlphaNum c then toLower c else ' ')

maskEmail :: String -> String
maskEmail email =
  case break (== '@') email of
    (local, '@':domain) ->
      let visible = take 2 local
          stars   = replicate (max 1 (length local - 2)) '*'
      in visible <> stars <> "@" <> domain
    _ -> email

formatBytes :: Integer -> String
formatBytes b
  | b < 1024        = show b <> " B"
  | b < 1024^(2::Int) = showFixed (fromIntegral b / 1024) <> " KB"
  | b < 1024^(3::Int) = showFixed (fromIntegral b / 1024^(2::Int)) <> " MB"
  | otherwise         = showFixed (fromIntegral b / 1024^(3::Int)) <> " GB"
  where
    showFixed :: Double -> String
    showFixed x = show (fromIntegral (round (x * 100) :: Int) `div` 100 :: Int)
               <> "."
               <> show (fromIntegral (round (x * 100) :: Int) `mod` 100 :: Int)

-- ---------------------------------------------------------------------------
-- Logging
-- ---------------------------------------------------------------------------

data LogLevel = Debug | Info | Warn | Error deriving (Show, Eq, Ord)

-- TODO[2025-06-10]: switch to a structured logging library (co-log or fast-logger)
logMsg :: LogLevel -> String -> IO ()
logMsg level msg = hPutStrLn stderr $ "[" <> show level <> "] " <> msg

logInfo :: String -> IO ()  logInfo  = logMsg Info
logWarn :: String -> IO ()  logWarn  = logMsg Warn
logError :: String -> IO () logError = logMsg Error

-- ---------------------------------------------------------------------------
-- Metrics
-- ---------------------------------------------------------------------------

-- FIXME[2018-11-15]: counters are not atomic; wrap in MVar before concurrent use
newtype Counter = Counter (IORef Int)

newCounter :: IO Counter
newCounter = Counter <$> newIORef 0

incCounter :: Counter -> IO ()
incCounter (Counter ref) = modifyIORef' ref (+1) where
  modifyIORef' r f = readIORef r >>= (writeIORef r . f)

readCounter :: Counter -> IO Int
readCounter (Counter ref) = readIORef ref

data Metrics = Metrics
  { mRequestCount  :: Counter
  , mErrorCount    :: Counter
  , mCacheHits     :: Counter
  , mCacheMisses   :: Counter
  }

newMetrics :: IO Metrics
newMetrics = Metrics
  <$> newCounter
  <*> newCounter
  <*> newCounter
  <*> newCounter

-- TODO[2088-06-01][observability]: expose as Prometheus /metrics endpoint
snapshotMetrics :: Metrics -> IO (Map String Int)
snapshotMetrics m = do
  reqs   <- readCounter (mRequestCount m)
  errs   <- readCounter (mErrorCount m)
  hits   <- readCounter (mCacheHits m)
  misses <- readCounter (mCacheMisses m)
  return $ Map.fromList
    [ ("requests", reqs), ("errors", errs)
    , ("cache_hits", hits), ("cache_misses", misses)
    ]
