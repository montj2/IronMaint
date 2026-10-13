# syntax=docker/dockerfile:1.7
#
# ironmaint/fake-services:0.1 — local stand-ins for the four
# privileged services.
#
# Spec: doc/CONTAINER-IMAGES.md §4.6. Built despite the spec's
# "Phase 1 — specified, not built" caveat because the immediate
# follow-up work (D-09 producer + integration tests) needs the
# test surface to exist. The shape is exactly what the spec
# requires; the depth is shallow because the spec is explicit
# that nothing here should pretend to be production.
#
# Layer graph: derives FROM ironmaint/base:0.1 (uid 1000 ironmaint,
# tini, ca-certificates, python3). The fake is a Python 3 stdlib
# HTTP server — no pip, no apt, no Node. ~20MB final image.
#
# Spec §4.6 constraint #1: endpoints come from
# IssueCapability::provider_id and ReleaseCapability::publication_plan.
#   providers: debian-bts, redhat-bugzilla
#   ops:       CanonicalRepositoryPush (Debian),
#              IssueTrackerMutation (covers both providers),
#              RemoteBuildSubmission (Fedora),
#              DistributionUpdateCreation (Fedora)
#
# Spec §4.6 constraint #2: must fail in the same shape as real
# services (auth, rate limit, partial failure). The server
# defaults to 401 without Bearer auth and 429 once an IP blows
# the rate limit; PARTIAL_FAILURE_RATE injects a small percentage
# of 502s to exercise the upstream-flaky path. A fake that always
# returns 200 would be worse than no fake.
#
# Spec §4.6 constraint #3: signing stays out of scope entirely.
# No /sign endpoint, no key file, no GPG plumbing. Anything that
# asks this container to sign something gets a 404.

# ---- base ---------------------------------------------------------------
FROM ironmaint/base:0.1

# ---- defaults -----------------------------------------------------------
# FAKE_SERVICES_TOKEN defaults to test-token-1234 so the smoke can
# auth without -e. A developer pointing ironclaw at this fake will
# pass -e FAKE_SERVICES_TOKEN=<their own>. The token is a bearer
# credential but lives inside compose and never leaves the developer's
# machine — it's a placeholder for whatever the integration test
# surface wires up later.
#
# FAKE_SERVICES_PORT (default 8080) — one process, one port. Routes
# carry the service identity.
#
# FAKE_SERVICES_RATE_LIMIT (default 60) — per-IP requests per minute.
# Above this, the fake returns 429.
#
# FAKE_SERVICES_PARTIAL_FAILURE_RATE (default 0.05) — probability of a
# simulated upstream outage (502). Set to 0.0 to make the fake
# deterministic for tests; 0.05 makes it realistic.
ENV FAKE_SERVICES_TOKEN=test-token-1234 \
    FAKE_SERVICES_PORT=8080 \
    FAKE_SERVICES_RATE_LIMIT=60 \
    FAKE_SERVICES_PARTIAL_FAILURE_RATE=0.05

# ---- fake server ---------------------------------------------------------
# The script is the entire server. python3-stdlib only (http.server,
# json, urllib.parse, collections.defaultdict, random, time). No
# pip, no apt. Spec §4.6 paragraph 1: "fakes cover exactly those,
# and nothing else" — this script is short enough to be read in one
# pass and audited end-to-end.
COPY --chmod=0755 fake-services.py /usr/local/bin/fake-services.py

# ---- smoke script --------------------------------------------------------
# Spec §11: a test that cannot fail is not a test. The smoke
# boots the server, hits each of the 6 endpoints in the spec with
# the right auth, and verifies the failure shapes (401, 429, 502).
# Boot, health, and reachability are not enough — the smoke must
# also exercise the route table, which is the only thing a real
# integration test will rely on.
COPY --chmod=0755 smoke.sh /usr/local/bin/smoke.sh

# ---- labels -------------------------------------------------------------
LABEL org.opencontainers.image.title="ironmaint/fake-services" \
      org.opencontainers.image.description="Local stand-ins for the four privileged services: Debian BTS, Red Hat Bugzilla, Canonical push, Koji, Bodhi. Python 3 stdlib HTTP server (no pip, no apt). Implements the 6 endpoint operations named in spec §4.6 (IssueTrackerMutation + provider reads for debian-bts and redhat-bugzilla; CanonicalRepositoryPush; RemoteBuildSubmission; DistributionUpdateCreation). Returns 401 on missing token, 429 above rate, 202 Accepted on auth+rate-OK, occasional 502 partial failure. Signing stays out of scope per spec §4.6 constraint #3." \
      org.opencontainers.image.source="https://github.com/montj2/IronMaint" \
      org.opencontainers.image.documentation="https://github.com/montj2/IronMaint/blob/develop/doc/CONTAINER-IMAGES.md" \
      org.opencontainers.image.licenses="(see repo LICENSE)" \
      org.opencontainers.image.vendor="IronMaint" \
      org.opencontainers.image.version="0.1" \
      org.opencontainers.image.authors="IronMaint maintainers"

# ---- network ------------------------------------------------------------
EXPOSE 8080

# ---- healthcheck ---------------------------------------------------------
# /healthz returns 200 when the server is up. curl is in base.
HEALTHCHECK --interval=10s --timeout=3s --retries=3 --start-period=2s \
    CMD curl -fsS --max-time 2 http://127.0.0.1:8080/healthz >/dev/null || exit 1

# ---- entrypoint ----------------------------------------------------------
# tini as PID 1 (signal story per spec §5.3). Exec-form CMD.
# The python process is foreground; tini forwards SIGTERM/SIGINT.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["python3", "/usr/local/bin/fake-services.py"]