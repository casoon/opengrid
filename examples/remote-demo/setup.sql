-- Demo table for running the remote demo against PostgreSQL.
-- The same 50 rows as the conformance fixture: NULLs, NaN, exact decimals,
-- the NFC/NFD pair and a date on the year boundary.
DROP TABLE IF EXISTS opengrid_demo_orders;
CREATE TABLE opengrid_demo_orders (
  "id" bigint, "customer" text, "country" text,
  "amount" numeric(12,2), "qty" bigint, "ratio" double precision,
  "flag" boolean, "ordered_on" date, "created_at" timestamptz, "note" text
);
\copy opengrid_demo_orders FROM 'crates/opengrid-conformance/data/orders.csv' WITH (FORMAT csv, HEADER true, NULL '\N')
