# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan/0003 §4).
# rust:alpine's host triple IS x86_64-unknown-linux-musl, so plain
# `cargo build` already produces a static binary. rustls only — a
# dependency that drags in openssl is a bug (AGENTS.md hard rules).
#
# The eval runtime is deno (plan/0013 D2), which ships no musl build:
# the grip binary stays musl-static; deno-dependent stages (ts-test,
# e2e) run on glibc bases, where the static binary works fine.

# GNU-linked loader fixtures require a GNU compiler and runtime. Coupled to
# the musl builder/native CI toolchain by scripts/check_pins.py.
ARG GNU_RUST_IMAGE=rust:1.98.0@sha256:620dbcd124499c59e2406d3741574b5c5838cf9eb9656f0c3a03948f79b02959

# Digest-pinned (plan/0042 F): this manifest-list sha256 is the exact
# rust:alpine whose rustc is 1.98.0 (88d9e12ae 2026-08-18) — the same
# toolchain CI's native jobs pin via dtolnay/rust-toolchain. Tags move;
# digests do not. Updates are deliberate, proposed by scripts/check_pins.py
# (upstream-watch workflow). The historical multi-architecture index remains
# pinned; active qualification resolves only its linux/amd64 leaf.
FROM rust:alpine@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce AS builder
ENV CARGO_BUILD_JOBS=4
RUN apk add --no-cache musl-dev git curl xz \
    && rustup component add clippy rustfmt
# cargo-auditable embeds the dependency tree into the release binary
# (plan/0022) — the SBOM travels with grip. Prebuilt + sha256-pinned:
# from-source tool installs break on new toolchains (the cargo-audit
# lesson, audit.yml). Debug/e2e builds stay plain cargo.
ARG CARGO_AUDITABLE_VERSION=0.7.5
RUN set -e; \
    arch="$(uname -m)"; \
    case "$arch" in \
      x86_64)  sha=3374daaf153e6f82028add5e4bf7cc2deab46537dee24f20be80df831193aeb4 ;; \
      aarch64) sha=35d90cee9648037eaa4c1a2649fdca9d1b9a9997b972d37be7f8629139ba1294 ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/ca.tar.xz "https://github.com/rust-secure-code/cargo-auditable/releases/download/v${CARGO_AUDITABLE_VERSION}/cargo-auditable-${arch}-unknown-linux-musl.tar.xz"; \
    echo "$sha  /tmp/ca.tar.xz" | sha256sum -c -; \
    mkdir -p /tmp/ca && tar -xJf /tmp/ca.tar.xz -C /tmp/ca; \
    mv /tmp/ca/*/cargo-auditable /usr/local/bin/cargo-auditable; \
    rm -rf /tmp/ca /tmp/ca.tar.xz
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Fuzzing shares Cargo.lock with production, including decoder dependencies.
COPY fuzz ./fuzz
# schema: the IR v3 JSON schema is include_str!'d by the ir crate's
# schema-acceptance tests (0042 E) — the test gate needs it in-image.
COPY schema ./schema
# the exec crate's build.rs embeds typescript/src as the frontend —
# without this COPY the embed is empty and every eval fails with
# "no embedded frontend"
COPY typescript ./typescript

# Real upstream LLB conformance is a test prerequisite, not a core/runtime
# dependency. Ordinary bin/release stages deliberately do not copy this helper.
FROM golang:1.26.3@sha256:e3665e241a474aba30bbfaf177cfa88e1913e970c83bd86889cacfb67d6e7e51 AS buildkit-bridge-test
WORKDIR /bridge
ENV GOFLAGS=-mod=readonly GOTOOLCHAIN=local
COPY tools/buildkit-bridge/go.mod tools/buildkit-bridge/go.sum ./
RUN go mod download
COPY tools/buildkit-bridge/ ./
RUN test -z "$(gofmt -l .)" \
    && go vet ./... \
    && go test -race ./... \
    && CGO_ENABLED=0 go build -trimpath -buildvcs=false -o /gripsack-buildkit-bridge .

FROM builder AS test
COPY --from=buildkit-bridge-test /gripsack-buildkit-bridge /usr/local/libexec/gripsack-buildkit-bridge
ENV GRIPSACK_TEST_BRIDGE=/usr/local/libexec/gripsack-buildkit-bridge
COPY tools/conda-helper ./tools/conda-helper
RUN apk add --no-cache python3 build-base cmake perl linux-headers xz-dev bzip2-dev zstd-dev
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo test --locked
# The optional installer has its own dependency lock, but not an optional gate.
RUN cargo fmt --manifest-path tools/conda-helper/Cargo.toml --all --check \
    && CARGO_TARGET_DIR=/app/target cargo clippy --locked --manifest-path tools/conda-helper/Cargo.toml --all-targets -- -D warnings \
    && CARGO_TARGET_DIR=/app/target cargo test --locked --manifest-path tools/conda-helper/Cargo.toml --all-targets
# CI-only script changes must not invalidate Rust compilation layers.
COPY scripts ./scripts
RUN python3 scripts/check_architecture.py --resolved-manifest tools/conda-helper/Cargo.toml
# Published artifacts and coordinator/journal/GC admission boundaries
# are checked and mutation-calibrated. Tool/build failures never qualify as
# successful semantic negatives.
RUN apk add --no-cache python3 \
    && cargo package --list -p gripsack-exec | grep -q "embedded_frontend.rs" \
    && python3 scripts/gen_diagnostic_registry.py --check \
    && python3 scripts/gen_diagnostic_registry.py --self-check \
    && python3 scripts/gen_frontend_embed.py --check \
    && python3 scripts/check_scheduler_loom.py \
    && python3 scripts/check_journal_admission.py \
    && python3 scripts/check_journal_protocol.py \
    && python3 scripts/check_gc_roots.py \
    && python3 scripts/check_rate_admission.py \
    && python3 scripts/check_http_budget.py \
    && python3 scripts/check_process_bounds.py \
    && python3 scripts/check_update_survey.py

# The debug binary for stages that need a runnable grip (e2e).
FROM builder AS bin
RUN cargo build --locked -p gripsack

# Required GNU-native admission regressions; these cannot execute on Alpine.
# The e2e stage separately exercises the shipped musl core on a GNU host.
FROM ${GNU_RUST_IMAGE} AS gnu-test
ENV CARGO_BUILD_JOBS=4
WORKDIR /app
COPY --from=builder /app /app
RUN cargo test --locked -p gripsack-exec --lib workspace::consumer::admit::host_runtime_tests \
    && cargo test --locked -p gripsack-process --lib input::tests

# The model gate (plan/0028, 0042 G): TLC over the TLA+ specs via the
# canonical runner scripts/check_models.sh — positive protocols AND
# calibrated counterexamples (each negative config must violate exactly
# the invariant/temporal property it names; parser errors never count).
# tla2tools is checksum-pinned to a stable release; prerelease assets are replaced.
# Digest independently corroborated by nixpkgs; base image digest-pinned like the rest.
FROM eclipse-temurin:21-jre@sha256:7a65df4b22d2de92d4e04056e884f3b9122d70b21e2847fd66084278bd0ce037 AS model
ARG TLA_TOOLS_VERSION=1.7.4
ARG TLA_TOOLS_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v${TLA_TOOLS_VERSION}/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
COPY scripts/check_models.sh ./scripts/check_models.sh
RUN sh scripts/check_models.sh /tla/tla2tools.jar

# The rolling upstream asset is byte-pinned; replacement must fail closed.
# GitHub asset 647831989 (2026-10-16, 46,758,585 bytes) and an independent
# HTTPS download matched this digest; the relocated binary reports 901ae51.
# Its Isabelle2025 bundle is self-contained. The full induction and mutant
# gate below must qualify this exact revision; no checksum or proof waiver.
FROM model AS tlaps
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
    python3 libgmp10 \
    && rm -rf /var/lib/apt/lists/*
ARG TLAPS_SHA256=b3c2554d3dd2198772ea8c814639a024fe9c390eac0ec25f4f444ad446a7dd28
ADD --checksum=sha256:${TLAPS_SHA256} https://github.com/tlaplus/tlapm/releases/download/1.6.0-pre/tlapm-1.6.0-pre-x86_64-linux-gnu.tar.gz /tmp/tlapm.tar.gz
RUN mkdir -p /opt/tlapm \
    && tar -xzf /tmp/tlapm.tar.gz -C /opt/tlapm --strip-components=1 \
    && rm /tmp/tlapm.tar.gz
ENV PATH="/opt/tlapm/bin:$PATH"
COPY scripts/check_tlaps.py scripts/tlaps_evidence.py scripts/tlaps_source.py scripts/tlaps_catalog.py ./scripts/
RUN python3 scripts/check_tlaps.py

# The verification gate (plan/0046): cargo-verus over the policy
# kernels, positive proof + seeded-mutant calibration, via the
# canonical runner scripts/check_verus.sh. Verus ships no musl or
# aarch64-linux prebuilt and wants glibc ≥ 2.39, so this stage is
# ubuntu:24.04 + amd64 by design — the musl release and macOS lanes
# never see the verifier. Base, verifier, toolchain and solver are
# all digest/version-pinned; the verifier's Rust (1.98.0) is the
# repo's pinned toolchain.
FROM ubuntu:24.04@sha256:224a1869083a311ef3f13648a154ba79832fbef6364d31493642ca03082da254 AS verify
ENV CARGO_BUILD_JOBS=4
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
    curl ca-certificates unzip build-essential python3 \
    && rm -rf /var/lib/apt/lists/*
ARG VERUS_RELEASE=0.2026.09.06.8dea4a2
ARG VERUS_SHA256=13d01e134c0620c3b29770874707d16c33b3d227c843a489c8ceb744d43c0a16
ARG Z3_VERSION=4.16.0
ARG Z3_SHA256=7288c49a5bd6dbafd7b0b0d1f65956b91672da24b08f09242919af159be3418e
ADD --checksum=sha256:${VERUS_SHA256} https://github.com/verus-lang/verus/releases/download/release/${VERUS_RELEASE}/verus-${VERUS_RELEASE}-x86-linux.zip /tmp/verus.zip
ADD --checksum=sha256:${Z3_SHA256} https://github.com/Z3Prover/z3/releases/download/z3-${Z3_VERSION}/z3-${Z3_VERSION}-x64-glibc-2.39.zip /tmp/z3.zip
RUN unzip -q /tmp/verus.zip -d /opt && unzip -q /tmp/z3.zip -d /opt \
    && rm /tmp/verus.zip /tmp/z3.zip \
    && curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.98.0 \
       --component rustc-dev --component llvm-tools --component rustfmt
ENV PATH="/opt/verus-x86-linux:/root/.cargo/bin:$PATH" \
    VERUS_Z3_PATH="/opt/z3-${Z3_VERSION}-x64-glibc-2.39/bin/z3" \
    RUSTUP_TOOLCHAIN=1.98.0
WORKDIR /app
# the workspace skeleton: the lockfile references every member
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY fuzz ./fuzz
COPY schema ./schema
COPY scripts/check_verus.sh scripts/check_verus.py scripts/verus_evidence.py scripts/check_collector_verus.py ./scripts/
RUN sh scripts/check_verus.sh

# TypeScript frontend tests and strict type checking of the four
# admission-only workspace examples (plan/0052 §5.4). The image tag
# matches the DENO_RELEASE provisioned by gripsack-fetch/src/host.rs.
FROM denoland/deno:2.9.6@sha256:2014dc167ece617ef7e7ba40631ac2234c59e75ce693e7cc2dc2602b3c87859d AS ts-test
WORKDIR /app
COPY typescript ./typescript
COPY examples ./examples
# deno install materializes node_modules (@types/node) for the
# type-checker; build-time network is fine — the runtime eval path
# stays --cached-only --no-remote.
RUN cd typescript && deno install && deno task test \
    && deno run --cached-only --no-remote --allow-all node_modules/typescript/bin/tsc \
       --project tsconfig.examples.json --noEmit

# E2E flow tests: the real (musl-static, runs-everywhere) binary
# against fixture env repos in a sandboxed HOME (offline). Base is
# glibc because the eval runtime (deno) ships no musl build; the
# harness stays pytest. The pinned deno is prefetched at image build
# into $GRIPSACK_HOME/tools (checksum-verified, same sha256 as
# DENO_RELEASE) and GRIPSACK_DENO points at it — e2e never provisions.
FROM python:3.13-slim@sha256:9d2e5553305c7c7b0097999bb17187c69b921ccd6bc9d40e4bb5ebe652c00285 AS e2e
WORKDIR /app
# git: trust; curl: installer fixtures; cc: real GNU loader/security fixtures.
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends git curl build-essential \
    && rm -rf /var/lib/apt/lists/*
ARG DENO_VERSION=2.9.6
ARG DENO_SHA256=394f07f4da2bebe6ce6f1e7ce0fa16429b29b08c35e3fac3fe25972676dff4b2
ADD --checksum=sha256:${DENO_SHA256} https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip /tmp/deno.zip
RUN python3 -m zipfile -e /tmp/deno.zip /tmp/deno \
    && mkdir -p /root/.local/share/gripsack/tools/deno-${DENO_VERSION} \
    && mv /tmp/deno/deno /root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno \
    && chmod 755 /root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno \
    && rm -rf /tmp/deno.zip /tmp/deno
# uv pinned to PyPI-published wheel digests (plan/0042 F) — official
# metadata, no guessed checksums; refreshed deliberately by
# scripts/check_pins.py. --require-hashes makes the digests the gate.
# Keep the upstream wheel identities; active qualification is Linux x86_64.
ARG UV_VERSION=0.12.10
RUN printf 'uv==%s --hash=sha256:f7d6248ad9f2d282fea795f248da8fa666ab382df50d8385bd98e01049440a0c --hash=sha256:bd0afae6918795c6a61a649e64d735e339f3d62f8310235da6f19eb3f66f323b\n' "$UV_VERSION" > /tmp/uv.req \
    && pip install --no-cache-dir --no-deps --require-hashes -r /tmp/uv.req \
    && uv --version \
    && rm -f /tmp/uv.req
ENV GRIPSACK_DENO=/root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno
COPY --from=bin /app/target/debug/grip /usr/local/bin/grip
COPY e2e/pyproject.toml e2e/uv.lock ./e2e/
RUN cd e2e && uv sync --locked
COPY e2e ./e2e
COPY examples ./examples
COPY install.sh ./install.sh
ENV GRIPSACK_E2E_IN_DOCKER=1
ENV GRIPSACK_BIN=/usr/local/bin/grip
WORKDIR /app/e2e
CMD ["uv", "run", "--locked", "--no-sync", "pytest"]

# Nonpublishing local qualification tools. Tool bytes are immutable; dependency
# audits deliberately fetch the current advisory database and retain its revision.
FROM ${GNU_RUST_IMAGE} AS local-rust-tools
FROM e2e AS local-tools
COPY --from=local-rust-tools /usr/local/rustup /usr/local/rustup
COPY --from=local-rust-tools /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH="/app/e2e/.venv/bin:/usr/local/cargo/bin:$PATH"
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
    file binutils zstd bzip2 xz-utils ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && install -m755 "$GRIPSACK_DENO" /usr/local/bin/deno
ARG LOCAL_NODE_VERSION=22.23.2
ARG LOCAL_NODE_SHA256=d60acfe00a2932254bb0ad20e01b0d74397a0875595de719654b214f4b03f307
ADD --checksum=sha256:${LOCAL_NODE_SHA256} https://nodejs.org/dist/v${LOCAL_NODE_VERSION}/node-v${LOCAL_NODE_VERSION}-linux-x64.tar.xz /tmp/node.tar.xz
RUN tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm /tmp/node.tar.xz
ARG LOCAL_AUDIT_VERSION=0.22.2
ARG LOCAL_AUDIT_SHA256=7fb9497f8594b389e5fce5ef9b92db08432996895b2e0c5a0167a69ed445c428
ADD --checksum=sha256:${LOCAL_AUDIT_SHA256} https://github.com/rustsec/rustsec/releases/download/cargo-audit/v${LOCAL_AUDIT_VERSION}/cargo-audit-x86_64-unknown-linux-musl-v${LOCAL_AUDIT_VERSION}.tgz /tmp/audit.tgz
RUN tar -xzf /tmp/audit.tgz -C /usr/local/bin --strip-components=1 \
    "cargo-audit-x86_64-unknown-linux-musl-v${LOCAL_AUDIT_VERSION}/cargo-audit" \
    && rm /tmp/audit.tgz
WORKDIR /app
COPY . .

# Native helper tooling; the source is mounted read-only at canonical /src.
# Its two independent target directories and pin/license/protocol checks remain.
FROM builder AS local-helper
RUN apk add --no-cache python3 build-base binutils cmake perl linux-headers \
    xz-dev bzip2-dev zstd-dev

# Stripped static release binary.
FROM builder AS release
RUN cargo auditable build --release --locked -p gripsack \
    && strip target/release/grip \
    && ldd target/release/grip 2>&1 | grep -q "Not a valid dynamic program\|not a dynamic executable" \
    && echo "static: ok"

# Fuzz builds share the core compiler. Select its installed version explicitly:
# +stable may provision a newer release even inside a digest-pinned image.
FROM builder AS fuzz
RUN apk add --no-cache bubblewrap python3 g++
COPY scripts ./scripts
ENV FUZZ_TOOLCHAIN=1.98.0
CMD ["sh", "scripts/check_fuzz.sh"]

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/grip /grip
ENTRYPOINT ["/grip"]
