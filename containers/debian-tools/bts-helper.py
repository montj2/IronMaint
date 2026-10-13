#!/usr/bin/env python3
# bts-helper.py — Phase 1 BTS read-only transport.
#
# PHASE-1.md §23, §27. A fixed-binary shim around
# `python3-debianbts` (the SOAP client shipped by Debian)
# that emits canonical JSON to stdout so the executor's
# normalizer doesn't have to know about the SOAP wire
# format.
#
# Two modes:
#
#   bts --self-test <source-package>
#     Emits a hard-coded canonical-JSON fixture (no network).
#     The smoke test exercises this mode. The fixture is
#     deterministic so a §97 run produces a byte-identical
#     report. The fixture's field set is the 1E.2 placeholder
#     — 1E.2's `DebianIssueReportV1` schema will pin the
#     exact report_kind discriminator and field names.
#
#   bts <source-package>
#     Calls `debianbts.get_bugs` against bugs.debian.org
#     and emits the same canonical JSON shape. 1E.2's
#     fake-BTS hermetic integration test is the verification
#     surface; the live network path is opt-in (not in the
#     §97 gate, which never touches the network).
#
# The wire format is:
#
#   {
#     "schema_version": 1,
#     "report_kind": "debian.bts_snapshot_v1",
#     "source_package": <str>,
#     "snapshot_at": <RFC3339 UTC string>,
#     "bugs": [ { "id": <int>, "severity": <str>,
#                  "status": <str>, "subject": <str>,
#                  "tags": [<str>...], "forwarded": <bool>,
#                  "patch_tagged": <bool> }, ... ],
#     "summary": { "open": <int>, "serious": <int>,
#                  "patch_tagged": <int>, "forwarded": <int> }
#   }
#
# The summary is computed from the bugs list (open count,
# serious count, patch-tagged count, forwarded count).
# The smoke test asserts the JSON is parseable and carries
# a `report_kind` field; the field set can grow in 1E.2
# without breaking the smoke.

import datetime
import json
import sys


# The `--self-test` mode emits a hard-coded fixture. The
# fixture is small (3 bugs) but covers all four summary
# dimensions and both bug states (open / done) so the
# normalizer's classification logic has something to work
# with. 1E.2's hermetic test will exercise the live path
# against the fake-BTS container.
SELF_TEST_BUGS = [
    {
        "id": 100001,
        "severity": "serious",
        "status": "open",
        "subject": "self-test fixture: serious open bug",
        "tags": ["patch"],
        "forwarded": False,
        "patch_tagged": True,
    },
    {
        "id": 100002,
        "severity": "normal",
        "status": "open",
        "subject": "self-test fixture: open bug, forwarded upstream",
        "tags": ["upstream"],
        "forwarded": True,
        "patch_tagged": False,
    },
    {
        "id": 100003,
        "severity": "minor",
        "status": "done",
        "subject": "self-test fixture: closed bug",
        "tags": [],
        "forwarded": False,
        "patch_tagged": False,
    },
]


def compute_summary(bugs):
    """Compute the §23 summary fields from a bugs list."""
    open_count = sum(1 for b in bugs if b.get("status") == "open")
    serious_count = sum(
        1 for b in bugs
        if b.get("status") == "open" and b.get("severity") == "serious"
    )
    patch_tagged_count = sum(1 for b in bugs if b.get("patch_tagged"))
    forwarded_count = sum(1 for b in bugs if b.get("forwarded"))
    return {
        "open": open_count,
        "serious": serious_count,
        "patch_tagged": patch_tagged_count,
        "forwarded": forwarded_count,
    }


def snapshot_at_now():
    """Return the current time as an RFC3339 UTC string.

    The format is `YYYY-MM-DDTHH:MM:SSZ` (no fractional
    seconds, no timezone offset other than Z) so the
    wire format is stable and easy to compare across
    runs. 1E.2's hermetic test will pin the timestamp
    to a fixture-controlled value to make assertions
    byte-identical; 1E.1's self-test accepts "now"
    because the smoke only checks parseability and
    the presence of `report_kind`.
    """
    now = datetime.datetime.now(datetime.timezone.utc)
    return now.strftime("%Y-%m-%dT%H:%M:%SZ")


def build_report(source_package, bugs):
    """Build the canonical-JSON report."""
    return {
        "schema_version": 1,
        "report_kind": "debian.bts_snapshot_v1",
        "source_package": source_package,
        "snapshot_at": snapshot_at_now(),
        "bugs": bugs,
        "summary": compute_summary(bugs),
    }


def emit_canonical_json(report):
    """Write the report to stdout as canonical JSON.

    The wire format is `json.dumps(..., sort_keys=True,
    separators=(",", ":"))` — no whitespace, sorted keys,
    RFC 8259. The normalizer expects canonical JSON; the
    test surface is `python3 -c 'import json,sys;
    json.load(sys.stdin)'`, which accepts any RFC 8259
    JSON. The `sort_keys=True` is the canonical form so
    the §23 contract is unambiguous.
    """
    sys.stdout.write(
        json.dumps(
            report,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        )
    )
    sys.stdout.write("\n")
    sys.stdout.flush()


def self_test_mode(source_package):
    """The --self-test mode: emit a hard-coded fixture.

    The fixture's source_package is the CLI argument so
    the smoke assertion `bts --self-test example` produces
    a report with `source_package == "example"`. The
    fixture is deterministic; only `snapshot_at` changes
    between runs (the smoke doesn't check it).
    """
    report = build_report(source_package, SELF_TEST_BUGS)
    emit_canonical_json(report)
    return 0


def live_mode(source_package):
    """The live network mode: call python3-debianbts.

    The actual SOAP call lives behind the import. The
    import is the only network-dependent line; if the
    python3-debianbts module is missing (e.g. the
    operator built the image without the apt package),
    the import fails with a clear error and the shim
    exits non-zero. The 1E.2 fake-BTS hermetic test is
    the verification surface for the live path.
    """
    try:
        from debianbts import get_bugs  # type: ignore
    except ImportError as e:
        sys.stderr.write(
            "bts: python3-debianbts is not installed: "
            f"{e}\n"
        )
        return 1
    try:
        raw = get_bugs(source_package)
    except Exception as e:  # noqa: BLE001 — the SOAP client
        # raises a variety of exceptions; the shim
        # normalises them to a single non-zero exit
        # with the diagnostic on stderr. 1E.2's
        # `BTS Failure Semantics` section (§24) is
        # the place to add structured error handling
        # (DNS / TLS / timeout / SOAP / malformed).
        sys.stderr.write(f"bts: get_bugs failed: {e}\n")
        return 1
    # The python3-debianbts return shape is a list of bug
    # dicts with at least `id`, `severity`, `status`, `subject`.
    # The shim normalises to the canonical report shape; missing
    # fields become empty/None so the summary can still be
    # computed.
    bugs = []
    for raw_bug in raw or []:
        severity = raw_bug.get("severity", "normal")
        status = raw_bug.get("status", "open")
        tags = list(raw_bug.get("tags", []))
        subject = raw_bug.get("subject", "")
        bug_id = raw_bug.get("id", 0)
        bugs.append({
            "id": bug_id,
            "severity": severity,
            "status": status,
            "subject": subject,
            "tags": tags,
            "forwarded": "upstream" in tags or "forwarded" in tags,
            "patch_tagged": "patch" in tags,
        })
    report = build_report(source_package, bugs)
    emit_canonical_json(report)
    return 0


def main(argv):
    if len(argv) < 2:
        sys.stderr.write(
            "bts: usage: bts [--self-test] <source-package>\n"
        )
        return 2

    if argv[1] == "--self-test":
        if len(argv) < 3:
            sys.stderr.write(
                "bts: --self-test requires a source package name\n"
            )
            return 2
        return self_test_mode(argv[2])

    return live_mode(argv[1])


if __name__ == "__main__":
    sys.exit(main(sys.argv))
