# Build from the repo root:  docker build -f crates/site/Dockerfile -t taskstaka-site .
# Deploy: cloudbuild.yaml at the repo root builds and pushes this image; then
#   gcloud run deploy taskstaka-site --image us-central1-docker.pkg.dev/taskstaka/site/taskstaka-site:<tag> --region us-central1 --project taskstaka
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY assets ./assets
COPY install.sh ./
RUN cargo build --release --locked -p taskstaka-site

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 site
COPY --from=build /src/target/release/taskstaka-site /usr/local/bin/taskstaka-site
USER site
ENV PORT=8080 RUST_LOG=info
EXPOSE 8080
CMD ["taskstaka-site"]
