-- The schema behind postgres.graphify.json: `graphify extract <empty dir> --postgres <dsn>
-- --code-only` against a fresh `postgres:16` database named `shop` with this file loaded.
CREATE SCHEMA billing;

CREATE TABLE customers (
    id serial PRIMARY KEY,
    email text NOT NULL UNIQUE,
    referred_by integer REFERENCES customers (id)
);

CREATE TABLE products (
    sku text PRIMARY KEY,
    title text NOT NULL,
    price_micros bigint NOT NULL
);

CREATE TABLE orders (
    id serial PRIMARY KEY,
    customer_id integer NOT NULL REFERENCES customers (id),
    placed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE order_items (
    order_id integer NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    sku text NOT NULL,
    quantity integer NOT NULL,
    PRIMARY KEY (order_id, sku),
    CONSTRAINT order_items_sku_fkey FOREIGN KEY (sku) REFERENCES products (sku)
);

CREATE TABLE warehouse_stock (
    warehouse text NOT NULL,
    sku text NOT NULL,
    on_hand integer NOT NULL,
    PRIMARY KEY (warehouse, sku)
);

CREATE TABLE stock_moves (
    id serial PRIMARY KEY,
    warehouse text NOT NULL,
    sku text NOT NULL,
    delta integer NOT NULL,
    FOREIGN KEY (warehouse, sku) REFERENCES warehouse_stock (warehouse, sku)
);

CREATE TABLE billing.invoices (
    id serial PRIMARY KEY,
    order_id integer NOT NULL REFERENCES public.orders (id),
    total_micros bigint NOT NULL
);

CREATE VIEW order_totals AS
SELECT o.id AS order_id, sum(i.quantity * p.price_micros) AS total_micros
FROM orders o
JOIN order_items i ON i.order_id = o.id
JOIN products p ON p.sku = i.sku
GROUP BY o.id;

CREATE VIEW billing.open_invoices AS
SELECT id, order_id FROM billing.invoices WHERE total_micros > 0;

CREATE MATERIALIZED VIEW top_products AS
SELECT sku, count(*) AS n FROM order_items GROUP BY sku;

CREATE FUNCTION order_total(oid integer) RETURNS bigint
LANGUAGE sql STABLE AS $$
    SELECT total_micros FROM order_totals WHERE order_id = oid
$$;

CREATE FUNCTION touch_stock() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE warehouse_stock SET on_hand = on_hand + NEW.delta
    WHERE warehouse = NEW.warehouse AND sku = NEW.sku;
    RETURN NEW;
END
$$;

CREATE TRIGGER stock_moves_apply AFTER INSERT ON stock_moves
FOR EACH ROW EXECUTE FUNCTION touch_stock();

CREATE PROCEDURE billing.close_invoice(inv integer)
LANGUAGE sql AS $$
    UPDATE billing.invoices SET total_micros = 0 WHERE id = inv
$$;

CREATE FUNCTION add_one(integer) RETURNS integer LANGUAGE sql IMMUTABLE AS 'SELECT $1 + 1';
CREATE FUNCTION add_one(bigint) RETURNS bigint LANGUAGE sql IMMUTABLE AS 'SELECT $1 + 1';
