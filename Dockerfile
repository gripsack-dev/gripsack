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
RUN apk add --no-cache musl-dev git \
    && rustup component add clippy rustfmt
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
RUN cargo build --release --locked -p gripsack \
    && strip target/release/grip \
    && ldd target/release/grip 2>&1 | grep -q "Not a valid dynamic program\|not a dynamic executable" \
    && echo "static: ok"

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/grip /grip
ENTRYPOINT ["/grip"]
