#!/usr/bin/env python3
# fake-services.py — ironmaint/fake-services:0.1
#
# Spec: doc/CONTAINER-IMAGES.md §4.6. Local stand-ins for the four
# privileged services IronMaint will eventually talk to:
#
#   Debian BTS       (IssueTrackerMutation, debian-bts provider)
#   Red Hat Bugzilla (IssueTrackerMutation, redhat-bugzilla provider)
#   Canonical push   (CanonicalRepositoryPush — Debian)
#   Koji             (RemoteBuildSubmission — Fedora)
#   Bodhi            (DistributionUpdateCreation — Fedora)
#
# Spec §4.6, constraint #2: "the fakes must fail in the same shape
# as the real services, including authentication failure, rate
# limiting and partial failure." So we DO NOT default to 200; we
# default to 401 without auth, 429 once an IP blows the rate
# limit, and 502 occasionally to simulate a flaky upstream.
#
# Spec §4.6, constraint #3: signing stays out of scope entirely.
# There is no /sign endpoint, no key file, no GPG plumbing.
#
# Constraints from this image's actual execution model (NOT spec):
#   - python3 stdlib only — no pip, no apt-get (base ships python3).
#     Single file, no dependencies, easy to audit.
#   - one process, one port (8080). Routes carry the service identity.
#   - all state is in-process. Restarting the container forgets
#     everything. That's intentional — a fake that remembers would
#     test as a real DB, which is out of scope.
#   - the auth token is read from FAKE_SERVICES_TOKEN (env). Default
#     is "test-token-1234" so the smoke can auth without -e.
#
# Spec §4.6 paragraph 1: "The fakes cover exactly those, and nothing
# else." The 6 operations below are the whole surface; if you need
# to add one, edit the spec first.

from __future__ import annotations

import json
import os
import random
import sys
import time
from collections import defaultdict, deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

# ---- endpoints (spec §4.6) ---------------------------------------------
#
#   1. GET  /issues/{provider}/{id}              read issue (debian-bts | redhat-bugzilla)
#   2. POST /issues/{provider}/{id}/mutate       IssueTrackerMutation
#   3. POST /release/debian/canonical-push       CanonicalRepositoryPush (Debian)
#   4. POST /release/fedora/koji-build           RemoteBuildSubmission (Fedora)
#   5. POST /release/fedora/bodhi-update         DistributionUpdateCreation (Fedora)
#   6. GET  /healthz                             liveness (NOT a privileged op; for
#                                                the Docker HEALTHCHECK)
#
# provider_id is opaque to core (IssueProviderId is a UUIDv7
# wrapper); the spec names "debian-bts" and "redhat-bugzilla" as
# the conventions. The fake accepts either string and treats them
# as distinct providers.

PROVIDERS = {"debian-bts", "redhat-bugzilla"}

PORT = int(os.environ.get("FAKE_SERVICES_PORT", "8080"))
TOKEN = os.environ.get("FAKE_SERVICES_TOKEN", "test-token-1234")
RATE_LIMIT = int(os.environ.get("FAKE_SERVICES_RATE_LIMIT", "60"))
WINDOW = 60.0  # seconds
PARTIAL_FAILURE_RATE = float(os.environ.get("FAKE_SERVICES_PARTIAL_FAILURE_RATE", "0.05"))


# ---- rate limiter (per-IP, in-memory) --------------------------------
# Spec §4.6 constraint #2. A naive deque-of-timestamps per IP is
# fine here — the fake isn't a real load balancer, and a request
# spike is exactly the failure mode we want to simulate.
_request_log: dict[str, deque[float]] = defaultdict(deque)


def is_rate_limited(ip: str) -> bool:
    now = time.monotonic()
    log = _request_log[ip]
    while log and now - log[0] > WINDOW:
        log.popleft()
    if len(log) >= RATE_LIMIT:
        return True
    log.append(now)
    return False


# ---- handler --------------------------------------------------------
class FakeServicesHandler(BaseHTTPRequestHandler):
    # Quieter logs (one line per request, prefixed by us).
    def log_message(self, format: str, *args: object) -> None:  # noqa: A002 (stdlib signature)
        sys.stderr.write(f"[fake-services] {self.address_string()} {format % args}\n")

    # BaseHTTPRequestHandler routes method-named verbs.
    def do_GET(self) -> None:  # noqa: N802 (stdlib signature)
        self._dispatch("GET")

    def do_POST(self) -> None:  # noqa: N802 (stdlib signature)
        self._dispatch("POST")

    def _dispatch(self, method: str) -> None:
        path = urlparse(self.path).path.rstrip("/") or "/"
        if path == "/healthz":
            return self._respond(200, {"ok": True, "service": "fake-services", "version": "0.1"})
        if path.startswith("/issues/"):
            return self._handle_issues(method, path)
        if path.startswith("/release/debian/canonical-push"):
            return self._handle_release(method, "debian", "CanonicalRepositoryPush", path)
        if path.startswith("/release/fedora/koji-build"):
            return self._handle_release(method, "fedora", "RemoteBuildSubmission", path)
        if path.startswith("/release/fedora/bodhi-update"):
            return self._handle_release(method, "fedora", "DistributionUpdateCreation", path)
        return self._respond(404, {"error": "no such endpoint", "method": method, "path": path})

    # ---- auth (spec §4.6 constraint #2 — must fail like a real svc) ---
    def _auth_ok(self) -> bool:
        h = self.headers.get("Authorization", "")
        if not h.startswith("Bearer "):
            return False
        # Constant-time-ish compare would be over-engineering for a fake;
        # equality is fine. If you change this, also change _respond.
        return h.split(" ", 1)[1] == TOKEN

    # ---- the 6 endpoint ops ------------------------------------------
    def _handle_issues(self, method: str, path: str) -> None:
        if not self._auth_ok():
            return self._respond(401, {"error": "missing or invalid Bearer token"})

        parts = path.split("/")
        # /issues/{provider}/{id}      -> ["", "issues", provider, id]
        # /issues/{provider}/{id}/mutate -> ["", "issues", provider, id, "mutate"]
        if method == "GET" and len(parts) == 4:
            provider, external_id = parts[2], parts[3]
        elif method == "POST" and len(parts) == 5 and parts[4] == "mutate":
            provider, external_id = parts[2], parts[3]
        else:
            return self._respond(404, {"error": "no such issues endpoint", "method": method, "path": path})

        if provider not in PROVIDERS:
            return self._respond(404, {"error": "unknown provider", "provider": provider,
                                       "allowed_services": sorted(PROVIDERS)})

        if is_rate_limited(self._ip()):
            return self._respond(429, {"error": "rate limit exceeded", "limit": RATE_LIMIT, "window_seconds": WINDOW})

        # Spec §4.6 constraint #2: a fake that always returns 200 makes
        # the privileged path look more reliable than reality. Inject
        # occasional partial failure.
        if random.random() < PARTIAL_FAILURE_RATE:
            return self._respond(502, {"error": "upstream service unavailable (simulated)"})

        if method == "GET":
            return self._respond(200, {
                "provider": provider,
                "external_id": external_id,
                "state": "open",
                "title": f"fake {provider} issue #{external_id}",
                "tags": ["fake", provider],
                "fetched_at": int(time.time()),
            })

        # POST /issues/.../mutate (IssueTrackerMutation)
        body = self._read_body()
        return self._respond(202, {
            "provider": provider,
            "external_id": external_id,
            "operation": "IssueTrackerMutation",
            "received": body,
            "accepted_at": int(time.time()),
            "fake_id": f"fake-{int(time.time() * 1000)}",
        })

    def _handle_release(self, method: str, distro: str, op: str, path: str) -> None:
        if method != "POST":
            return self._respond(405, {"error": "method not allowed", "allowed": ["POST"], "path": path})
        if not self._auth_ok():
            return self._respond(401, {"error": "missing or invalid Bearer token"})

        if is_rate_limited(self._ip()):
            return self._respond(429, {"error": "rate limit exceeded", "limit": RATE_LIMIT, "window_seconds": WINDOW})

        if random.random() < PARTIAL_FAILURE_RATE:
            return self._respond(502, {"error": "upstream service unavailable (simulated)"})

        body = self._read_body()
        return self._respond(202, {
            "distribution": distro,
            "operation": op,
            "received": body,
            "accepted_at": int(time.time()),
            "fake_id": f"fake-{int(time.time() * 1000)}",
        })

    # ---- response / body helpers ------------------------------------
    def _respond(self, status: int, body: dict[str, object]) -> None:
        payload = json.dumps(body, sort_keys=True).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        # Discourage accidental real-credential use. Anyone using this
        # in front of the real services needs to read the spec first.
        self.send_header("X-Fake-Services", "1")
        self.end_headers()
        self.wfile.write(payload)

    def _read_body(self) -> dict[str, object]:
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        if length <= 0:
            return {}
        raw = self.rfile.read(length)
        try:
            data = json.loads(raw)
            return data if isinstance(data, dict) else {"_value": data}
        except json.JSONDecodeError:
            return {"_raw": raw.decode("utf-8", errors="replace")}

    def _ip(self) -> str:
        # x-forwarded-for is fine for a fake; a real service would
        # care, but this one runs inside a container.
        return self.headers.get("X-Forwarded-For", self.client_address[0]).split(",")[0].strip()


def main() -> None:
    server = ThreadingHTTPServer(("0.0.0.0", PORT), FakeServicesHandler)
    sys.stderr.write(f"[fake-services] listening on 0.0.0.0:{PORT} token={TOKEN!r} "
                     f"rate_limit={RATE_LIMIT}/{WINDOW}s partial_failure={PARTIAL_FAILURE_RATE}\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        sys.stderr.write("[fake-services] shutting down\n")
        server.server_close()


if __name__ == "__main__":
    main()