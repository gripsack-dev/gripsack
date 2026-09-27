"""The token and counter increment share one SQLite transaction.

Only this database mutation is deduplicated. Running a shell command or sending
another request after commit would NOT inherit that atomicity.
"""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat


def intent_identity(value: str | None) -> str:
    if value is None or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ValueError("GRIPSACK_ACTIVATION_INTENT_ID must be the core-injected identity")
    return value


def increment(directory: Path, intent: str, counter: str) -> dict:
    intent_identity(intent)
    if not counter or len(counter.encode()) > 256:
        raise ValueError("counter name is empty or exceeds the example budget")
    directory.mkdir(mode=0o700, exist_ok=True)
    metadata = directory.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise ValueError("state must be an owned real directory")
    owned = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fchmod(owned, 0o700)
        os.fsync(owned)
        parent = os.open(directory.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        os.close(owned)
    database = directory / "counter.sqlite"
    descriptor = os.open(database, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise ValueError("counter database must be a regular file")
        os.fchmod(descriptor, 0o600)
    finally:
        os.close(descriptor)
    # The private directory and host integrity are assumptions of SQLite's
    # pathname reopen. This example does not claim race-free native confinement.
    payload = hashlib.sha256(json.dumps({"counter": counter, "delta": 1},
                                      sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    connection = sqlite3.connect(database, timeout=5, isolation_level=None)
    try:
        connection.execute("PRAGMA journal_mode=DELETE")
        connection.execute("PRAGMA synchronous=FULL")
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("CREATE TABLE IF NOT EXISTS receipts(intent TEXT PRIMARY KEY, payload TEXT NOT NULL)")
        connection.execute("CREATE TABLE IF NOT EXISTS counters(name TEXT PRIMARY KEY, value INTEGER NOT NULL CHECK(value>=0 AND value<=9223372036854775807))")
        prior = connection.execute("SELECT payload FROM receipts WHERE intent=?", (intent,)).fetchone()
        applied = prior is None
        if prior is not None and prior[0] != payload:
            raise ValueError("the same intent identity was reused for a different operation")
        if applied:
            current = connection.execute("SELECT value FROM counters WHERE name=?", (counter,)).fetchone()
            value = 0 if current is None else current[0]
            if value == 9223372036854775807:
                raise OverflowError("counter exhausted")
            connection.execute("INSERT INTO receipts VALUES(?,?)", (intent, payload))
            connection.execute("INSERT INTO counters VALUES(?,?) ON CONFLICT(name) DO UPDATE SET value=excluded.value", (counter, value + 1))
        value = connection.execute("SELECT value FROM counters WHERE name=?", (counter,)).fetchone()[0]
        connection.execute("COMMIT")
        return {"intent": intent, "applied": applied, "value": value}
    except BaseException:
        if connection.in_transaction:
            connection.execute("ROLLBACK")
        raise
    finally:
        connection.close()
