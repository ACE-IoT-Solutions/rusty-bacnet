---
title: "Configure BACnet/SC"
description: "Treat certificate trust, local identities, and build features as explicit prerequisites."
---

BACnet/SC setup combines a WebSocket/TLS connection with BACnet-specific identities and hub behavior. A successful TCP or TLS connection alone does not establish a complete BACnet/SC deployment.

This guide covers CLI, Python and Rust setup, trust roles, durable device identity and failure stages. The [upgrade guide](/rusty-bacnet/project/upgrading/#bacnetsc-security-serverpolicy-and-principals) lists the release changes.

## Before connecting

Obtain the hub URL, the CA certificate that verifies the hub, your node's certificate and private key, and an allocated VMAC and device UUID. Ensure the hub's certificate identity matches the hub name you are using and that the system clock is appropriate for certificate validation.

Keep private keys outside the repository and the generated site. Example names below are not deployable credentials.

## Prepare identities and credentials before connecting

Obtain these from the installation owner:

- The hub's `wss://` URL and the CA PEM used to verify it, including the expected hostname.
- This node's operational certificate and matching private key.
- A nonreserved local VMAC and a nonzero 16-byte device UUID, distinct from other devices.
- A known remote VMAC, object and property for a bounded initial read.

The caller provisions and durably stores the device UUID before first deployment, then loads the same identity across restarts. Do not generate a new UUID in startup code. Distinct devices must not share it; the hub's same-UUID replacement behavior is intentional. Credentials and UUIDs are different roles: a syntactically valid UUID does not establish certificate ownership.

## CLI feature and identity setup

Every [release CLI executable](/rusty-bacnet/start/installation/#install-a-cli-executable) includes BACnet/SC. For a [source build](/rusty-bacnet/start/installation/#prefer-a-source-build), enable the `sc-tls` feature:

```sh
cargo install --path crates/bacnet-cli --locked --features sc-tls
```

The CLI requires a hub URL, the hub's CA, a client certificate and private key, a local VMAC, and a nonzero local device UUID:

```sh
bacnet --sc \
  --sc-url wss://hub.example.com/bacnet \
  --sc-ca site-ca.pem \
  --sc-cert node-cert.pem \
  --sc-key node-key.pem \
  --sc-vmac 22:01:02:03:04:05 \
  --sc-device-uuid 00112233-4455-6677-8899-aabbccddeeff \
  read 00:01:02:03:04:05 ai:1 pv
```

Replace every identity and path with an approved value for your network. The final VMAC is the **remote** target; `--sc-vmac` is the **local** node identity. Do not reuse the example VMAC or UUID throughout a deployment.

The read targets the named peer without discovery. Check the [CLI transport reference](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md#transport-variants) for parser and startup failures.

### Trust the hub's issuer, not the system roots

`--sc-ca` names the CA PEM that verifies the hub's certificate. The CLI has no fallback to the operating system's root certificates, so a private hub CA needs no change to the host's trust store.

Do not “fix” certificate failures by disabling certificate validation. A client certificate supplied through `--sc-cert` is not the same thing as trusting the issuer of the hub's certificate.

## Python trust configuration

The Python client and server take `sc_ca_cert`, `sc_client_cert`, `sc_client_key` and a keyword-only `sc_device_uuid`; all four are required for `transport="sc"`. The hub (`ScHub`) takes `ca_cert` for trusted node issuers. These are different roles: a node trusts the hub's issuer, and the hub validates connecting node identities using its configured trust.

A hub always requires `ca_cert` and verifies each node's certificate; there is no server-auth-only mode. Use the Python API and secure-connect example linked below for constructor details rather than translating CLI flags mechanically into Python names.

## Use the same prerequisites in Python and Rust

Python `BACnetClient` and `BACnetServer` with `transport="sc"` require nonempty `sc_ca_cert`, `sc_client_cert`, `sc_client_key` and keyword-only `sc_device_uuid`. Load the latter as 16 bytes from caller-owned durable storage. Missing or invalid identity/credential presence fails construction; file loading and TLS configuration happen at async startup. A trusted hub's verification policy remains a separate fact.

The [Python credential example](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#required-operational-credentials) and [UUID migration](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#sc-device-uuid-migration) are the signature authority. Local TLS configuration errors can be repaired at the same paths and retried within the documented boundary; this does not promise rollback after every later peer/dial failure.

Rust uses `ScNodeTlsConfig` and the transport's configured device identity. Raw `ScTransport::new(ws, vmac)` is initially unconfigured; set a valid UUID before start. Because the caller can dial `ws` first, transport validation cannot undo that earlier connection. A shared endpoint uses `ScEndpointBuilder` with a caller-dialed TLS WebSocket; see [shared endpoints](/rusty-bacnet/integrate/shared-endpoints/).

## Diagnose the stage that failed

| Stage | Useful check |
|---|---|
| Local configuration | CA/certificate/key file readability and PEM validity, matching key, required UUID and VMAC |
| TLS connection | URL, hostname, certificate validity and issuer trust; operational credentials on both peers |
| SC Connect | Nonzero peer UUID and nonzero advertised limits; hub admission/replacement policy |
| BACnet request | Remote VMAC, requested object/property, request limits and response/error |
| Shutdown | Await the owning client/server/session cleanup; separately stop and join any caller-owned direct listener |

Zero-limit rejection is a bounded local policy, not a claim that all positive limit combinations satisfy the full Standard. TLS 1.3 and explicit node credentials are local policy; successful connection does not prove that an arbitrary remote hub requested and verified the node certificate.

## Keep transport identity separate from operation authority

Accepted direct TLS connections can carry a verified leaf identity and original-socket response authority in the native Rust path. A Hub-relayed NPDU does not become an authenticated originating leaf. Outgoing client transactions retain their documented correlation/routing rules, not a universal same-leaf continuity guarantee. Python does not expose a direct-listener entry point or a principal authorizer.

Passive Number replies, including replies to direct queries, use the Hub broadcast path. These controls do not add configured SC Network Port authority or control-origin authorization. Keep application mutation authorization distinct from TLS trust.

## Validate the whole path

Check the package/build feature, hub URL and port, certificate chain and validity, hostname, local VMAC/UUID, hub acceptance, and remote target identity. After joining the hub, perform a bounded read against a known object. Record sanitized failure stages; never attach a private key to an issue.

These instructions explain configuration boundaries. They are not a certificate-authority operating procedure or a BACnet/SC certification claim.

## Sources and release scope

These instructions target **v0.14.0**. Source review is not a claim of hardware qualification.

[CLI SC transport](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/CLI.md#transport-variants) · [CLI flags](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/crates/bacnet-cli/src/args.rs) · [Python credentials](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/python-api.md#required-operational-credentials) · [SC example](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/examples/python/sc_secure_connect.py).

## Next steps

[SC Rust contracts](https://github.com/jscott3201/rusty-bacnet/blob/v0.14.0/docs/rust-api.md#bacnetsc-client-transport) · [Shared endpoints](/rusty-bacnet/integrate/shared-endpoints/) · [Transport evidence](/rusty-bacnet/reference/transports/) · [Network Number controls](/rusty-bacnet/integrate/network-ports/)
