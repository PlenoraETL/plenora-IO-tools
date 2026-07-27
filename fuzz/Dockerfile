FROM rust:1.92-slim-bookworm

ARG NIGHTLY=nightly-2026-07-21
ARG CARGO_FUZZ_VERSION=0.13.2

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates clang g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install "${NIGHTLY}" --profile minimal \
    && rustup component add rustfmt \
    && rustup component add --toolchain "${NIGHTLY}" rustfmt \
    && cargo +"${NIGHTLY}" install cargo-fuzz \
        --version "${CARGO_FUZZ_VERSION}" \
        --locked

ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-07-21
WORKDIR /work
