# Normal B/IP loopback packet audit

Run each profile in its own rootless Linux Podman container. It needs only loopback and `CAP_NET_RAW`. The controller, SDK owner and tcpdump share that private network namespace. The fixture uses UDP port 47810 for the SDK and 47811 for its scripted peer. No ports are published.

Build the capture image and supply a locally available Linux CPython 3.13 wheel that matches the image architecture. The recipe pins a multi-architecture Python base and the capture package versions. The run records the actual local image ID, Python version, tcpdump/libpcap versions, namespace and UID map. The wheel source SHA is operator-declared. The runner hashes the wheel and fixture on the host, verifies the mounted bytes inside the container before running them, and compares the installed extension with the archive member.

```sh
podman build -t localhost/rusty-bacnet-capture-qa:py313 scripts/interop/traffic_capture
python3 scripts/interop/traffic_capture/run.py --owner shared \
  --image localhost/rusty-bacnet-capture-qa:py313 --wheel "$WHEEL" \
  --source "$WHEEL_SOURCE_SHA" --output target/capture-shared
python3 scripts/interop/traffic_capture/run.py --owner standalone \
  --image localhost/rusty-bacnet-capture-qa:py313 --wheel "$WHEEL" \
  --source "$WHEEL_SOURCE_SHA" --output target/capture-standalone
```

The output directory must be new. With `--connection`, the Podman host must be able to mount the specified input paths. Output, including failures, is copied from the owned container before its removal.

## Expected exchange

1. Start unfiltered tcpdump on `lo` before constructing the SDK. Observe a nonce-bearing start marker as a complete capture record. Start a shared B/IP endpoint with receive segmentation and max APDU 480 or a standalone client with its default response budget.
2. Send idle boundary markers around the discovery-table read, shared health snapshot, local expectation creation and withdrawal, and a two-second idle window. Set `--idle-seconds` for a different finite interval. Only markers are expected in this phase.
3. Explicitly direct Who-Is to the peer for Device 200. With an unknown local network number, the first discovery sends **two** outbound datagrams in order. The SDK broadcasts What-Is-Network-Number on its local B/IP port, then sends the directed Who-Is. The peer returns I-Am. A second directed Who-Is elicits another I-Am without repeating the network-number question. The peer provides no Network-Number-Is answer. The client verifies both device identities and link senders.
4. Read Description and read a two-selector RPM. Register a strict expectation for process 7 and AI 1. Send SubscribeCOV, then one explicit renewal with a different lifetime.
5. Receive a confirmed COV in four segments with window two. The peer waits for ACK 0 before segment 1, ACK 2 before segment 3, and the final ACK 3 before the terminal SimpleACK. The event carries 250 UCS-4 characters and its original encoded bytes. Unsubscribe and close the lease. A subsequent confirmed notification for the closed lease gets the unknown-subscription Error and produces no event before the iterator ends.
6. Stop the owner and close the scripted peer. Send the end marker, wait for its complete record on disk, then interrupt tcpdump. Success requires a clean capture exit, explicit zero kernel drops, complete classic Ethernet pcap records, and an exact ordered comparison with the expected exchange. Invoke IDs bind to the corresponding confirmed request. The audit checks UDP endpoints and full BACnet datagram bytes and preserves duplicates. The kernel's received-by-filter statistic has platform-specific counting semantics.

`loopback.pcap` retains the complete capture. `receipt.json` records the container's capture statistics, observed values, action names, payload hashes and timestamps. `run.json` records the host audit, cleanup, source and image provenance, and offline controls. `inputs.json` records the independently calculated mount hashes. For a passing run the offline verifier removes an RP or the end marker, repeats an ACK, truncates a record and changes an Ethernet type. Each altered copy must fail for its expected reason.

Repeat a command with `--inject-unexpected` and a fresh output directory for the negative control. The fixture sends a separate UDP packet to a bound sink during idle, completes the workload, and exits nonzero. Inspect the offending frame in the retained pcap and audit error.

This measures traffic during one declared operation sequence and idle interval. Deployment acceptance still needs a comparison with the downstream traffic budget and legacy trace, deployed interfaces, and the Edge polling and renewal policy. FDR, BBMD operation, learned routes, SC, physical ingress isolation and performance measurements require separate runs.
