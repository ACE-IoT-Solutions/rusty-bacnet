"""Reject mutations of a real passing capture using the production offline audit."""
import hashlib

from pcap import audit, records
from oracle import scenario


def rejected_mutations(raw, owner, nonce, idle):
    packets = records(raw)
    expected = scenario(owner, nonce)
    audit(raw, owner, nonce, idle)
    pieces = []
    offset = 24
    for item in packets:
        end = offset + 16 + len(item.data)
        pieces.append(raw[offset:end])
        offset = end
    rp = next(i for i, p in enumerate(expected) if p.name == "rp")
    ack = next(i for i, p in enumerate(expected) if p.name == "segment_ack2")
    first = bytearray(pieces[0])
    first[16 + 12:16 + 14] = b"\x86\xdd"  # Ethernet type IPv6
    controls = [
        ("truncated_record", raw[:-1], "truncated record body"),
        ("omitted_rp", raw[:24] + b"".join(pieces[:rp] + pieces[rp + 1:]), "rp:"),
        ("repeated_segment_ack", raw[:24] + b"".join(pieces[:ack] + [pieces[ack]] + pieces[ack:]), "cov3:"),
        ("unclassified_ethertype", raw[:24] + first + b"".join(pieces[1:]), "unclassified Ethernet"),
        ("omitted_end_marker", raw[:-len(pieces[-1])], "capture ended before end"),
    ]
    results = []
    for name, data, reason in controls:
        try:
            audit(data, owner, nonce, idle)
        except ValueError as error:
            if reason not in str(error):
                raise ValueError(f"{name}: unexpected audit failure: {error}") from error
            results.append(dict(name=name, result="rejected", expected=reason, error=str(error),
                                mutated_sha256=hashlib.sha256(data).hexdigest()))
        else:
            raise ValueError(f"audit accepted the {name} control")
    return results
