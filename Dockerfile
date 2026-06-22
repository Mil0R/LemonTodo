# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim AS web-builder
WORKDIR /src
COPY apps/web/package.json apps/web/package-lock.json apps/web/
RUN npm ci --prefix apps/web
COPY apps/web apps/web
RUN npm --prefix apps/web run build

FROM rust:1.95-bookworm AS server-builder
WORKDIR /src
RUN rustup target add wasm32-unknown-unknown \
    && cargo install wasm-bindgen-cli --version 0.2.125 --locked
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY scripts scripts
RUN cargo build --release -p lemontodo-server
RUN LEMONTODO_REGISTER_WASM_DIR=/out/register-wasm ./scripts/build-register-wasm.sh

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /opt/lemontodo --create-home lemontodo \
    && mkdir -p /data /opt/lemontodo/register-wasm /opt/lemontodo/web \
    && chown -R lemontodo:lemontodo /data /opt/lemontodo

COPY --chown=lemontodo:lemontodo --from=server-builder /src/target/release/ltd-server /usr/local/bin/ltd-server
COPY --chown=lemontodo:lemontodo --from=server-builder /out/register-wasm /opt/lemontodo/register-wasm
COPY --chown=lemontodo:lemontodo --from=web-builder /src/apps/web/dist /opt/lemontodo/web
RUN chmod 755 /usr/local/bin/ltd-server \
    && chmod -R u+rwX,go+rX /opt/lemontodo/register-wasm /opt/lemontodo/web

ENV LEMONTODO_SERVER_HOST=0.0.0.0 \
    LEMONTODO_SERVER_PORT=8787 \
    LEMONTODO_SERVER_DB=/data/lemontodo.db \
    LEMONTODO_ALLOW_REGISTRATION=false \
    LEMONTODO_BILLING_ENABLED=false \
    LEMONTODO_BILLING_PROVIDER= \
    LEMONTODO_BILLING_MONTHLY_PRICE_CENTS=0 \
    LEMONTODO_BILLING_CURRENCY=USD \
    LEMONTODO_REGISTER_WASM_DIR=/opt/lemontodo/register-wasm \
    LEMONTODO_WEB_STATIC_DIR=/opt/lemontodo/web \
    LEMONTODO_WEB_CLIENT_URL=/

WORKDIR /opt/lemontodo
VOLUME ["/data"]
EXPOSE 8787
USER lemontodo
CMD ["ltd-server"]
