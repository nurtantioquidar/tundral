CREATE PROCEDURE archive_orders(cutoff timestamptz)
LANGUAGE sql
AS $$
    INSERT INTO orders_archive
    SELECT * FROM orders WHERE placed_at < cutoff;
    DELETE FROM orders WHERE placed_at < cutoff;
$$;

CREATE OR REPLACE PROCEDURE billing.close_invoice(inv integer)
LANGUAGE plpgsql
AS $$
BEGIN
    UPDATE billing.invoices SET total_micros = (
        SELECT sum(amount) FROM billing.invoices WHERE id = inv
    ) WHERE id = inv;
END
$$;

CREATE PROCEDURE refresh_totals()
LANGUAGE sql
AS $$
    WITH recent AS (SELECT id FROM orders)
    UPDATE customers SET email = email
    FROM recent
    JOIN order_items ON true;
$$;
