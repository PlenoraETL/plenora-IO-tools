FROM rust:1.92-slim-bookworm

# Tenuti allineati a scripts/toolchain-pins.env dal gate check_toolchain_pins.py.
ARG PLENORA_RUST_NIGHTLY=nightly-2026-07-21
ARG PLENORA_CARGO_FUZZ_VERSION=0.13.2

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates clang g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install "${PLENORA_RUST_NIGHTLY}" --profile minimal \
    && rustup component add rustfmt \
    && rustup component add --toolchain "${PLENORA_RUST_NIGHTLY}" rustfmt \
    && cargo +"${PLENORA_RUST_NIGHTLY}" install cargo-fuzz \
        --version "${PLENORA_CARGO_FUZZ_VERSION}" \
        --locked

ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-07-21
WORKDIR /work
