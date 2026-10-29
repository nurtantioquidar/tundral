CREATE FUNCTION order_total(oid integer) RETURNS bigint
LANGUAGE sql STABLE AS $$
    SELECT items FROM order_totals WHERE order_id = oid
$$;

CREATE OR REPLACE FUNCTION touch_order() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    SELECT 1 FROM orders WHERE id = NEW.order_id;
    RETURN NEW;
END
$$;

CREATE TRIGGER order_items_touch
    AFTER INSERT ON order_items
    FOR EACH ROW EXECUTE FUNCTION touch_order();

CREATE TRIGGER invoices_close
    BEFORE UPDATE ON billing.invoices
    FOR EACH ROW EXECUTE FUNCTION billing.close_invoice();
