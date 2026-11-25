# Two-interface broadcast qualification

Run a Linux CPython 3.13 wheel in rootless Podman with the Netavark bridge driver. The image must provide Python 3.13, venv and pip.

```sh
python3 scripts/interop/distinct_links/run.py --image "$IMAGE" --wheel "$WHEEL" \
  --source "$SDK_SOURCE" --output target/distinct-links-run
```

The launcher inspects existing bridge subnets and the engine host's nondefault IPv4 routes. It chooses two unused RFC1918 /28s or validates the pair supplied with `--subnets A B`. It creates two internal DNS-disabled networks, a dual-homed SUT and one peer per link. Network attachments and run labels are checked by ID. Inputs are mounted read-only and verified before execution, and the imported native extension is compared with the wheel. Each endpoint resolves its assigned interface name, binds its own IPv4 address on port 47808 and requires healthy subnet and limited-broadcast listeners.

Each peer uses a distinct source port per simulated Device and sends distinguishable I-Am and unconfirmed COV messages first to the limited broadcast, then its subnet broadcast, then the endpoint unicast. The probe checks each admitted discovery table and COV stream after each phase, drains the streams through close, and requires both opposite-index warnings from the limited broadcast phase. Each peer also verifies a ReadProperty response from its endpoint's address and port. The receipt records the actual namespaces, interfaces, kernel indices, SDK and fixture hashes, messages, health and resource cleanup. Offline controls exercise the admission oracle with a wrong ingress index, a foreign COV and a missing device.

This finite observation covers two disjoint virtual links. Physical interface behavior, overlapping routes, duplicate addresses and performance require separate qualification.
