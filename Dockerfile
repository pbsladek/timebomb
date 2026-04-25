# syntax=docker/dockerfile:1.7

ARG RUST_IMAGE=dhi.io/rust:1-debian13-dev
ARG STATIC_IMAGE=dhi.io/static:20250419-glibc-debian13

FROM ${RUST_IMAGE} AS build
WORKDIR /work

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY benches ./benches

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/work/target \
    cargo build --locked --profile dist && \
    cp target/dist/timebomb /usr/local/bin/timebomb

FROM ${STATIC_IMAGE}

LABEL org.opencontainers.image.title="timebomb" \
      org.opencontainers.image.description="Scan source code for deadline-tagged fuses and fail when they detonate" \
      org.opencontainers.image.source="https://github.com/pbsladek/timebomb" \
      org.opencontainers.image.licenses="MIT"

COPY --from=build /usr/local/bin/timebomb /usr/local/bin/timebomb

ENTRYPOINT ["/usr/local/bin/timebomb"]
