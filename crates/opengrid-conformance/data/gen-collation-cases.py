#!/usr/bin/env python3
"""Writes `collation-cases/` (rule S4a, issue #391) with the orders PostgreSQL gives.

The expected rows are not the engine's own answer: a PostgreSQL with ICU sorts
`collation.csv` by its predefined collations, and the engine and SQLite have to
agree with that. Run it from this crate's directory against a local server
(`psql -h localhost -d postgres`); the cases checked in were written by
PostgreSQL 16.15 with ICU 78.

A case with one sort key has no two rows that tie (S6 leaves ties open, and
PostgreSQL does not keep the input order); the others end in `id`.
"""
import csv, glob, json, os, subprocess

ROWS = {}
for row in csv.DictReader(open("data/collation.csv", encoding="utf-8")):
    ROWS[int(row["id"])] = {
        "id": int(row["id"]),
        "kind": row["kind"],
        "word": None if row["word"] == "\\N" else row["word"],
    }


def psql(sql):
    out = subprocess.run(
        ["psql", "-h", "localhost", "-d", "postgres", "-qAt",
         "-c", 'create temp table t(id bigint, kind text collate "C", word text collate "C")',
         "-c", "\\copy t from 'data/collation.csv' with (format csv, header true, null '\\N')",
         "-c", sql],
        capture_output=True, text=True)
    assert out.returncode == 0 and not out.stderr.strip(), out.stderr
    return [int(line) for line in out.stdout.split()]


def collate(name):
    return '"C"' if name == "binary" else f'"{name}-x-icu"'


def case(name, sort, where=None, filter=None, offset=None, limit=None):
    columns = ["id"] + [key["field"] for key in sort if key["field"] != "id"]
    query = {"source": "collation", "select": columns}
    if filter:
        query["filter"] = filter
    query["sort"] = sort
    if offset is not None:
        query["offset"] = offset
    if limit is not None:
        query["limit"] = limit
    order = ", ".join(
        "id asc" if key["field"] == "id" else
        f'{key["field"]} collate {collate(key.get("collation", "binary"))} '
        f'{key.get("direction", "asc")} nulls {key.get("nulls", "last")}'
        for key in sort)
    sql = "select id from t" + (f" where {where}" if where else "") + f" order by {order}"
    if limit is not None:
        sql += f" limit {limit}"
    if offset is not None:
        sql += f" offset {offset}"
    expected = [[ROWS[id][column] for column in columns] for id in psql(sql)]
    body = {"id": name, "rule": "S4a", "query": query,
            "expected": {"columns": columns, "rows": expected}, "ordered": True}
    with open(f"collation-cases/{name}.json", "w", encoding="utf-8") as out:
        out.write(json.dumps(body, ensure_ascii=False, indent=2) + "\n")


def word(locale, direction="asc", nulls="last"):
    return {"field": "word", "direction": direction, "nulls": nulls, "collation": locale}


ID = {"field": "id"}
KIND = lambda kind: {"where": f"kind = '{kind}'", "filter": {"field": "kind", "op": "eq", "value": kind}}
ACCENTS = {"where": "id in (17, 18, 19, 27, 28)",
           "filter": {"field": "id", "op": "in", "value": [17, 18, 19, 27, 28]}}

os.makedirs("collation-cases", exist_ok=True)
for old in glob.glob("collation-cases/*.json"):
    os.remove(old)

# One key: the engine's path for a single column.
case("s4a-german-umlauts-sort-with-their-letter", [word("de")], **KIND("a"))
case("s4a-swedish-a-ring-and-umlauts-after-z", [word("sv")], **KIND("a"))
case("s4a-turkish-dotless-i-before-i", [word("tr")], **KIND("b"))
case("s4a-the-root-puts-turkish-i-together", [word("und")], **KIND("b"))
case("s4a-greek-and-cyrillic-by-the-root", [word("und")], **KIND("c"))
case("s4a-a-region-is-a-locale", [word("de-AT")], **KIND("a"))
case("s4a-desc-reverses-the-values-not-the-null", [word("de", "desc")], **KIND("a"))
case("s4a-nulls-first", [word("de", "asc", "first")], **KIND("a"))
case("s4a-equal-by-the-collation-orders-by-bytes", [word("de")], **ACCENTS)
case("s4a-equal-by-the-collation-orders-by-bytes-desc", [word("de", "desc")], **ACCENTS)
# Several keys: the engine's path of encoded rows.
case("s4a-root-orders-letters-before-case", [word("und"), ID])
case("s4a-binary-stays-code-point-order", [word("binary"), ID])
case("s4a-a-script-is-a-locale", [word("sr-Latn"), ID])
case("s4a-desc-reverses-the-values-not-the-nulls", [word("de", "desc"), ID])
case("s4a-a-page-of-a-collated-order", [word("de"), ID], offset=10, limit=8)
case("s4a-a-locale-key-after-a-binary-key",
     [{"field": "kind", "direction": "desc"}, {"field": "word", "collation": "tr"}, ID])
case("s4a-two-locale-keys",
     [{"field": "kind", "collation": "de"},
      {"field": "word", "direction": "desc", "nulls": "first", "collation": "sv"}, ID])
case("s4a-equal-by-the-collation-orders-by-bytes-after-a-key",
     [{"field": "kind"}, word("de")], **ACCENTS)
