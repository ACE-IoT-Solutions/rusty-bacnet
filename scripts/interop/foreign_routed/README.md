# Foreign-device lease and routed return path

Run the installed CPython 3.13 Linux SDK against three scripted UDP peers in a new rootless Podman namespace. The BBMD, router and untrusted forwarder have distinct loopback IPs on port 47808. Both endpoint generations bind 127.0.0.1:47809. The endpoint has an explicit Network Port on network 7. An endpoint without a known local number can also ask for it on first hearing a routed device. The container needs only Python 3.13, venv and pip. The launcher verifies mounted inputs in the container and compares the installed native extension with the wheel. Supply the full Git SHA you associate with the wheel as an operator declaration.

```sh
python3 scripts/interop/foreign_routed/run.py --image "$IMAGE" --wheel "$WHEEL" \
  --source "$SDK_SOURCE" --output target/foreign-routed-run
```

The BBMD accepts the endpoint's initial registration and a renewal. Its forwarded I-Am names a remote station on network 5, reached via the router. The probe uses the public arrival fields to build a routed target and checks a ReadProperty exchange on the router socket. An untrusted forwarder sends the same forwarded COV bytes as the BBMD. Distinct accepted values fence the event stream. A confirmed COV admitted by the strict lease receives a routed SimpleACK at the router. The observer verifies delivery, effective source, station and value octets.

The BBMD refuses a renewal, then withholds responses until the original lease expires. The receipt records public registration and transport-health snapshots and a successful routed read while registration is down. Further registrations succeed when replies resume. After close, a replacement foreign device uses the original bind and an eight-second TTL. Multiple renewals distinguish it from the original four-second TTL. The probe checks every datagram delivered to its three sockets, observes them for at least 2.25 seconds after the second close and rebinds the released address. Closing stops local renewals. A remote BBMD can retain an entry until its expiry policy removes it.

`run.json`, `receipt.json` and the container log retain the input hashes, immutable image ID, installed native hash, loopback namespace, status transitions, packets, application observations and cleanup results. These data cover the scripted exchange and finite observation interval. Independent BBMD expiry policy, physical ingress, Edge orchestration and network-wide traffic budgets require separate qualification.
