-- Initial schema. Orders reference customers; items reference products.
CREATE TABLE IF NOT EXISTS customers (
    id SERIAL PRIMARY KEY,
    referred_by INT REFERENCES customers (id),
    email TEXT NOT NULL UNIQUE
);

CREATE TABLE products (
    id SERIAL PRIMARY KEY,
    sku TEXT NOT NULL,
    supplier_id INT REFERENCES catalog.suppliers (id)
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    customer_id INT NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    placed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE order_items (
    order_id INT NOT NULL,
    product_id INT NOT NULL,
    quantity INT NOT NULL CHECK (quantity > 0),
    CONSTRAINT order_items_order_fk FOREIGN KEY (order_id) REFERENCES orders (id),
    FOREIGN KEY (product_id) REFERENCES products (id)
);
-- order_items once REFERENCES legacy_items too.

CREATE INDEX order_items_order_idx ON order_items (order_id);
CREATE UNIQUE INDEX products_sku_idx ON products (sku);
CREATE INDEX ON orders (placed_at);
