"""Run one Rusty attachment through start and close as the container's actual UID."""

import asyncio
import hashlib
from importlib import import_module, metadata
from ipaddress import IPv4Interface
import json
from pathlib import Path
import sys
import zipfile

import rusty_bacnet as rb
from common import emit, failure, identity


async def main():
    wheel, cidr, port, instance, name = sys.argv[1:]
    interface = IPv4Interface(cidr)
    native = Path(import_module("rusty_bacnet.rusty_bacnet").__file__)
    with native.open("rb") as fp:
        installed = hashlib.file_digest(fp, "sha256").hexdigest()
    with zipfile.ZipFile(wheel) as archive, archive.open("rusty_bacnet/" + native.name) as fp:
        expected = hashlib.file_digest(fp, "sha256").hexdigest()
    if installed != expected:
        raise ValueError("installed native binary differs from mounted wheel")
    record = {"identity": identity(), "native_sha256": installed, "sdk_version": metadata.version("ace-rusty-bacnet"),
              "cidr": cidr, "port": int(port), "instance": int(instance), "name": name}
    endpoint = rb.BipEndpoint(int(instance), device_name=name, vendor_id=999, interface=str(interface.ip),
                              broadcast_address=str(interface.network.broadcast_address), port=int(port),
                              share_port_by_address=True)
    try:
        try:
            await endpoint.start()
        except Exception as error:
            emit({**record, "started": False, "error": failure(error)})
            return
        emit({**record, "started": True, "address": await endpoint.local_address(),
              "health": (await endpoint.status())["transport_health"]})
        command = await asyncio.to_thread(sys.stdin.readline)
        if command and json.loads(command) != {"op": "close"}:
            raise ValueError("unexpected worker command")
    finally:
        await endpoint.close()
    emit({"closed": True})


if __name__ == "__main__":
    asyncio.run(main())
