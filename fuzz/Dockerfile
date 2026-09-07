FROM rust:1.98-slim-bookworm

# Tenuti allineati a scripts/toolchain-pins.env dal gate check_toolchain_pins.py.
#
# Lo stable si installa **esplicitamente** anche qui: il tag `rust:1.98`
# porta l'ultima patch che Docker Hub ha pubblicato per quel minor, che non
# e' necessariamente quella pinnata. Senza, `rustup component add rustfmt`
# qui sotto finirebbe sulla patch dell'immagine invece che su quella del
# progetto, e `cargo fmt` girerebbe da un'altra parte.
ARG PLENORA_RUST_STABLE=1.98.1
ARG PLENORA_RUST_NIGHTLY=nightly-2026-07-21
ARG PLENORA_CARGO_FUZZ_VERSION=0.13.2

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates clang g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup toolchain install "${PLENORA_RUST_STABLE}" --profile minimal \
    && rustup default "${PLENORA_RUST_STABLE}" \
    && rustup toolchain install "${PLENORA_RUST_NIGHTLY}" --profile minimal \
    && rustup component add clippy rustfmt llvm-tools-preview \
    && rustup component add --toolchain "${PLENORA_RUST_NIGHTLY}" rustfmt \
    && cargo +"${PLENORA_RUST_NIGHTLY}" install cargo-fuzz \
        --version "${PLENORA_CARGO_FUZZ_VERSION}" \
        --locked

ENV PLENORA_FUZZ_TOOLCHAIN=nightly-2026-07-21
WORKDIR /work
