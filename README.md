# IronMaint

Agentic Linux distribution package-maintenance framework.

The repository is currently in **Phase 0A** — implementing the
distribution-neutral domain model, state machine, evidence and policy
contracts, and the adapter API. No Debian- or Fedora-specific behavior, no
MCP servers, no IronClaw integration, no I/O. See
[`doc/phases/PHASE-0A.md`](doc/phases/PHASE-0A.md) for the implementation
spec and [`doc/IronMaint-Universal.md`](doc/IronMaint-Universal.md) for the
master architecture.

## Workspace layout

```
.
├── Cargo.toml                    # workspace manifest, lints, dependencies
├── rust-toolchain.toml           # pinned channel + components
├── rustfmt.toml, clippy.toml     # formatter and linter config
├── deny.toml                     # cargo-deny license + advisory policy
├── xtask/                        # cargo xtask verify-architecture, verify-schemas
├── crates/
│   ├── ironmaint-core/           # value types, IDs, candidates (bottom of graph)
│   ├── ironmaint-evidence/       # MaintenanceEvidence + gate evaluation
│   ├── ironmaint-policy/         # Authority + MaintenanceObligation
│   ├── ironmaint-state/          # JobState + TransitionEngine
│   ├── ironmaint-adapter-api/    # DistributionAdapter trait + Port traits
│   └── ironmaint-testkit/        # fakes, fixtures, conformance suite
├── adapters/
│   ├── debian-stub/              # DistributionAdapter impl for Debian
│   └── fedora-stub/              # DistributionAdapter impl for Fedora
└── doc/
    ├── IronMaint-Universal.md    # master architecture spec
    ├── phases/PHASE-0A.md        # Phase 0A implementation spec
    ├── vendor/ironclaw/          # vendored reference orchestrator
    └── schemas/                  # committed JSON schema snapshots
```

## Building

```sh
cargo check --workspace
cargo build --workspace
```

## Linting

```sh
cargo fmt --check --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Testing

```sh
cargo test --workspace
```

## Architecture verification

```sh
cargo xtask verify-architecture
```

Asserts the dependency graph from `doc/phases/PHASE-0A.md` §5 and §71:
adapters → adapter-api → {core, evidence, policy}; core is the bottom.

## License

MIT. See [`LICENSE`](LICENSE).
