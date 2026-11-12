FROM docker.io/library/rust:1.97.0-bookworm AS rust-build
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY apps/bank-api ./apps/bank-api
COPY apps/bank-worker ./apps/bank-worker
COPY apps/provider-simulator ./apps/provider-simulator
COPY migrations ./migrations
COPY contracts ./contracts
RUN cargo build --release --locked --workspace --bins

FROM docker.io/library/debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 bank && useradd --uid 10001 --gid bank --no-create-home bank
COPY --from=rust-build /build/target/release/bank-api /build/target/release/bank-worker \
    /build/target/release/provider-simulator /build/target/release/bank-migrate \
    /build/target/release/bank-seed /build/target/release/bank-fund /usr/local/bin/
USER 10001:10001
WORKDIR /tmp
CMD ["bank-api"]

FROM docker.io/library/node:24.15.0-bookworm-slim AS web-build
WORKDIR /build
COPY package.json package-lock.json ./
COPY apps/customer-web ./apps/customer-web
COPY apps/staff-web ./apps/staff-web
COPY packages ./packages
RUN npm ci --ignore-scripts && npm run build --workspace @piggybank/customer-web \
    && npm run build --workspace @piggybank/staff-web

FROM docker.io/library/nginx:1.28-alpine AS web
COPY --from=web-build /build/apps/customer-web/dist /usr/share/nginx/customer
COPY --from=web-build /build/apps/staff-web/dist /usr/share/nginx/staff
COPY infra/cloud/shared/nginx.conf /etc/nginx/nginx.conf
