# Factory-owned build/test image for the local-container executor. Every input is pinned.
FROM ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG RUST_VERSION=1.96.0
ARG RUSTUP_VERSION=1.29.2
ARG RUSTUP_SHA256=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash ca-certificates curl gcc libc6-dev mingw-w64 python3 python3-venv \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
RUN curl -fsSL -o /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
 && chmod +x /tmp/rustup-init \
 && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}" \
      --target x86_64-pc-windows-gnu \
 && rm /tmp/rustup-init \
 && chmod -R a+rX /opt/rustup /opt/cargo

COPY requirements.txt /opt/factory/requirements.txt
RUN python3 -m venv /opt/pytest \
 && /opt/pytest/bin/pip install --no-cache-dir --require-hashes -r /opt/factory/requirements.txt

COPY run-acceptance.sh vendor-crates.sh /opt/factory/
RUN chmod 755 /opt/factory/*.sh \
 && useradd --create-home --uid 10001 runner \
 && mkdir -p /work /out /build /vendor \
 && chown runner:runner /out /build /vendor

# Agents' code runs as `runner`; the toolchain, pytest and the copied workspace stay root-owned.
ENV PATH=/opt/pytest/bin:/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin \
    CARGO_HOME=/home/runner/.cargo \
    RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_TARGET_DIR=/build/target \
    PYTHONDONTWRITEBYTECODE=1
USER runner
WORKDIR /work
