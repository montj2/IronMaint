# containers/Makefile — build, test, and clean the IronMaint image set.
#
# Targets:
#   make build                Build base + workspace + debian-tools + fedora-tools
#                             + agent + fake-services for the native arch.
#   make build-base           Build just base.
#   make build-workspace      Build just workspace.
#   make build-debian-tools   Build just debian-tools (FROM ironmaint/base:0.1).
#   make build-fedora-tools   Build just fedora-tools (FROM fedora:44, NOT base).
#   make build-agent          Build just agent (FROM ironmaint/base:0.1).
#   make build-fake-services  Build just fake-services (FROM ironmaint/base:0.1).
#   make build-multi          Multi-arch build (arm64 + amd64), pushes to local
#                             registry at localhost:5000. Requires the registry
#                             running (see `make registry`).
#   make smoke                Run smoke tests for base + workspace + debian-tools
#                             + fedora-tools + agent + fake-services.
#   make smoke-base
#   make smoke-workspace
#   make smoke-debian-tools
#   make smoke-fedora-tools
#   make smoke-agent
#   make smoke-fake-services
#   make verify               Build + smoke, fail on any non-zero exit.
#   make registry             Start a local registry:2 on :5000 (background).
#   make inspect              Print image metadata.
#   make clean                Remove locally-loaded images.
#   make clean-volumes        Also drop the compose-managed named volumes.

REGISTRY            ?= localhost:5000
BASE_TAG            ?= ironmaint/base:0.1
WORKSPACE_TAG       ?= ironmaint/workspace:0.1
DEBIAN_TOOLS_TAG    ?= ironmaint/debian-tools:0.1
FEDORA_TOOLS_TAG    ?= ironmaint/fedora-tools:0.1
AGENT_TAG           ?= ironmaint/agent:0.1
FAKE_SERVICES_TAG   ?= ironmaint/fake-services:0.1
NATIVE_ARCH    := $(shell docker buildx inspect default 2>/dev/null \
                    | awk -F'= ' '/Platforms/ {print $$2; exit}' \
                    | awk -F, '{print $$1}' \
                    | sed 's/linux\///')
ifeq ($(NATIVE_ARCH),)
NATIVE_ARCH    := arm64
endif
PLATFORMS      := linux/arm64,linux/amd64

# Detect the docker driver for buildx. The `docker` driver supports
# --load; the `docker-container` driver does not — it requires --push.
# For local multi-arch we use docker-container + push to local registry.
DRIVER         := $(shell docker buildx inspect 2>/dev/null \
                    | awk -F': ' '/^Driver:/ {print $$2; exit}')

# ---- build ----------------------------------------------------------------
.PHONY: build
build: build-base build-workspace build-debian-tools build-fedora-tools build-agent build-fake-services

.PHONY: build-base
build-base:
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(BASE_TAG) \
		--load \
		--file containers/base/Dockerfile \
		containers/base/

.PHONY: build-workspace
build-workspace:
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(WORKSPACE_TAG) \
		--load \
		--file containers/workspace/Dockerfile \
		containers/workspace/

.PHONY: build-debian-tools
build-debian-tools: build-base
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(DEBIAN_TOOLS_TAG) \
		--load \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/debian-tools/Dockerfile \
		containers/debian-tools/

.PHONY: build-fedora-tools
build-fedora-tools:
	# fedora-tools derives FROM fedora:44, not from ironmaint/base —
	# spec §3 explicit. No build-base dependency.
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(FEDORA_TOOLS_TAG) \
		--load \
		--file containers/fedora-tools/Dockerfile \
		containers/fedora-tools/

.PHONY: build-agent
build-agent: build-base
	# Agent pulls a prebuilt ironclaw release tarball (no compilation,
	# no Node toolchain, no Rust toolchain in this image). Spec §4.5:
	# build conventions (pinned base digests, exec-form ENTRYPOINT,
	# tini as PID 1), not the submodule's Railway deployment shape.
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(AGENT_TAG) \
		--load \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/agent/Dockerfile \
		containers/agent/

.PHONY: build-fake-services
build-fake-services: build-base
	# fake-services is a Python 3 stdlib HTTP server (~20MB final).
	# Spec §4.6 — built despite the "Phase 1, specified not built"
	# caveat because the integration-test surface for the privileged
	# path needs to exist before the D-09 producer lands.
	docker buildx build \
		--platform linux/$(NATIVE_ARCH) \
		--tag $(FAKE_SERVICES_TAG) \
		--load \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/fake-services/Dockerfile \
		containers/fake-services/

# ---- multi-arch ----------------------------------------------------------
.PHONY: build-multi
build-multi: registry
	@if [ "$(DRIVER)" != "docker-container" ]; then \
		echo "Re-creating buildx builder with docker-container driver..."; \
		docker buildx create --use --name ironmaint-multi \
			--driver docker-container --bootstrap; \
	fi
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(BASE_TAG) \
		--tag $(REGISTRY)/ironmaint/base:latest \
		--push \
		--file containers/base/Dockerfile \
		containers/base/
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(WORKSPACE_TAG) \
		--tag $(REGISTRY)/ironmaint/workspace:latest \
		--push \
		--file containers/workspace/Dockerfile \
		containers/workspace/
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(DEBIAN_TOOLS_TAG) \
		--tag $(REGISTRY)/ironmaint/debian-tools:latest \
		--push \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/debian-tools/Dockerfile \
		containers/debian-tools/
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(FEDORA_TOOLS_TAG) \
		--tag $(REGISTRY)/ironmaint/fedora-tools:latest \
		--push \
		--file containers/fedora-tools/Dockerfile \
		containers/fedora-tools/
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(AGENT_TAG) \
		--tag $(REGISTRY)/ironmaint/agent:latest \
		--push \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/agent/Dockerfile \
		containers/agent/
	docker buildx build \
		--platform $(PLATFORMS) \
		--tag $(REGISTRY)/$(FAKE_SERVICES_TAG) \
		--tag $(REGISTRY)/ironmaint/fake-services:latest \
		--push \
		--build-arg BASE_TAG=$(BASE_TAG) \
		--file containers/fake-services/Dockerfile \
		containers/fake-services/

.PHONY: registry
registry:
	@if ! docker ps --format '{{.Names}}' | grep -q '^ironmaint-registry$$'; then \
		echo "Starting local registry:2 on :5000..."; \
		docker run -d -p 5000:5000 --restart=always \
			--name ironmaint-registry registry:2; \
	fi

# ---- smoke ---------------------------------------------------------------
.PHONY: smoke
smoke: smoke-base smoke-workspace smoke-debian-tools smoke-fedora-tools smoke-agent smoke-fake-services

.PHONY: smoke-base
smoke-base: build-base
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		$(BASE_TAG) \
		/usr/local/bin/smoke.sh

.PHONY: smoke-workspace
smoke-workspace: build-workspace
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		-v "$(PWD)":/work \
		-w /work \
		$(WORKSPACE_TAG) \
		bash /usr/local/bin/smoke.sh

.PHONY: smoke-debian-tools
smoke-debian-tools: build-debian-tools
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		-v "$(PWD)":/work \
		-w /work \
		$(DEBIAN_TOOLS_TAG) \
		bash /usr/local/bin/smoke.sh

.PHONY: smoke-fedora-tools
smoke-fedora-tools: build-fedora-tools
	# Smoke runs WITHOUT --privileged: the spec §4.4 mock init/build gate
	# is in scripts/fedora-tools-build-smoke.sh and runs separately from
	# compose. This smoke verifies the toolchain binaries + group setup
	# + TLS, which don't need privileges.
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		-v "$(PWD)":/work \
		-w /work \
		$(FEDORA_TOOLS_TAG) \
		bash /usr/local/bin/smoke.sh

.PHONY: smoke-agent
smoke-agent: build-agent
	# Agent smoke is just version + env + TLS — no real `ironclaw serve`
	# against a postgres sidecar (that's a Phase 1 acceptance gate,
	# spec §4.5). No bind mount needed because the smoke doesn't read
	# the repo.
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		$(AGENT_TAG) \
		bash /usr/local/bin/smoke.sh

.PHONY: smoke-fake-services
smoke-fake-services: build-fake-services
	# The smoke starts the server in the background and exercises each
	# of the 6 spec endpoints in turn. No bind mount, no port publish
	# to the host — the smoke listens on 127.0.0.1 inside the container.
	# Disable partial failure in smoke (the 5% 502 rate is for realism,
	# not for an audit-time smoke check).
	docker run --rm --platform linux/$(NATIVE_ARCH) \
		-e FAKE_SERVICES_PARTIAL_FAILURE_RATE=0.0 \
		$(FAKE_SERVICES_TAG) \
		bash /usr/local/bin/smoke.sh

# ---- inspect / clean -----------------------------------------------------
.PHONY: inspect
inspect:
	docker buildx imagetools inspect $(BASE_TAG) || true
	docker buildx imagetools inspect $(WORKSPACE_TAG) || true
	docker buildx imagetools inspect $(DEBIAN_TOOLS_TAG) || true
	docker buildx imagetools inspect $(FEDORA_TOOLS_TAG) || true
	docker buildx imagetools inspect $(AGENT_TAG) || true
	docker buildx imagetools inspect $(FAKE_SERVICES_TAG) || true

.PHONY: clean
clean:
	-docker image rm -f $(BASE_TAG) $(WORKSPACE_TAG) $(DEBIAN_TOOLS_TAG) $(FEDORA_TOOLS_TAG) $(AGENT_TAG) $(FAKE_SERVICES_TAG) 2>/dev/null
	-docker buildx stop ironmaint-multi 2>/dev/null
	-docker buildx rm ironmaint-multi 2>/dev/null
	-docker container rm -f ironmaint-registry 2>/dev/null

.PHONY: clean-volumes
clean-volumes: clean
	docker volume rm ironmaint_target ironmaint_cargo-registry \
		ironmaint_cargo-git ironmaint_ironmaint-state 2>/dev/null || true

# ---- CI-style verification ----------------------------------------------
.PHONY: verify
verify: build smoke
	@echo "All images built and smoke tests passed for $(NATIVE_ARCH)."
