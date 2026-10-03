# syntax=docker/dockerfile:1.7
# Production images of ControlHub, built from the repository root (see deploy/docker-bake.hcl):
#   docker buildx bake -f deploy/docker-bake.hcl
# One target per service; all Rust services come from one compile stage.

ARG RUST_IMAGE=rust:1.98.1-bookworm
ARG NODE_IMAGE=node:22.23.3-bookworm-slim
ARG RUNTIME_IMAGE=debian:12.15-slim

# ---------------------------------------------------------------- Rust services
FROM ${RUST_IMAGE} AS rust-build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=controlhub-release-target \
    cargo build --release --locked \
      -p controlhub-ingest -p controlhub-processor -p controlhub-api -p controlhub-executor -p controlhub-archiver \
 && mkdir -p /out \
 && for b in ingest processor api executor archiver; do cp "target/release/controlhub-$b" /out/; done

# ---------------------------------------------------------------- WASM engine
FROM ${NODE_IMAGE} AS node
FROM ${RUST_IMAGE} AS wasm-build
ARG WASM_BINDGEN_VERSION=0.2.129
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version "${WASM_BINDGEN_VERSION}" --locked
COPY --from=node /usr/local/bin/node /usr/local/bin/node
WORKDIR /src
COPY . .
RUN rustup target add wasm32-unknown-unknown && node scripts/build-wasm.mjs

# ---------------------------------------------------------------- web
FROM ${NODE_IMAGE} AS web-build
WORKDIR /src
RUN corepack enable
COPY . .
COPY --from=wasm-build /src/packages/web-engine/pkg packages/web-engine/pkg
COPY --from=wasm-build /src/apps/web/public/wasm apps/web/public/wasm
COPY --from=wasm-build /src/apps/web/src/generated/wasm-manifest.ts apps/web/src/generated/wasm-manifest.ts
# Fixed at build time: rewrites point at the compose service names; the public URL feeds
# canonical links, robots.txt and the sitemap.
ARG CONTROLHUB_SITE_URL=http://localhost:3000
ENV CONTROLHUB_SITE_URL=${CONTROLHUB_SITE_URL} \
    CONTROLHUB_API_URL=http://api:8080 \
    CONTROLHUB_INGEST_URL=http://ingest:4318 \
    NEXT_TELEMETRY_DISABLED=1
RUN pnpm install --frozen-lockfile --filter "@controlhub/web..." \
 && pnpm --filter @controlhub/web build

FROM ${NODE_IMAGE} AS web
WORKDIR /app
ENV NODE_ENV=production HOSTNAME=0.0.0.0 PORT=3000 NEXT_TELEMETRY_DISABLED=1
COPY --from=web-build --chown=node:node /src/apps/web/.next/standalone ./
COPY --from=web-build --chown=node:node /src/apps/web/.next/static ./apps/web/.next/static
COPY --from=web-build --chown=node:node /src/apps/web/public ./apps/web/public
USER node
EXPOSE 3000
CMD ["node", "apps/web/server.js"]

# ---------------------------------------------------------------- service images
FROM ${RUNTIME_IMAGE} AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin controlhub
USER controlhub

FROM runtime AS ingest
COPY --from=rust-build /out/controlhub-ingest /usr/local/bin/
ENTRYPOINT ["controlhub-ingest"]

FROM runtime AS processor
COPY --from=rust-build /out/controlhub-processor /usr/local/bin/
ENTRYPOINT ["controlhub-processor"]

FROM runtime AS api
COPY --from=rust-build /out/controlhub-api /usr/local/bin/
ENTRYPOINT ["controlhub-api"]
CMD ["serve"]

FROM runtime AS executor
COPY --from=rust-build /out/controlhub-executor /usr/local/bin/
ENTRYPOINT ["controlhub-executor"]

FROM runtime AS archiver
COPY --from=rust-build /out/controlhub-archiver /usr/local/bin/
ENTRYPOINT ["controlhub-archiver"]
CMD ["run"]
