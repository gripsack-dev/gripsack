# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan/0003 §4).
# rust:alpine's host triple IS x86_64-unknown-linux-musl, so plain
# `cargo build` already produces a static binary. rustls only — a
# dependency that drags in openssl is a bug (AGENTS.md hard rules).

FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev git \
    && rustup component add clippy rustfmt
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM builder AS test
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo test --locked

# TypeScript frontend tests (plan/0005 §1). Independent stage — no rust
# cache needed, builds in parallel with the chain below.
FROM node:22-alpine AS ts-test
WORKDIR /app
COPY typescript/package.json typescript/package-lock.json ./typescript/
RUN cd typescript && npm ci
COPY typescript ./typescript
RUN cd typescript && npm test

# Python frontend tests. FROM test reuses the compiled cache and makes
# the rust gate a precondition of this stage existing at all.
FROM test AS pytest
RUN apk add --no-cache python3 uv
COPY python ./python
RUN cd python && uv sync --locked \
    && uv run --locked --no-sync pytest

# E2E flow tests: real binary + real frontend against fixture env repos
# in a sandboxed HOME (offline). Binary comes from the gate's cache.
# Bun (pinned + sha256-verified, musl build) + the built TS frontend
# make the dual-frontend parity corpus run HERE, in the required gate —
# not just in the examples canary (final review, 0.16.1).
FROM pytest AS e2e
ARG BUN_VERSION=1.4.0
ARG BUN_SHA256=83b5f12fd258dd8d4fdcaea65ede954366aa717dab399e20093ecab280d54e7a
RUN apk add --no-cache curl unzip \
    && curl -fsSL -o /tmp/bun.zip \
      "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-linux-x64-musl.zip" \
    && echo "${BUN_SHA256}  /tmp/bun.zip" | sha256sum -c - \
    && unzip -q /tmp/bun.zip -d /tmp \
    && mv /tmp/bun-linux-x64-musl/bun /usr/local/bin/bun \
    && rm -rf /tmp/bun.zip /tmp/bun-linux-x64-musl
COPY e2e/pyproject.toml e2e/uv.lock ./e2e/
RUN cargo build --locked && cd e2e && uv sync --locked
COPY e2e ./e2e
COPY --from=ts-test /app/typescript/dist ./typescript/dist
COPY typescript/package.json ./typescript/package.json
ENV GRIPSACK_E2E_IN_DOCKER=1
ENV GRIPSACK_BIN=/app/target/debug/grip
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
