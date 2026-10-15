CREATE TABLE zeta (id int PRIMARY KEY);
CREATE TABLE alpha (id int PRIMARY KEY);
CREATE TABLE "Mixed Case" (id int PRIMARY KEY);
CREATE TABLE src (
    id int PRIMARY KEY,
    z int,
    a int,
    m int,
    CONSTRAINT a_first_fkey FOREIGN KEY (z) REFERENCES zeta (id),
    CONSTRAINT z_last_fkey FOREIGN KEY (a) REFERENCES alpha (id),
    CONSTRAINT m_mid_fkey FOREIGN KEY (m) REFERENCES "Mixed Case" (id)
);
CREATE TABLE events (id int, at date NOT NULL) PARTITION BY RANGE (at);
CREATE TABLE events_2026 PARTITION OF events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
CREATE VIEW v_cte AS
WITH recent AS (SELECT * FROM events WHERE at > '2026-06-01')
SELECT r.id, (SELECT count(*) FROM alpha) AS n FROM recent r, zeta z WHERE z.id = r.id;
CREATE VIEW "View Two" AS SELECT id FROM "Mixed Case" UNION SELECT id FROM src;
CREATE FUNCTION joiner() RETURNS bigint LANGUAGE sql AS $$
SELECT count(*) FROM src s JOIN alpha a ON a.id = s.a LEFT JOIN zeta ON zeta.id = s.z
$$;
CREATE FUNCTION plain_c() RETURNS int LANGUAGE sql AS 'SELECT 1';
