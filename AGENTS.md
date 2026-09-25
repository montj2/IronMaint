# AGENTS.md

Instructions for AI coding agents operating in this repository. Follow these rules without exception unless a human explicitly overrides them in a given session.

## System Access

You have full, unrestricted `sudo` access in this environment. Use it as needed to install packages, modify system configuration, manage services, or otherwise complete the task at hand. Do not ask for permission before running standard package installs or environment setup commands. Do not artificially restrict yourself to user-space workarounds when a straightforward `sudo` operation is the correct fix.

Exceptions requiring explicit confirmation before acting:
- Destructive operations outside the repository or its designated working directories (e.g., `rm -rf` on paths not under project control, disk partitioning, user/account changes).
- Anything that modifies host networking, firewall rules, or credentials/secrets stores.
- Anything irreversible with no rollback path.

When in doubt, narrate what you're about to run and why before running it.

## Branching Strategy: Git Flow

This repository uses Git Flow. Know the branch roles and stay inside them.

| Branch | Purpose | Branches from | Merges to |
|---|---|---|---|
| `main` | Production-ready, tagged releases only | — | — |
| `develop` | Integration branch, always deployable to staging | `main` | — |
| `feature/<short-desc>` | New work, one feature per branch | `develop` | `develop` |
| `release/<version>` | Release stabilization, no new features | `develop` | `main` + `develop` |
| `hotfix/<short-desc>` | Emergency production fixes | `main` | `main` + `develop` |

Rules:
- Never commit directly to `main` or `develop`.
- Branch names are lowercase, hyphenated, and descriptive: `feature/sbom-diff-cache`, not `feature/fix2`.
- One feature or fix per branch. Do not stack unrelated work on a single branch.
- Rebase feature branches on `develop` before opening a PR if `develop` has moved; do not merge `develop` into a stale feature branch as a substitute for rebasing unless the team's convention says otherwise.
- Delete branches after merge.

## Commit Standards: Atomic and Clean

Every commit must represent exactly one logical change. If you can't describe the commit in one sentence without using "and," split it.

Requirements:
- **Atomic**: one concern per commit. A refactor and a bug fix are two commits, not one, even if they touch the same file.
- **Buildable**: every commit should leave the repo in a working, testable state. No commits that break the build "temporarily."
- **No WIP commits**: don't commit half-finished work, debug prints, commented-out code, or `TODO: fix this` placeholders. If you need checkpoints, use `git stash` or local amends, then clean up before the commit lands in history.
- **No noise**: no unrelated formatting-only diffs bundled with functional changes. If a file needs reformatting, that's its own commit.
- **Message format**: imperative mood, 50-character summary line, blank line, wrapped body explaining *why* not *what* (the diff already shows what).

```
Fix SBOM merge dropping duplicate CPE entries

Deduplication was keying on package name only, which collapsed
distinct components sharing a name across ecosystems. Key on
(name, version, purl) instead.
```

- Squash fixup commits (`fix typo`, `address review comment`) before merge. Use `git rebase -i` or `git commit --fixup` + autosquash. History on `develop` and `main` should read as a clean sequence of intentional changes, not a session transcript.
- Never rewrite history on `main`, `develop`, or any shared branch other people have pulled. Force-push is confined to your own feature branches, and only with `--force-with-lease`, never bare `--force`.

## Push Policy

- Push atomic commits to your feature branch as you complete each logical unit of work. Don't hoard a giant unpushed local history.
- Before pushing, confirm the branch builds and tests pass locally.
- Push to `origin/<your-branch>`, not to `develop` or `main` directly.
- Open a PR against `develop` (or `main` for hotfixes) once the branch is ready for review. Include a summary of the change and any testing performed.
- Never force-push to `main`, `develop`, `release/*`, or any branch other contributors are actively working on.

## Pre-Commit Checklist

Run before every commit, not just before the PR:

1. Diff review: `git diff --staged` — confirm only intended changes are included.
2. Tests pass locally (update this line with the project's actual test command).
3. Linter/formatter clean, no suppressed warnings without justification.
4. No secrets, credentials, or `.env` files staged.
5. Commit message follows the format above.

## When to Stop and Ask

- The requested change conflicts with an existing architectural decision and you don't know why the decision was made.
- A task requires deleting or overwriting work that isn't yours (uncommitted changes from another session, another branch's history).
- You're about to touch CI/CD pipeline definitions, release tagging, or anything that affects other contributors' workflow.

Otherwise, proceed. You have the access and the mandate to do the work end to end: branch, implement, commit atomically, push, open the PR.
