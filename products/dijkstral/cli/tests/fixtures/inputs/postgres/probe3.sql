CREATE SCHEMA app;
CREATE TABLE app.t1 (id int PRIMARY KEY, v int);
CREATE TABLE t2 (id int PRIMARY KEY, v int);
CREATE TABLE t3 (id int PRIMARY KEY);
CREATE VIEW sys_view AS SELECT relname FROM pg_class;
CREATE FUNCTION writer() RETURNS void LANGUAGE sql AS $$
INSERT INTO t2 (id, v) SELECT id, v FROM app.t1;
DELETE FROM t3 WHERE id < 0;
UPDATE t2 SET v = 0 FROM t3 WHERE t3.id = t2.id;
$$;
CREATE AGGREGATE my_sum(int) (SFUNC = int4pl, STYPE = int);
CREATE FUNCTION plp() RETURNS int LANGUAGE plpgsql AS $$
DECLARE n int;
BEGIN
  SELECT count(*) INTO n FROM t1;
  SELECT count(*) INTO n FROM nosuch_table;
  SELECT count(*) INTO n FROM pg_catalog.pg_proc;
  SELECT count(*) INTO n FROM "App"."Odd";
  RETURN n;
END
$$;
CREATE VIEW sys_view2 AS SELECT relname FROM pg_class UNION SELECT relname FROM pg_catalog.pg_class;
