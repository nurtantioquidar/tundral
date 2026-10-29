--
-- PostgreSQL database dump
--

CREATE SCHEMA billing;

CREATE TABLE billing.invoices (
    id integer NOT NULL,
    order_id integer NOT NULL,
    total_micros bigint DEFAULT 0 NOT NULL
);

CREATE TABLE public.orders_archive (
    id integer NOT NULL,
    customer_id integer
);

CREATE SEQUENCE billing.invoices_id_seq START WITH 1;

ALTER TABLE ONLY billing.invoices
    ADD CONSTRAINT invoices_pkey PRIMARY KEY (id);

ALTER TABLE ONLY billing.invoices
    ADD CONSTRAINT invoices_order_id_fkey FOREIGN KEY (order_id) REFERENCES public.orders(id);

ALTER TABLE ONLY public.orders_archive
    ADD CONSTRAINT orders_archive_customer_fkey FOREIGN KEY (customer_id) REFERENCES customers(id);

CREATE INDEX invoices_order_idx ON billing.invoices USING btree (order_id);

GRANT SELECT ON billing.invoices TO reporting;
