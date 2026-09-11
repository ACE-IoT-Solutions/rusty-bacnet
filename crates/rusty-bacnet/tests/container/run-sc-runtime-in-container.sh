#!/bin/sh
set -eu

openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout /tmp/sc-ca-key.pem \
    -out /tmp/sc-ca-cert.pem \
    -subj /CN=rusty-bacnet-test-ca \
    -addext 'basicConstraints=critical,CA:TRUE' >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes \
    -keyout /tmp/sc-server-key.pem \
    -out /tmp/sc-server.csr \
    -subj /CN=localhost >/dev/null 2>&1
printf '%s\n' \
    'basicConstraints=critical,CA:FALSE' \
    'subjectAltName=DNS:localhost,IP:127.0.0.1' \
    'extendedKeyUsage=serverAuth' > /tmp/sc-server.ext
openssl x509 -req -days 1 \
    -in /tmp/sc-server.csr \
    -CA /tmp/sc-ca-cert.pem \
    -CAkey /tmp/sc-ca-key.pem \
    -CAcreateserial \
    -out /tmp/sc-server-cert.pem \
    -extfile /tmp/sc-server.ext >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes \
    -keyout /tmp/sc-client-key.pem \
    -out /tmp/sc-client.csr \
    -subj /CN=rusty-bacnet-client >/dev/null 2>&1
printf '%s\n' \
    'basicConstraints=critical,CA:FALSE' \
    'extendedKeyUsage=clientAuth' > /tmp/sc-client.ext
openssl x509 -req -days 1 \
    -in /tmp/sc-client.csr \
    -CA /tmp/sc-ca-cert.pem \
    -CAkey /tmp/sc-ca-key.pem \
    -CAcreateserial \
    -out /tmp/sc-client-cert.pem \
    -extfile /tmp/sc-client.ext >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout /tmp/sc-rogue-key.pem \
    -out /tmp/sc-rogue-cert.pem \
    -subj /CN=untrusted-client \
    -addext 'basicConstraints=critical,CA:FALSE' \
    -addext 'extendedKeyUsage=clientAuth' >/dev/null 2>&1
exec python /fixture.py \
    /tmp/sc-ca-cert.pem \
    /tmp/sc-server-cert.pem \
    /tmp/sc-server-key.pem \
    /tmp/sc-client-cert.pem \
    /tmp/sc-client-key.pem \
    /tmp/sc-rogue-cert.pem \
    /tmp/sc-rogue-key.pem
