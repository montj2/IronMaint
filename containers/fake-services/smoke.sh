#!/usr/bin/env bash
# smoke.sh — ironmaint/fake-services:0.1
# Runs INSIDE the container. Must exit non-zero on any failure.
# Spec §11: a test that cannot fail is not a test.
#
# Invocation patterns:
#   docker run --rm --platform linux/arm64 -p 8080:8080 \
#     ironmaint/fake-services:0.1 bash /usr/local/bin/smoke.sh
#   make -f containers/Makefile smoke-fake-services
#
# The image's default CMD runs the fake server in the foreground.
# When invoked as `… bash /usr/local/bin/smoke.sh`, no server is
# running yet, so this smoke starts a background instance on
# 127.0.0.1:8080, exercises each of the 6 spec endpoints + the
# auth/route failure shapes, then tears it down. The whole
# container exit code is the test's signal.

set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; cleanup; exit 1; }
ok()   { printf 'OK:   %s\n' "$*"; }
true

cleanup() {
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# ---- 1. PID 1 is tini (inherited from base) ------------------------------
# base sets tini as PID 1. If something clobbers the entrypoint later,
# this catches it.
[ "$(cat /proc/1/comm 2>/dev/null)" = "tini" ] \
    || fail "PID 1 is $(cat /proc/1/comm 2>/dev/null), expected tini"
ok "PID 1 is tini"

# ---- 2. uid 1000 ironmaint ---------------------------------------------
[ "$(id -u)" = "1000" ] || fail "running as uid $(id -u), expected 1000"
[ "$(id -un)" = "ironmaint" ] || fail "running as $(id -un), expected ironmaint"
ok "running as uid 1000 (ironmaint)"

# ---- 3. boot the server on a free port --------------------------------
# Use 8080 (the documented default). Bind to 127.0.0.1 so the smoke
# never sees a public bind — the IMAGE's healthcheck still binds
# 0.0.0.0 in normal operation.
PORT=8080
python3 /usr/local/bin/fake-services.py &
SERVER_PID=$!

# Wait for /healthz to come up. Don't tail the server log — we don't
# want a noisy smoke.
for _ in $(seq 1 50); do
  if curl -fsS --max-time 1 -o /dev/null "http://127.0.0.1:${PORT}/healthz"; then
    break
  fi
  sleep 0.1
done
if ! curl -fsS --max-time 1 -o /dev/null "http://127.0.0.1:${PORT}/healthz"; then
  fail "server did not come up on 127.0.0.1:${PORT} within 5s"
fi
ok "server is up on 127.0.0.1:${PORT}"

# ---- helpers -----------------------------------------------------------
# All curl invocations go through this so SIGPIPE / pipefail / -o /dev/null
# are handled the same way. Returns "STATUS BODY" (BODY empty on -o /dev/null).
# Without pipefail dance, the second `tr` would never run because
# head -n1 would EPIPE first; with dance, curl exits cleanly and we
# can read both the status line and the body without losing curl's
# exit code.
http_get() {
  # http_get STATUS_VAR BODY_VAR URL [extra curl args...]
  local status_var="$1" body_var="$2" url="$3"
  shift 3
  set +e
  set +o pipefail
  body=$(curl -sS --max-time 5 -w "\n%{http_code}" "$@" "$url" 2>&1)
  rc=$?
  set -o pipefail
  set -e
  if [ "$rc" -ne 0 ]; then
    fail "curl to $url failed (exit $rc) — TLS / network issue?"
  fi
  # Body and status separated by the trailing newline curl wrote.
  status=$(printf '%s' "$body" | tail -n1)
  body_only=$(printf '%s' "$body" | sed '$d')
  printf -v "$status_var" '%s' "$status"
  printf -v "$body_var" '%s' "$body_only"
}

TOKEN="test-token-1234"

# ---- 4. unauth → 401 on a privileged endpoint ---------------------------
# Spec §4.6 constraint #2: "must fail in the same shape as the real
# services, including authentication failure". The server's default
# contract is "no Bearer token → 401".
http_get status body "http://127.0.0.1:${PORT}/issues/debian-bts/123" || true
[ "$status" = "401" ] || fail "unauth GET /issues/debian-bts/123 returned $status, expected 401"
ok "unauth GET /issues/debian-bts/123 returns 401"

# ---- 5. auth → 200 on GET /issues/{provider}/{id} (both providers) -----
# Operations #1 of the 6 (spec §4.6 paragraph "When it is built,
# three constraints"). Two provider_ids to exercise the routing.
http_get status body "http://127.0.0.1:${PORT}/issues/debian-bts/123" \
    -H "Authorization: Bearer ${TOKEN}" || true
[ "$status" = "200" ] || fail "GET /issues/debian-bts/123 (auth) returned $status, expected 200"
case "$body" in
  *"\"provider\": \"debian-bts\""*) ok "GET debian-bts/123 returns provider-tagged body" ;;
  *) fail "GET debian-bts/123 body did not include provider=debian-bts: $body" ;;
esac

http_get status body "http://127.0.0.1:${PORT}/issues/redhat-bugzilla/456" \
    -H "Authorization: Bearer ${TOKEN}" || true
[ "$status" = "200" ] || fail "GET /issues/redhat-bugzilla/456 (auth) returned $status, expected 200"
case "$body" in
  *"\"provider\": \"redhat-bugzilla\""*) ok "GET redhat-bugzilla/456 returns provider-tagged body" ;;
  *) fail "GET redhat-bugzilla/456 body did not include provider=redhat-bugzilla: $body" ;;
esac

# ---- 6. auth → 202 on IssueTrackerMutation (POST /issues/.../mutate) --
# Operation #2.
http_get status body "http://127.0.0.1:${PORT}/issues/debian-bts/123/mutate" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    --data '{"kind":"comment","text":"smoke comment"}' || true
[ "$status" = "202" ] || fail "POST /issues/debian-bts/123/mutate returned $status, expected 202"
case "$body" in
  *"\"operation\": \"IssueTrackerMutation\""*) ok "POST issue mutate returns IssueTrackerMutation-tagged body" ;;
  *) fail "POST issue mutate body did not include IssueTrackerMutation: $body" ;;
esac

# ---- 7. auth → 202 on CanonicalRepositoryPush (POST /release/debian/...) --
# Operation #3.
http_get status body "http://127.0.0.1:${PORT}/release/debian/canonical-push" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    --data '{"package":"libfoo","version":"1.2.3-1"}' || true
[ "$status" = "202" ] || fail "POST /release/debian/canonical-push returned $status, expected 202"
case "$body" in
  *"\"operation\": \"CanonicalRepositoryPush\""*) ok "POST canonical-push returns CanonicalRepositoryPush-tagged body" ;;
  *) fail "POST canonical-push body did not include CanonicalRepositoryPush: $body" ;;
esac

# ---- 8. auth → 202 on RemoteBuildSubmission (POST /release/fedora/koji-build) --
# Operation #4.
http_get status body "http://127.0.0.1:${PORT}/release/fedora/koji-build" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    --data '{"srpm":"foo-1.2.3-1.fc44.src.rpm","target":"rawhide"}' || true
[ "$status" = "202" ] || fail "POST /release/fedora/koji-build returned $status, expected 202"
case "$body" in
  *"\"operation\": \"RemoteBuildSubmission\""*) ok "POST koji-build returns RemoteBuildSubmission-tagged body" ;;
  *) fail "POST koji-build body did not include RemoteBuildSubmission: $body" ;;
esac

# ---- 9. auth → 202 on DistributionUpdateCreation (POST /release/fedora/bodhi-update) --
# Operation #5.
http_get status body "http://127.0.0.1:${PORT}/release/fedora/bodhi-update" \
    -H "Authorization: Bearer ${TOKEN}" \
    -H "Content-Type: application/json" \
    --data '{"package":"foo","version":"1.2.3-1.fc44"}' || true
[ "$status" = "202" ] || fail "POST /release/fedora/bodhi-update returned $status, expected 202"
case "$body" in
  *"\"operation\": \"DistributionUpdateCreation\""*) ok "POST bodhi-update returns DistributionUpdateCreation-tagged body" ;;
  *) fail "POST bodhi-update body did not include DistributionUpdateCreation: $body" ;;
esac

# ---- 10. unknown provider → 404 ---------------------------------------
http_get status body "http://127.0.0.1:${PORT}/issues/not-a-real-provider/1" \
    -H "Authorization: Bearer ${TOKEN}" || true
[ "$status" = "404" ] || fail "GET /issues/not-a-real-provider/1 returned $status, expected 404"
ok "unknown provider returns 404"

# ---- 11. GET on a release endpoint → 405 -------------------------------
http_get status body "http://127.0.0.1:${PORT}/release/debian/canonical-push" \
    -H "Authorization: Bearer ${TOKEN}" || true
[ "$status" = "405" ] || fail "GET /release/debian/canonical-push returned $status, expected 405"
ok "GET on a POST-only release endpoint returns 405"

# ---- 12. unknown route → 404 ------------------------------------------
http_get status body "http://127.0.0.1:${PORT}/no/such/path" || true
[ "$status" = "404" ] || fail "GET /no/such/path returned $status, expected 404"
ok "unknown route returns 404"

# ---- 13. signing stays out of scope (spec §4.6 constraint #3) ---------
# There must be NO /sign endpoint. If a future change adds one, this
# check will start passing — flip it to fail in that case.
http_get status body "http://127.0.0.1:${PORT}/sign/foo" || true
case "$status" in
  404|405) ok "no /sign endpoint exposed (spec §4.6 constraint #3)" ;;
  *) fail "/sign returns $status — signing is forbidden per spec §4.6" ;;
esac

echo "ironmaint/fake-services:0.1 — SMOKE OK"