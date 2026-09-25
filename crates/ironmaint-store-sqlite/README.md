# `ironmaint-store-sqlite`

SQLite implementation of [`IronMaintStore`](https://github.com/montj2/IronMaint/tree/main/crates/ironmaint-store).

## Runtime SQL only

This crate uses `sqlx::query` and `sqlx::query_as::<_, T>` (the
runtime SQL API). It deliberately avoids the compile-time
`sqlx::query!` / `sqlx::query_as!` macros because those require a
live database at build time, which 0B's CI does not have. Every
query is constructed and parameterised at runtime; serde
serialisation of payloads (events, evidence, gate definitions,
etc.) is done in Rust, not in SQL.

If a future contributor wants compile-time-checked queries, set
`SQLX_OFFLINE=true` and use `cargo sqlx prepare` to commit a
`.sqlx/` directory, then enable the macro syntax. That's deferred
until 0B+.

## PRAGMAs

`open()` applies the standard durability/recovery PRAGMAs from
PHASE-0B.md §30:

| PRAGMA | Value | Why |
|---|---|---|
| `journal_mode` | `WAL` | Concurrent readers + single writer |
| `synchronous`  | `NORMAL` | Crash-safe with WAL; faster than `FULL` |
| `foreign_keys` | `ON` | Enforce schema-level invariants |
| `busy_timeout` | configurable (default 5000 ms) | Surface contention as an error |

## Lock acquisition

`open()` also acquires an `fs2::FileExt::try_lock_exclusive` on
`<state-dir>/ironmaint.lock`. The lock is held for the lifetime of
the returned `SqliteStore`. A second call to `open()` against the
same state directory fails with `StoreError::Conflict` rather than
blocking — so two daemons cannot silently corrupt each other's
state (PHASE-0B.md §10).

## Migrations

The committed migrations live in `../../migrations/`. The daemon
applies them at startup if `schema_migrations` does not contain a
row for the filename. The verifier (`cargo xtask
verify-migrations`) confirms that the committed files match what
an empty database would produce.
