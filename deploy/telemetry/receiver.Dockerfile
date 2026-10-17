FROM docker.io/library/rust:1.99.1-slim-bookworm AS build

RUN apt-get update \
	&& apt-get install -y --no-install-recommends gcc libc6-dev \
	&& rm -rf /var/lib/apt/lists/*
WORKDIR /source
COPY Cargo.toml Cargo.lock README.md LICENSE ./
COPY crates ./crates
ARG CARGO_BUILD_JOBS=2
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} CARGO_PROFILE_RELEASE_STRIP=symbols
RUN cargo build --locked --release -p nixbox-core --features telemetry-receiver --example telemetry-receiver

FROM docker.io/library/debian:bookworm-slim
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates curl \
	&& rm -rf /var/lib/apt/lists/* \
	&& useradd --system --uid 10001 --user-group --no-create-home telemetry \
	&& install -d -o telemetry -g telemetry -m 0700 /data
COPY --from=build /source/target/release/examples/telemetry-receiver /usr/local/bin/telemetry-receiver
ENV NIXBOX_TELEMETRY_BIND=0.0.0.0:8787 NIXBOX_TELEMETRY_DATABASE=/data/counts.sqlite3
USER telemetry:telemetry
WORKDIR /data
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
	CMD ["curl", "--fail", "--silent", "--max-time", "2", "http://127.0.0.1:8787/health"]
ENTRYPOINT ["/usr/local/bin/telemetry-receiver"]
