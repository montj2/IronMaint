# Migrations

IronMaint's SQLite schema migrations live in this directory. They
are forward-only: each file's filename (`NNNN_*.sql`) is its
identity, and files are applied in lexicographic order on top of
an empty database.

## Layout

```text
migrations/
├── README.md
├── 0001_initial.sql     # commit 3 (this commit)
└── NNNN_*.sql           # future commits
```

## Verifier

The `cargo xtask verify-migrations` subcommand:

1. Reads every committed `NNNN_*.sql` filename.
2. Spins up a throwaway SQLite database (in-memory), applies each
   file in order, then introspects `sqlite_master` to record what
   schema objects exist.
3. Diffs the committed set against the applied set. Any drift
   (missing files, unexpected tables, order changes) fails the
   verifier.
4. With `--write`, regenerates the snapshot of applied schema
   objects so a deliberate change can be accepted.

The verifier does **not** apply migrations to the production
state directory — that is the daemon's responsibility at startup.

## Forward-only

Per PHASE-0B.md §30: "Migrations are forward-only in 0B." There
are no downgrade scripts. To roll back a schema change, write a
new migration that undoes it.
