#!/usr/bin/env python3
"""Generate orders.json, the JSON twin of orders.csv.

The CSV is the source of the data; this script only re-encodes the same cells in
the column-oriented wire format (plan/spezifikation/02-query-modell.md, E6/E13):
decimals, dates and timestamps as strings, non-finite floats as their wire
spelling, NULL as null. Cell texts are copied verbatim otherwise, so the twin
cannot drift from the CSV.

Run it after editing orders.csv:

    python3 crates/opengrid-conformance/data/gen-orders-json.py

The invariants below are asserted here; that the two encodings really produce
identical Arrow batches is asserted by
crates/opengrid-arrow-engine/tests/conformance_dataset.rs, which is the
authoritative guard.
"""

import json
from pathlib import Path

DATA = Path(__file__).resolve().parent
CSV = DATA / "orders.csv"
JSON = DATA / "orders.json"

# name -> one of int64, utf8, decimal, float64, bool, date, timestamp
TYPES = [
    ("id", "int64"),
    ("customer", "utf8"),
    ("country", "utf8"),
    ("amount", "decimal"),
    ("qty", "int64"),
    ("ratio", "float64"),
    ("flag", "bool"),
    ("ordered_on", "date"),
    ("created_at", "timestamp"),
    ("note", "utf8"),
]
NULL = "\\N"
NON_FINITE = {"NaN", "Infinity", "-Infinity"}


def cell(text: str, kind: str):
    """The JSON form of one CSV cell."""
    if text == NULL:
        return None
    if kind == "int64":
        return int(text)
    if kind == "utf8":
        return text
    if kind == "decimal":
        return text
    if kind == "float64":
        return text if text in NON_FINITE else float(text)
    if kind == "bool":
        assert text in ("true", "false"), text
        return text == "true"
    if kind in ("date", "timestamp"):
        return text
    raise AssertionError(kind)


def main() -> None:
    lines = CSV.read_text(encoding="utf-8").splitlines()
    header = lines[0].split(",")
    assert header == [name for name, _ in TYPES], header
    records = [line.split(",") for line in lines[1:] if line]

    # --- invariants -------------------------------------------------------
    assert len(records) == 50, len(records)
    assert [int(record[0]) for record in records] == list(range(1, 51))
    assert all(len(record) == len(TYPES) for record in records)

    def column(name: str) -> list:
        index = header.index(name)
        return [record[index] for record in records]

    assert column("customer").count(NULL) == 4
    assert column("customer").count("") == 1
    assert column("country").count(NULL) == 7
    assert column("country").count("") == 3, "empty string is a value, not NULL (rule S14)"
    assert column("amount").count(NULL) == 3
    assert column("qty").count(NULL) == 2
    assert column("ratio").count(NULL) == 2
    assert column("ratio").count("NaN") == 1
    assert column("ratio").count("-0.0") == 1
    assert column("flag").count(NULL) == 2
    assert column("ordered_on").count(NULL) == 1
    assert column("created_at").count(NULL) == 2
    assert column("note").count(NULL) == 41
    assert column("note").count("") == 1
    assert column("note").count("abc") == 1

    # Two spellings of the same letter, kept apart on purpose (rule S13).
    accented = {
        int(record[0]): record[header.index("note")]
        for record in records
        if record[header.index("note")].encode() in (b"\xc3\xa9", b"e\xcc\x81")
    }
    assert accented == {45: "\u00e9", 46: "e\u0301"}, accented

    # --- write ------------------------------------------------------------
    rows = []
    for record in records:
        row = {}
        for (name, kind), text in zip(TYPES, record):
            row[name] = cell(text, kind)
        rows.append(row)

    body = "[\n"
    body += ",\n".join(
        "  " + json.dumps(row, ensure_ascii=False, separators=(", ", ": "))
        for row in rows
    )
    body += "\n]\n"
    JSON.write_text(body, encoding="utf-8")

    # --- check what was written ------------------------------------------
    written = json.loads(JSON.read_text(encoding="utf-8"))
    assert len(written) == 50
    assert all(list(row) == [name for name, _ in TYPES] for row in written)
    assert all(
        cell(text, kind) == row[name]
        for record, row in zip(records, written)
        for (name, kind), text in zip(TYPES, record)
    )
    print(f"wrote {JSON.relative_to(DATA.parent.parent.parent)}: {len(written)} rows, {JSON.stat().st_size} bytes")


if __name__ == "__main__":
    main()
