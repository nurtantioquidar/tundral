-- PL/pgSQL the grammar only partly parses: routines are read from the text (§45 S2).
CREATE TABLE "public"."ledger" (
    id bigserial PRIMARY KEY,
    order_id integer REFERENCES orders(id),
    account_id integer REFERENCES "public"."accounts"
);

ALTER TABLE audit_events ADD COLUMN actor_id integer REFERENCES users(id);

CREATE OR REPLACE FUNCTION "public"."post_order"(p_order integer) RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM 1 FROM orders o JOIN customers c ON c.id = o.customer_id WHERE o.id = p_order;
    INSERT INTO "public"."ledger" (order_id) VALUES (p_order);
    PERFORM 1 FROM orders WHERE id = p_order;
END;
$$;

CREATE FUNCTION close_year(p_year integer) RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
    NULL;
    INSERT INTO ledger_archive SELECT * FROM "public"."ledger";
END;
$$; CREATE PROCEDURE purge_archive(p_year integer) LANGUAGE sql AS $$
    DELETE FROM ledger_archive WHERE EXTRACT(YEAR FROM archived_at) < p_year;
    INSERT INTO ledger_archive
    SELECT * FROM "public"."ledger" WHERE account_id IS DISTINCT FROM order_id;
    UPDATE ledger_archive SET note = SUBSTRING(lower(note) FROM note_prefixes);
    SELECT count(*) FROM ledger_archive;
$$;

CREATE FUNCTION reconcile() RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM "public"."ledger" l JOIN accounts a ON a.id = l.account_id;
    UPDATE accounts SET balance = 0 WHERE NOT EXISTS (SELECT 1 FROM "public"."ledger");
END;
$$;
