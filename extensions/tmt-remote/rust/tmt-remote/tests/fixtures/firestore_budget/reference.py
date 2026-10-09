#!/usr/bin/env python3
"""Independent reference for firestore_budget (does not use the Rust code).

Regenerate: python3 reference.py > vectors.json
Quota numbers: https://firebase.google.com/docs/firestore/quotas and /pricing, read 2026-10-09.
Pacific days come from the system tz database (zoneinfo), not from the rule the Rust code implements.
"""
import json
from datetime import datetime, timedelta, timezone
from zoneinfo import ZoneInfo

READS, WRITES = 50_000, 20_000
READ_LOOKUPS, WRITE_LOOKUPS = 2, 3
WARN, REFUSE = 70, 90
ACTIVE_S = 2 * 60 * 60

def append_row(n, w):
    rpa = WRITE_LOOKUPS + (n - 1) * (1 + READ_LOOKUPS)
    limit = min(READS // rpa, WRITES)
    share = limit // w
    return {
        "members": n, "writers": w,
        "readsPerAppend": rpa,
        # Reference only (not used by the guard): dependents cached, so listeners pay one read each.
        "optimisticReadsPerAppend": WRITE_LOOKUPS + (n - 1),
        "dailyAppendLimit": limit,
        "share": share,
        "warnAt": share * WARN // 100,
        "refuseAt": share * REFUSE // 100,
        "minFlushIntervalMs": -(-ACTIVE_S * 1000 // max(1, limit * WARN // 100)),
    }

models = [(1, 1), (2, 1), (2, 2), (3, 2), (5, 3), (5, 5), (10, 4), (10, 10), (25, 5), (25, 25), (100, 10), (10_000, 10_000)]
append = [append_row(n, w) for n, w in models]

assess = []
for row in append:
    pts = sorted({0, row["warnAt"] - 1, row["warnAt"], row["refuseAt"] - 1, row["refuseAt"], row["refuseAt"] + 1})
    for used in pts:
        if used < 0:
            continue
        verdict = "refuse" if used >= row["refuseAt"] else "warn" if used >= row["warnAt"] else "ok"
        assess.append({"members": row["members"], "writers": row["writers"], "used": used, "verdict": verdict})

pacific = []
la = ZoneInfo("America/Los_Angeles")
instants = [
    "2026-01-15T12:00:00", "2026-07-15T12:00:00",
    "2026-03-08T09:59:59", "2026-03-08T10:00:00", "2026-03-09T06:59:59", "2026-03-09T07:00:00",
    "2026-11-01T08:59:59", "2026-11-01T09:00:00", "2026-11-02T07:59:59", "2026-11-02T08:00:00",
    "2026-12-31T07:59:59", "2026-12-31T08:00:00", "2027-03-14T09:59:59", "2027-03-14T10:00:00",
    "2028-02-29T12:00:00", "2028-11-05T08:59:59", "2028-11-05T09:00:00", "2070-06-30T00:00:00",
]
epoch = datetime(1970, 1, 1, tzinfo=timezone.utc)
for text in instants:
    utc = datetime.fromisoformat(text).replace(tzinfo=timezone.utc)
    local = utc.astimezone(la)
    day = (local.date() - datetime(1970, 1, 1).date()).days
    # Rebuild the next midnight from the date so a 23/25-hour day cannot skew it.
    nd = local.date() + timedelta(days=1)
    reset = datetime(nd.year, nd.month, nd.day, tzinfo=la).astimezone(timezone.utc)
    pacific.append({
        "nowMs": int((utc - epoch).total_seconds() * 1000),
        "day": day,
        "resetAtMs": int((reset - epoch).total_seconds() * 1000),
    })

print(json.dumps({"append": append, "assess": assess, "pacific": pacific}, indent=1))
