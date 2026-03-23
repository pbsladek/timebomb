-- This file contains structured timebomb annotations for use in integration tests.
-- Dates are chosen to be permanently in the past or far future so tests never
-- depend on the current wall-clock date.

-- ── Expired annotations (date in the past) ────────────────────────────────

-- TODO[2020-01-01]: drop temp_users table after migration completes
CREATE TABLE temp_users (
    id          SERIAL PRIMARY KEY,
    username    VARCHAR(255) NOT NULL,
    email       VARCHAR(320) NOT NULL,
    legacy_field TEXT
);

-- FIXME[2019-08-15]: remove this view once reporting pipeline is updated
CREATE VIEW legacy_report_view AS
SELECT id, username FROM temp_users;

-- HACK[2018-06-01]: denormalized column added for perf, remove after index added
ALTER TABLE temp_users ADD COLUMN cache_value TEXT;

-- TEMP[2020-03-31]: temporary index for slow query workaround, drop after upgrade
CREATE INDEX idx_temp_cache ON temp_users(cache_value);

-- REMOVEME[2021-01-15]: old audit log table, superseded by audit_v2
CREATE TABLE audit_log_old (
    id          SERIAL PRIMARY KEY,
    action      TEXT        NOT NULL,
    actor_id    INTEGER,
    resource    TEXT,
    created_at  TIMESTAMP   NOT NULL DEFAULT NOW()
);

-- TODO[2020-01-01][eve]: eve to drop after confirming backfill job succeeded
ALTER TABLE temp_users ADD COLUMN backfill_done BOOLEAN NOT NULL DEFAULT FALSE;

-- ── Expiring-soon annotations (dates used by tests injecting a close `today`) ──
-- Tests that need "expiring soon" status should inject today = 2025-06-01 or similar.

-- TODO[2025-06-10]: remove this column after data migration window closes
ALTER TABLE temp_users ADD COLUMN migration_flag INTEGER NOT NULL DEFAULT 0;

-- FIXME[2025-06-08]: revert this constraint relaxation after hotfix is verified
ALTER TABLE temp_users ALTER COLUMN legacy_field DROP NOT NULL;

-- ── Future annotations (far future — always OK) ───────────────────────────

-- TODO[2099-01-01]: revisit sharding strategy when user count exceeds 1B
CREATE INDEX idx_users_future ON temp_users(id);

-- FIXME[2099-12-31]: long-term schema tech debt, tracked in issue #8888
COMMENT ON TABLE temp_users IS 'Legacy table, see issue #8888';

-- HACK[2088-09-20]: workaround for ORM limitation, remove when ORM is replaced
CREATE OR REPLACE VIEW compat_users_view AS
SELECT id, username, NULL::TEXT AS deprecated_col FROM temp_users;

-- TODO[2099-01-01][frank]: frank owns the schema cleanup for the next major version
ALTER TABLE temp_users ADD COLUMN future_field TEXT;

-- ── Non-matching annotations (must be ignored by scanner) ────────────────

-- TODO: plain todo with no date bracket — must NOT be matched
-- FIXME: another undecorated one — must NOT be matched
-- NOTE[2020-01-01]: NOTE is not in the default tag list — must NOT be matched
-- TODO [2020-01-01]: space between tag and bracket — must NOT be matched

-- =============================================================================
-- Full schema: users, accounts, products, orders, audit, reporting
-- =============================================================================

-- -----------------------------------------------------------------------------
-- Users
-- -----------------------------------------------------------------------------

CREATE TABLE users (
    id              BIGSERIAL       PRIMARY KEY,
    username        VARCHAR(64)     NOT NULL UNIQUE,
    email           VARCHAR(320)    NOT NULL UNIQUE,
    password_hash   VARCHAR(255)    NOT NULL,
    role            VARCHAR(32)     NOT NULL DEFAULT 'viewer'
                                    CHECK (role IN ('admin', 'editor', 'viewer', 'guest')),
    status          VARCHAR(32)     NOT NULL DEFAULT 'pending'
                                    CHECK (status IN ('active', 'suspended', 'pending', 'deleted')),
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    last_login_at   TIMESTAMPTZ,
    deleted_at      TIMESTAMPTZ
);

CREATE INDEX idx_users_email          ON users (email);
CREATE INDEX idx_users_status         ON users (status);
CREATE INDEX idx_users_role_status    ON users (role, status);
CREATE INDEX idx_users_created_at     ON users (created_at DESC);

COMMENT ON TABLE  users IS 'Core user accounts';
COMMENT ON COLUMN users.role    IS 'Authorization role: admin > editor > viewer > guest';
COMMENT ON COLUMN users.status  IS 'Lifecycle state of the account';

-- -----------------------------------------------------------------------------
-- User profiles
-- -----------------------------------------------------------------------------

CREATE TABLE user_profiles (
    user_id         BIGINT          PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    display_name    VARCHAR(128),
    bio             TEXT,
    avatar_url      VARCHAR(2048),
    website         VARCHAR(2048),
    location        VARCHAR(255),
    timezone        VARCHAR(64)     NOT NULL DEFAULT 'UTC',
    language        VARCHAR(8)      NOT NULL DEFAULT 'en',
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------------
-- Addresses
-- -----------------------------------------------------------------------------

CREATE TABLE addresses (
    id              BIGSERIAL       PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label           VARCHAR(64)     NOT NULL DEFAULT 'home',
    street          VARCHAR(255)    NOT NULL,
    city            VARCHAR(128)    NOT NULL,
    state           VARCHAR(128),
    country         VARCHAR(64)     NOT NULL,
    postal_code     VARCHAR(20),
    is_primary      BOOLEAN         NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_addresses_user_id ON addresses (user_id);

-- -----------------------------------------------------------------------------
-- Products
-- -----------------------------------------------------------------------------

CREATE TABLE categories (
    id          SERIAL          PRIMARY KEY,
    parent_id   INTEGER         REFERENCES categories(id),
    slug        VARCHAR(128)    NOT NULL UNIQUE,
    name        VARCHAR(255)    NOT NULL,
    description TEXT,
    sort_order  INTEGER         NOT NULL DEFAULT 0
);

CREATE TABLE products (
    id              BIGSERIAL       PRIMARY KEY,
    sku             VARCHAR(64)     NOT NULL UNIQUE,
    category_id     INTEGER         REFERENCES categories(id),
    name            VARCHAR(255)    NOT NULL,
    description     TEXT,
    price_cents     INTEGER         NOT NULL CHECK (price_cents >= 0),
    currency        VARCHAR(3)      NOT NULL DEFAULT 'USD',
    stock_qty       INTEGER         NOT NULL DEFAULT 0 CHECK (stock_qty >= 0),
    weight_grams    INTEGER,
    is_active       BOOLEAN         NOT NULL DEFAULT TRUE,
    tags            TEXT[]          NOT NULL DEFAULT '{}',
    metadata        JSONB           NOT NULL DEFAULT '{}',
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_products_category       ON products (category_id);
CREATE INDEX idx_products_sku            ON products (sku);
CREATE INDEX idx_products_is_active      ON products (is_active) WHERE is_active = TRUE;
CREATE INDEX idx_products_tags           ON products USING GIN (tags);
CREATE INDEX idx_products_metadata       ON products USING GIN (metadata);

-- -----------------------------------------------------------------------------
-- Orders
-- -----------------------------------------------------------------------------

CREATE TABLE orders (
    id              BIGSERIAL       PRIMARY KEY,
    user_id         BIGINT          NOT NULL REFERENCES users(id),
    status          VARCHAR(32)     NOT NULL DEFAULT 'pending'
                                    CHECK (status IN (
                                        'pending', 'confirmed', 'processing',
                                        'shipped', 'delivered', 'cancelled', 'refunded'
                                    )),
    subtotal_cents  INTEGER         NOT NULL CHECK (subtotal_cents >= 0),
    tax_cents       INTEGER         NOT NULL DEFAULT 0,
    shipping_cents  INTEGER         NOT NULL DEFAULT 0,
    total_cents     INTEGER         NOT NULL,
    currency        VARCHAR(3)      NOT NULL DEFAULT 'USD',
    notes           TEXT,
    address_id      BIGINT          REFERENCES addresses(id),
    placed_at       TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    shipped_at      TIMESTAMPTZ,
    delivered_at    TIMESTAMPTZ,
    cancelled_at    TIMESTAMPTZ
);

CREATE INDEX idx_orders_user_id   ON orders (user_id);
CREATE INDEX idx_orders_status    ON orders (status);
CREATE INDEX idx_orders_placed_at ON orders (placed_at DESC);

CREATE TABLE order_items (
    id              BIGSERIAL   PRIMARY KEY,
    order_id        BIGINT      NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    product_id      BIGINT      NOT NULL REFERENCES products(id),
    quantity        INTEGER     NOT NULL CHECK (quantity > 0),
    unit_price_cents INTEGER    NOT NULL CHECK (unit_price_cents >= 0),
    discount_cents  INTEGER     NOT NULL DEFAULT 0,
    line_total_cents INTEGER    NOT NULL
);

CREATE INDEX idx_order_items_order   ON order_items (order_id);
CREATE INDEX idx_order_items_product ON order_items (product_id);

-- -----------------------------------------------------------------------------
-- Payments
-- -----------------------------------------------------------------------------

CREATE TABLE payments (
    id              BIGSERIAL       PRIMARY KEY,
    order_id        BIGINT          NOT NULL REFERENCES orders(id),
    provider        VARCHAR(64)     NOT NULL,
    provider_txn_id VARCHAR(255),
    status          VARCHAR(32)     NOT NULL DEFAULT 'pending'
                                    CHECK (status IN (
                                        'pending', 'authorized', 'captured',
                                        'failed', 'refunded', 'disputed'
                                    )),
    amount_cents    INTEGER         NOT NULL,
    currency        VARCHAR(3)      NOT NULL DEFAULT 'USD',
    raw_response    JSONB,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_payments_order_id        ON payments (order_id);
CREATE INDEX idx_payments_provider_txn_id ON payments (provider_txn_id);
CREATE INDEX idx_payments_status          ON payments (status);

-- -----------------------------------------------------------------------------
-- Inventory events
-- -----------------------------------------------------------------------------

CREATE TABLE inventory_events (
    id          BIGSERIAL       PRIMARY KEY,
    product_id  BIGINT          NOT NULL REFERENCES products(id),
    delta       INTEGER         NOT NULL,
    reason      VARCHAR(64)     NOT NULL
                                CHECK (reason IN (
                                    'purchase', 'return', 'restock', 'adjustment', 'write_off'
                                )),
    order_id    BIGINT          REFERENCES orders(id),
    actor_id    BIGINT          REFERENCES users(id),
    note        TEXT,
    occurred_at TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_inventory_product    ON inventory_events (product_id, occurred_at DESC);
CREATE INDEX idx_inventory_order      ON inventory_events (order_id);

-- -----------------------------------------------------------------------------
-- Audit log v2 (replaces audit_log_old)
-- -----------------------------------------------------------------------------

CREATE TABLE audit_v2 (
    id          BIGSERIAL       PRIMARY KEY,
    actor_id    BIGINT          REFERENCES users(id),
    action      VARCHAR(128)    NOT NULL,
    resource    VARCHAR(64)     NOT NULL,
    resource_id TEXT,
    before_json JSONB,
    after_json  JSONB,
    ip_address  INET,
    user_agent  TEXT,
    occurred_at TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_audit_actor       ON audit_v2 (actor_id, occurred_at DESC);
CREATE INDEX idx_audit_resource    ON audit_v2 (resource, resource_id);
CREATE INDEX idx_audit_action      ON audit_v2 (action);
CREATE INDEX idx_audit_occurred_at ON audit_v2 (occurred_at DESC);

-- -----------------------------------------------------------------------------
-- Notifications
-- -----------------------------------------------------------------------------

CREATE TABLE notifications (
    id          BIGSERIAL       PRIMARY KEY,
    user_id     BIGINT          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    type        VARCHAR(64)     NOT NULL,
    title       VARCHAR(255)    NOT NULL,
    body        TEXT,
    is_read     BOOLEAN         NOT NULL DEFAULT FALSE,
    metadata    JSONB           NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    read_at     TIMESTAMPTZ
);

CREATE INDEX idx_notifications_user_unread
    ON notifications (user_id, created_at DESC)
    WHERE is_read = FALSE;

-- -----------------------------------------------------------------------------
-- Feature flags
-- -----------------------------------------------------------------------------

CREATE TABLE feature_flags (
    id          SERIAL          PRIMARY KEY,
    name        VARCHAR(128)    NOT NULL UNIQUE,
    description TEXT,
    enabled     BOOLEAN         NOT NULL DEFAULT FALSE,
    rollout_pct NUMERIC(5,2)    NOT NULL DEFAULT 0.00
                                CHECK (rollout_pct BETWEEN 0 AND 100),
    config      JSONB           NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

-- -----------------------------------------------------------------------------
-- Sessions
-- -----------------------------------------------------------------------------

CREATE TABLE sessions (
    id          UUID            PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     BIGINT          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  VARCHAR(255)    NOT NULL UNIQUE,
    ip_address  INET,
    user_agent  TEXT,
    expires_at  TIMESTAMPTZ     NOT NULL,
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    last_seen   TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sessions_user_id    ON sessions (user_id);
CREATE INDEX idx_sessions_expires_at ON sessions (expires_at);

-- -----------------------------------------------------------------------------
-- Tags / labels (polymorphic)
-- -----------------------------------------------------------------------------

CREATE TABLE tags (
    id          SERIAL          PRIMARY KEY,
    name        VARCHAR(64)     NOT NULL UNIQUE,
    color       VARCHAR(7),
    created_at  TIMESTAMPTZ     NOT NULL DEFAULT NOW()
);

CREATE TABLE taggings (
    tag_id      INTEGER         NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    resource    VARCHAR(64)     NOT NULL,
    resource_id BIGINT          NOT NULL,
    PRIMARY KEY (tag_id, resource, resource_id)
);

CREATE INDEX idx_taggings_resource ON taggings (resource, resource_id);

-- -----------------------------------------------------------------------------
-- Reporting views
-- -----------------------------------------------------------------------------

CREATE OR REPLACE VIEW v_order_summary AS
SELECT
    o.id                            AS order_id,
    o.user_id,
    u.username,
    u.email,
    o.status,
    o.total_cents,
    o.currency,
    COUNT(oi.id)                    AS item_count,
    SUM(oi.quantity)                AS total_qty,
    o.placed_at
FROM orders       o
JOIN users        u  ON u.id = o.user_id
JOIN order_items  oi ON oi.order_id = o.id
GROUP BY o.id, o.user_id, u.username, u.email, o.status,
         o.total_cents, o.currency, o.placed_at;

CREATE OR REPLACE VIEW v_product_revenue AS
SELECT
    p.id                                AS product_id,
    p.sku,
    p.name,
    SUM(oi.quantity)                    AS units_sold,
    SUM(oi.line_total_cents)            AS revenue_cents,
    COUNT(DISTINCT oi.order_id)         AS order_count
FROM products       p
JOIN order_items    oi ON oi.product_id = p.id
JOIN orders         o  ON o.id = oi.order_id
WHERE o.status NOT IN ('cancelled', 'refunded')
GROUP BY p.id, p.sku, p.name;

CREATE OR REPLACE VIEW v_user_activity AS
SELECT
    u.id        AS user_id,
    u.username,
    u.email,
    u.status,
    u.last_login_at,
    COUNT(DISTINCT o.id)    AS order_count,
    COALESCE(SUM(o.total_cents), 0) AS lifetime_value_cents
FROM users  u
LEFT JOIN orders o ON o.user_id = u.id AND o.status NOT IN ('cancelled', 'refunded')
GROUP BY u.id, u.username, u.email, u.status, u.last_login_at;

-- -----------------------------------------------------------------------------
-- Stored functions / triggers
-- -----------------------------------------------------------------------------

-- Automatically keep updated_at current
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at := NOW();
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER trg_products_updated_at
    BEFORE UPDATE ON products
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER trg_payments_updated_at
    BEFORE UPDATE ON payments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Validate that order total matches line items
CREATE OR REPLACE FUNCTION validate_order_total(p_order_id BIGINT)
RETURNS BOOLEAN LANGUAGE plpgsql AS $$
DECLARE
    v_computed  INTEGER;
    v_stored    INTEGER;
BEGIN
    SELECT COALESCE(SUM(line_total_cents), 0)
    INTO v_computed
    FROM order_items
    WHERE order_id = p_order_id;

    SELECT subtotal_cents + tax_cents + shipping_cents
    INTO v_stored
    FROM orders
    WHERE id = p_order_id;

    RETURN v_computed = v_stored;
END;
$$;

-- Adjust inventory on order confirmation
CREATE OR REPLACE FUNCTION reserve_inventory(
    p_order_id   BIGINT,
    p_actor_id   BIGINT DEFAULT NULL
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT product_id, quantity
        FROM order_items
        WHERE order_id = p_order_id
    LOOP
        UPDATE products
        SET    stock_qty  = stock_qty - r.quantity,
               updated_at = NOW()
        WHERE  id = r.product_id
        AND    stock_qty  >= r.quantity;

        IF NOT FOUND THEN
            RAISE EXCEPTION 'Insufficient stock for product %', r.product_id;
        END IF;

        INSERT INTO inventory_events (product_id, delta, reason, order_id, actor_id)
        VALUES (r.product_id, -r.quantity, 'purchase', p_order_id, p_actor_id);
    END LOOP;
END;
$$;

-- Release inventory on cancellation
CREATE OR REPLACE FUNCTION release_inventory(
    p_order_id   BIGINT,
    p_actor_id   BIGINT DEFAULT NULL
) RETURNS VOID LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT product_id, quantity
        FROM order_items
        WHERE order_id = p_order_id
    LOOP
        UPDATE products
        SET    stock_qty  = stock_qty + r.quantity,
               updated_at = NOW()
        WHERE  id = r.product_id;

        INSERT INTO inventory_events (product_id, delta, reason, order_id, actor_id)
        VALUES (r.product_id, r.quantity, 'return', p_order_id, p_actor_id);
    END LOOP;
END;
$$;

-- Convenience: soft-delete a user
CREATE OR REPLACE FUNCTION soft_delete_user(p_user_id BIGINT)
RETURNS VOID LANGUAGE plpgsql AS $$
BEGIN
    UPDATE users
    SET    status     = 'deleted',
           deleted_at = NOW(),
           updated_at = NOW()
    WHERE  id = p_user_id;
END;
$$;

-- Pagination helper
CREATE OR REPLACE FUNCTION paginate_users(
    p_status    VARCHAR DEFAULT NULL,
    p_role      VARCHAR DEFAULT NULL,
    p_limit     INTEGER DEFAULT 20,
    p_offset    INTEGER DEFAULT 0
) RETURNS TABLE (
    id          BIGINT,
    username    VARCHAR,
    email       VARCHAR,
    role        VARCHAR,
    status      VARCHAR,
    created_at  TIMESTAMPTZ
) LANGUAGE sql STABLE AS $$
    SELECT id, username, email, role, status, created_at
    FROM   users
    WHERE  (p_status IS NULL OR status  = p_status)
    AND    (p_role   IS NULL OR role    = p_role)
    ORDER  BY created_at DESC
    LIMIT  p_limit
    OFFSET p_offset;
$$;

-- Revenue report by day
CREATE OR REPLACE FUNCTION daily_revenue(
    p_from  DATE,
    p_to    DATE
) RETURNS TABLE (
    day             DATE,
    order_count     BIGINT,
    revenue_cents   BIGINT
) LANGUAGE sql STABLE AS $$
    SELECT
        placed_at::DATE         AS day,
        COUNT(*)                AS order_count,
        SUM(total_cents)        AS revenue_cents
    FROM  orders
    WHERE placed_at::DATE BETWEEN p_from AND p_to
    AND   status NOT IN ('cancelled', 'refunded')
    GROUP BY placed_at::DATE
    ORDER BY day;
$$;

-- Cleanup expired sessions
CREATE OR REPLACE FUNCTION purge_expired_sessions()
RETURNS INTEGER LANGUAGE plpgsql AS $$
DECLARE
    v_count INTEGER;
BEGIN
    DELETE FROM sessions
    WHERE expires_at < NOW()
    RETURNING *
    INTO v_count;
    RETURN COALESCE(v_count, 0);
END;
$$;

-- -----------------------------------------------------------------------------
-- Seed data (reference / lookup tables)
-- -----------------------------------------------------------------------------

INSERT INTO categories (slug, name, description, sort_order) VALUES
    ('electronics',    'Electronics',          'Gadgets and devices',              10),
    ('clothing',       'Clothing',             'Apparel and accessories',          20),
    ('books',          'Books',                'Print and digital books',          30),
    ('home-garden',    'Home & Garden',        'Furniture, decor, and garden',     40),
    ('sports',         'Sports & Outdoors',    'Equipment and activewear',         50),
    ('toys',           'Toys & Games',         'For all ages',                     60),
    ('beauty',         'Beauty & Health',      'Personal care and wellness',       70),
    ('automotive',     'Automotive',           'Parts, accessories, tools',        80)
ON CONFLICT (slug) DO NOTHING;

INSERT INTO feature_flags (name, description, enabled, rollout_pct) VALUES
    ('new_checkout',        'Redesigned checkout flow',             FALSE,   0.00),
    ('ai_recommendations',  'ML-powered product recommendations',   FALSE,  10.00),
    ('dark_mode',           'Dark theme for the dashboard',         TRUE,  100.00),
    ('beta_search',         'Elasticsearch-backed search',          FALSE,  25.00),
    ('instant_payment',     'Skip authorization step for low-value orders', FALSE, 5.00)
ON CONFLICT (name) DO NOTHING;

INSERT INTO tags (name, color) VALUES
    ('vip',         '#FFD700'),
    ('wholesale',   '#4A90D9'),
    ('internal',    '#7B7B7B'),
    ('verified',    '#2ECC71'),
    ('flagged',     '#E74C3C')
ON CONFLICT (name) DO NOTHING;

-- -----------------------------------------------------------------------------
-- Materialized views for reporting
-- -----------------------------------------------------------------------------

CREATE MATERIALIZED VIEW mv_monthly_revenue AS
SELECT
    DATE_TRUNC('month', o.placed_at)   AS month,
    COUNT(DISTINCT o.id)               AS order_count,
    COUNT(DISTINCT o.user_id)          AS unique_buyers,
    SUM(o.total_cents)                 AS revenue_cents,
    AVG(o.total_cents)::INTEGER        AS avg_order_cents
FROM orders o
WHERE o.status NOT IN ('cancelled', 'refunded')
GROUP BY DATE_TRUNC('month', o.placed_at)
ORDER BY month;

CREATE UNIQUE INDEX ON mv_monthly_revenue (month);

CREATE MATERIALIZED VIEW mv_top_products AS
SELECT
    p.id            AS product_id,
    p.sku,
    p.name,
    c.name          AS category,
    SUM(oi.quantity) AS units_sold,
    SUM(oi.line_total_cents) AS revenue_cents
FROM products   p
JOIN order_items oi ON oi.product_id = p.id
JOIN orders      o  ON o.id = oi.order_id AND o.status NOT IN ('cancelled', 'refunded')
LEFT JOIN categories c ON c.id = p.category_id
GROUP BY p.id, p.sku, p.name, c.name
ORDER BY revenue_cents DESC;

CREATE UNIQUE INDEX ON mv_top_products (product_id);

-- Refresh policy (to be called by a scheduler)
CREATE OR REPLACE FUNCTION refresh_reporting_views()
RETURNS VOID LANGUAGE plpgsql AS $$
BEGIN
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_monthly_revenue;
    REFRESH MATERIALIZED VIEW CONCURRENTLY mv_top_products;
END;
$$;

-- -----------------------------------------------------------------------------
-- Row-level security
-- -----------------------------------------------------------------------------

ALTER TABLE users           ENABLE ROW LEVEL SECURITY;
ALTER TABLE orders          ENABLE ROW LEVEL SECURITY;
ALTER TABLE notifications   ENABLE ROW LEVEL SECURITY;

-- Admins see everything; regular users see only their own rows.
CREATE POLICY users_self_or_admin ON users
    USING (
        id = current_setting('app.current_user_id', TRUE)::BIGINT
        OR current_setting('app.current_role', TRUE) = 'admin'
    );

CREATE POLICY orders_self_or_admin ON orders
    USING (
        user_id = current_setting('app.current_user_id', TRUE)::BIGINT
        OR current_setting('app.current_role', TRUE) = 'admin'
    );

CREATE POLICY notifications_self ON notifications
    USING (user_id = current_setting('app.current_user_id', TRUE)::BIGINT);

-- -----------------------------------------------------------------------------
-- Maintenance / housekeeping queries (not executed — reference only)
-- -----------------------------------------------------------------------------

-- Find orders placed > 30 days ago still in 'pending'
-- SELECT id, user_id, placed_at FROM orders
-- WHERE status = 'pending'
--   AND placed_at < NOW() - INTERVAL '30 days';

-- Identify products with negative stock (data integrity check)
-- SELECT id, sku, name, stock_qty FROM products WHERE stock_qty < 0;

-- Top 10 users by lifetime order value
-- SELECT user_id, username, lifetime_value_cents
-- FROM v_user_activity
-- ORDER BY lifetime_value_cents DESC LIMIT 10;

-- Count active sessions per user
-- SELECT user_id, COUNT(*) AS active_sessions
-- FROM sessions WHERE expires_at > NOW()
-- GROUP BY user_id HAVING COUNT(*) > 5;
