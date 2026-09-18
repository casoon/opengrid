-- Demo-Tabelle für die Remote-Demo gegen PostgreSQL (Plan-Punkt 26).
-- Dieselben 50 Zeilen wie die Conformance-Fixture: NULLs, NaN, exakte Decimals,
-- das NFC/NFD-Paar und ein Datum an der Jahresgrenze.
DROP TABLE IF EXISTS opengrid_demo_orders;
CREATE TABLE opengrid_demo_orders (
  "id" bigint, "customer" text, "country" text,
  "amount" numeric(12,2), "qty" bigint, "ratio" double precision,
  "flag" boolean, "ordered_on" date, "created_at" timestamptz, "note" text
);
\copy opengrid_demo_orders FROM 'crates/opengrid-conformance/data/orders.csv' WITH (FORMAT csv, HEADER true, NULL '\N')
