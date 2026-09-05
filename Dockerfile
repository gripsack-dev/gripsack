# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan/0003 §4).
# rust:alpine's host triple IS x86_64-unknown-linux-musl, so plain
# `cargo build` already produces a static binary. rustls only — a
# dependency that drags in openssl is a bug (AGENTS.md hard rules).
#
# The eval runtime is deno (plan/0013 D2), which ships no musl build:
# the grip binary stays musl-static; deno-dependent stages (ts-test,
# e2e) run on glibc bases, where the static binary works fine.

FROM rust:alpine AS builder
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
# the exec crate's build.rs embeds typescript/src as the frontend —
# without this COPY the embed is empty and every eval fails with
# "no embedded frontend"
COPY typescript ./typescript

FROM builder AS test
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo test --locked

# The debug binary for stages that need a runnable grip (e2e).
FROM builder AS bin
RUN cargo build --locked -p gripsack

# The transaction model check (plan/0028): TLC over the TLA+ spec of
# the journal protocol — the shipped configs must check clean, and the
# tla2tools is checksum-pinned like deno.
FROM eclipse-temurin:21-jre AS model
ARG TLA_TOOLS_SHA256=b658b4e504fdf0b721caf7066320f6b6fe5805f4dd2f717d0e47baba4097205e
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
RUN java -jar /tla/tla2tools.jar -cleanup -config specs/cfg/ownership.cfg specs/Ownership.tla > /tmp/ownership.log 2>&1 \
    && grep -q "Model checking completed. No error" /tmp/ownership.log \
    || { echo "TLC failed for ownership"; cat /tmp/ownership.log; exit 1; }; \
    echo "ownership: clean"; \
    for cfg in apply-deploy rollback-deploy apply-prune rollback-prune; do \
      java -jar /tla/tla2tools.jar -cleanup -config specs/cfg/$cfg.cfg specs/Transaction.tla > /tmp/$cfg.log 2>&1 \
      && grep -q "Model checking completed. No error" /tmp/$cfg.log \
      || { echo "TLC failed for $cfg"; cat /tmp/$cfg.log; exit 1; }; \
      echo "$cfg: clean"; \
    done \
    && java -jar /tla/tla2tools.jar -cleanup -config specs/cfg/activation.cfg specs/Activation.tla > /tmp/activation.log 2>&1 \
    && grep -q "Model checking completed. No error" /tmp/activation.log \
    || { echo "TLC failed for activation"; cat /tmp/activation.log; exit 1; }; \
    echo "activation: clean"

# TypeScript frontend tests (plan/0005 §1, plan/0013 D1): `deno test`
# on the source tree — no transpile chain, no node_modules. The image
# tag is the same version DENO_RELEASE pins in
# crates/gripsack-fetch/src/host.rs; bump them together.
FROM denoland/deno:2.9.6 AS ts-test
WORKDIR /app
COPY typescript ./typescript
# deno install materializes node_modules (@types/node) for the
# type-checker; build-time network is fine — the runtime eval path
# stays --cached-only --no-remote.
RUN cd typescript && deno install && deno task test

# E2E flow tests: the real (musl-static, runs-everywhere) binary
# against fixture env repos in a sandboxed HOME (offline). Base is
# glibc because the eval runtime (deno) ships no musl build; the
# harness stays pytest. The pinned deno is prefetched at image build
# into $GRIPSACK_HOME/tools (checksum-verified, same sha256 as
# DENO_RELEASE) and GRIPSACK_DENO points at it — e2e never provisions.
FROM python:3.13-slim AS e2e
WORKDIR /app
# git: --repo clone tests and the trust gate's remote/commit probes
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends git \
    && rm -rf /var/lib/apt/lists/*
ARG DENO_VERSION=2.9.6
ARG DENO_SHA256=394f07f4da2bebe6ce6f1e7ce0fa16429b29b08c35e3fac3fe25972676dff4b2
ADD --checksum=sha256:${DENO_SHA256} https://github.com/denoland/deno/releases/download/v${DENO_VERSION}/deno-x86_64-unknown-linux-gnu.zip /tmp/deno.zip
RUN python3 -m zipfile -e /tmp/deno.zip /tmp/deno \
    && mkdir -p /root/.local/share/gripsack/tools/deno-${DENO_VERSION} \
    && mv /tmp/deno/deno /root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno \
    && chmod 755 /root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno \
    && rm -rf /tmp/deno.zip /tmp/deno \
    && pip install --no-cache-dir uv
ENV GRIPSACK_DENO=/root/.local/share/gripsack/tools/deno-${DENO_VERSION}/deno
COPY --from=bin /app/target/debug/grip /usr/local/bin/grip
COPY e2e/pyproject.toml e2e/uv.lock ./e2e/
RUN cd e2e && uv sync --locked
COPY e2e ./e2e
ENV GRIPSACK_E2E_IN_DOCKER=1
ENV GRIPSACK_BIN=/usr/local/bin/grip
WORKDIR /app/e2e
CMD ["uv", "run", "--locked", "--no-sync", "pytest"]

# Stripped static release binary.
FROM builder AS release
RUN cargo auditable build --release --locked -p gripsack \
    && strip target/release/grip \
    && ldd target/release/grip 2>&1 | grep -q "Not a valid dynamic program\|not a dynamic executable" \
    && echo "static: ok"

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/grip /grip
ENTRYPOINT ["/grip"]
