CREATE VIEW order_totals AS
SELECT o.id AS order_id, sum(i.quantity) AS items
FROM orders o
JOIN order_items i ON i.order_id = o.id
GROUP BY o.id;

CREATE OR REPLACE VIEW big_customers AS
WITH spend AS (
    SELECT o.customer_id, count(*) AS n
    FROM orders o
    GROUP BY o.customer_id
)
SELECT c.email, s.n
FROM spend s
LEFT JOIN customers c ON c.id = s.customer_id
WHERE s.n > (SELECT avg(items) FROM order_totals);

CREATE MATERIALIZED VIEW daily_orders AS
SELECT date_trunc('day', placed_at) AS day, count(*) FROM orders GROUP BY 1;

CREATE VIEW all_people AS
SELECT email FROM customers
UNION
SELECT email FROM staff;
