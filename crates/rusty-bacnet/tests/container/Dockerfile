FROM rust:1.93-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        cmake \
        python3 \
        python3-dev \
        python3-pip \
        python3-venv \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN python3 -m venv /build-venv \
    && /build-venv/bin/pip install --no-cache-dir "maturin>=1,<2" \
    && /build-venv/bin/maturin build \
        --release \
        --manifest-path crates/rusty-bacnet/Cargo.toml \
        --interpreter /build-venv/bin/python \
        --out /dist

FROM python:3.11-slim-bookworm AS runtime

COPY --from=builder /dist/rusty_bacnet-*.whl /tmp/
RUN python -m pip install --no-cache-dir /tmp/rusty_bacnet-*.whl \
    && rm /tmp/rusty_bacnet-*.whl

COPY crates/rusty-bacnet/tests/container/discovery_fixture.py /fixture.py

ENTRYPOINT ["python", "/fixture.py"]
