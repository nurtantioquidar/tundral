-- The grammar loses this whole function (PERFORM with a JOIN), so it is read from the text (S2).
CREATE FUNCTION nightly_maintenance() RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM orders o JOIN customers c ON c.id = o.customer_id;
    PERFORM 1 FROM orders WHERE id = 0;
END;
$$;
