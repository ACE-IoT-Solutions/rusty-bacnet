# Separate-UID shared-port qualification

Two Rusty B/IP attachments run in separate rootless Podman containers on the
Linux engine's host network. The controller runs in a third host-network
container and uses BACpypes3 0.0.106 to encode directed-broadcast Who-Is and
unicast ReadProperty requests and decode the answers. Its socket binds an
explicit local source address and an ephemeral port. The controller queries
only the Device IDs and addresses supplied for this run.

Use Python 3.11 or later on the host. The QA image needs CPython 3.13 with
`venv` and pip, the wheel's runtime libraries, and BACpypes3 0.0.106 at
`/opt/qa/bin/python`. The five SDK containers install only the mounted wheel
in fresh environments. The probe never installs host packages. For a remote
Podman connection, the engine needs read-only bind-mount access to the fixture
directory and wheel at the specified paths.

```sh
mkdir -p target/host-network
python3 scripts/interop/host_network/run.py --wheel "$RUSTY_WHEEL" \
  --source-sha "$RUSTY_SOURCE_SHA" --output target/host-network/sample.json
```

By default, the addresses are `127.0.0.1/8` and `127.0.0.2/8` for Devices
7101 and 7102, with source `127.0.0.10/8` and shared UDP port 47808.
`--image` selects the QA image and `--connection` selects an existing Podman
connection. On a Mac with a Podman VM, host networking refers to the Linux
guest. For a deployment run, supply all of `--cidr-a`, `--cidr-b`,
`--controller-cidr`, `--device-a` and `--device-b` explicitly. Use authorized
preassigned unicast addresses with their actual interface prefixes and unused
Device IDs. `--port` selects the common B/IP port. The runner leaves interface
configuration, VM state and existing containers alone.

Before launching either SDK worker, the controller checks that it can bind
the two primary addresses, their directed broadcasts and the limited broadcast
on the requested port with exclusive sockets. It releases the preflight sockets
before starting the attachments. A busy socket ends the run and records the
Linux errno. Concurrent activity after preflight can still cause startup to
fail. Successful workers must report their requested primary address, active
primary reception and every required broadcast listener active.

The controller uses one restricted Who-Is range when two consecutive Device
IDs share a directed broadcast. Otherwise it queries each owned ID separately.
It checks the UDP source IP and port of each I-Am and ReadProperty ACK and
validates the returned object identifier, property, invoke and Device name.
A third UID must receive `EADDRINUSE` when trying the first live address.
After the first owner closes, a replacement in a new container tries the
second owner's live address and must receive the same error. The neighbor
remains readable. A fresh container restores the first address with a new
Device name; both names are read again before close and a final bind check.

Before executing any mounted fixture module or installing the wheel, the
controller and each worker hash the mounted Python sources and wheel from
inside the container. The runner supplies the expected local digests as
command arguments and records the actual mounted digests. A different file
at the same absolute path on a remote engine stops that container before it
opens a BACnet socket.

The JSON receipt records each container ID and inspected network mode, effective
UID, kernel namespace IDs and UID map. The process map must agree with the
rootless engine map. It also records the operator-declared source commit, wheel
and installed native extension hashes, readiness, packets and errors. Cleanup
removes only IDs created by this invocation. Failure returns a nonzero exit
and preserves the available evidence with the original error.

## Observed Linux arm64 loopback results

The development wheel from `97015c3d3` had SHA-256
`63958ccb72b6ba44f76bb477ac57b6b98501354d36d7742f38f4503b4efb8265`.
The two attached processes ran as container UIDs 10001 and 10002, mapping
to Linux host UIDs 110000 and 110001. Their independent containers and the
controller shared guest network namespace `net:[4026531840]`. Both workers
reported two active broadcast listeners. One bounded Who-Is yielded both
I-Ams from the respective primary addresses on port 47808. Both bind
conflicts returned Linux errno 98, the neighbor retained its Device name,
and both identities matched after restoration. The final exclusive socket
checks passed. A separate busy-at-entry control exited after preflight and
left its original socket holder running.

This exercises separate attachments on the Podman VM's host loopback.
Ingress isolation across physical interfaces, deployed x86_64/aarch64 hosts
and an application's generation-aware attachment manager require their own
measurements.
