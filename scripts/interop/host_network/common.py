"""JSON-line records for the separately owned rootless qualification processes."""

import errno
import json
import os
from pathlib import Path


def emit(record):
    print(json.dumps(record), flush=True)


def identity():
    uid = os.getuid()
    mapping = [list(map(int, line.split())) for line in Path("/proc/self/uid_map").read_text().splitlines()]
    host_uid, = [host + uid - local for local, host, count in mapping if local <= uid < local + count]
    return {"uid": uid, "uid_map": mapping, "mapped_uid": host_uid,
            "net_namespace": os.readlink("/proc/self/ns/net"), "user_namespace": os.readlink("/proc/self/ns/user")}


def failure(error):
    code = getattr(error, "errno", None)
    return {"type": type(error).__name__, "message": str(error), "errno": code, "errno_name": errno.errorcode.get(code)}
