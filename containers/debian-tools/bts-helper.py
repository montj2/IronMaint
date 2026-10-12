#!/usr/bin/env python3
# bts-helper.py — Phase 1 BTS read-only transport.
#
# This is the RED-state stub for PR 1E.1 (PHASE-1.md §23, §27).
# The real implementation is a thin shim around the
# `python3-debianbts` package (the SOAP client) that emits
# canonical JSON to stdout, so the executor's normalizer
# doesn't have to know about the SOAP wire format.
#
# This stub does the minimum that the smoke test can detect:
# it parses argv, recognises `--self-test <source-package>`,
# and exits 1 with a `not yet implemented` message. The
# smoke assertion `bts --self-test example` therefore fails
# in RED and passes in GREEN.
#
# Invocation (will be real in GREEN):
#   bts --self-test <source-package>   # canonical JSON, no network
#   bts <source-package>               # live `debianbts.get_bugs`
#
# The wire format is the placeholder for 1E.2 — the
# `debian.bts_snapshot_v1` report kind and the field set
# are defined when 1E.2's `DebianIssueReportV1` schema lands.
# For 1E.1, the helper's job is the transport plumbing:
# emit valid JSON on stdout, propagate stderr, exit 0 on
# success, exit non-zero on any failure.

import sys


def main(argv):
    if len(argv) < 2:
        sys.stderr.write(
            "bts: usage: bts [--self-test] <source-package>\n"
        )
        return 2

    if argv[1] == "--self-test":
        # The self-test mode is what the smoke test exercises.
        # The real implementation (1E.1 GREEN) emits a
        # hard-coded canonical-JSON fixture; the network path
        # is opt-in via the non-self-test mode and is gated
        # by 1E.2's fake-BTS hermetic integration test.
        if len(argv) < 3:
            sys.stderr.write(
                "bts: --self-test requires a source package name\n"
            )
            return 2
        sys.stderr.write(
            "bts: not yet implemented: --self-test mode "
            "(1E.1 GREEN lands the canonical-JSON fixture)\n"
        )
        return 1

    # Live mode would call `debianbts.get_bugs(argv[1])` and
    # emit the canonical JSON report. 1E.2's fake-BTS hermetic
    # integration test is the verification surface; the live
    # network path is opt-in and not gated by the §97 set.
    sys.stderr.write(
        "bts: not yet implemented: live network mode "
        "(1E.1 GREEN lands the python3-debianbts shim, "
        "1E.2 lands the fake-BTS hermetic test)\n"
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
