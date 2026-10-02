# containers/Makefile — build, test, and clean the IronMaint image set.
#
# Targets:
#   make build            Build base and workspace for the native arch.
#   make build-base       Build just base.
#   make build-workspace  Build just workspace.
#   make build-multi      Multi-arch build (arm64 + amd64), pushes to local
#                         registry at localhost:5000. Requires the registry
#                         running (see `make registry`).
#   make smoke            Run smoke tests for base and workspace.
#   make smoke-base
#   make smoke-workspace
#   make verify           Build + smoke, fail on any non-zero exit.
#   make registry         Start a local registry:2 on :5000 (background).
#   make inspect          Print image metadata.
#   make clean            Remove locally-loaded images.
#   make clean-volumes    Also drop the compose-managed named volumes.

REGISTRY       ?= localhost:5000
BASE_TAG       ?= ironmaint/base:0.1
WORKSPACE_TAG  ?= ironmaint/workspace:0.1
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
build: build-base build-workspace

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

.PHONY: registry
registry:
	@if ! docker ps --format '{{.Names}}' | grep -q '^ironmaint-registry$$'; then \
		echo "Starting local registry:2 on :5000..."; \
		docker run -d -p 5000:5000 --restart=always \
			--name ironmaint-registry registry:2; \
	fi

# ---- smoke ---------------------------------------------------------------
.PHONY: smoke
smoke: smoke-base smoke-workspace

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

# ---- inspect / clean -----------------------------------------------------
.PHONY: inspect
inspect:
	docker buildx imagetools inspect $(BASE_TAG) || true
	docker buildx imagetools inspect $(WORKSPACE_TAG) || true

.PHONY: clean
clean:
	-docker image rm -f $(BASE_TAG) $(WORKSPACE_TAG) 2>/dev/null
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
