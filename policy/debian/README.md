# policy/<distribution>/

Versioned distribution-policy data. Each subdirectory holds a
`<topic>.vN.json` data file bundled into the relevant adapter via
`include_str!`. **Do not add a `Cargo.toml` here** — the data is
consumed by adapter crates, not by this directory.
