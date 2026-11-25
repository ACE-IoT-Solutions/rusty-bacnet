FROM docker.io/library/python:3.13.18-slim-trixie@sha256:e48f59cdba69a3cbd9ed2cf0f348d90d45a8d350d98134cf1070bc106ee9ed90
RUN apt-get update && apt-get install -y --no-install-recommends tcpdump=4.99.5-2 libpcap0.8t64=1.10.5-2 && rm -rf /var/lib/apt/lists/*
