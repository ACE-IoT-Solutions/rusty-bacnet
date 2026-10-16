# Offline SC credential checks

`bacnet sc-check` reads local credential files and reports certificate inventory,
dates, extension observations, key consistency and advisory Unix key read bits.
It requires the `sc-tls` feature and is available in the development checkout;
older releases may not include it. Build from the repository with:

```sh
cargo install --path crates/bacnet-cli --locked --features sc-tls
bacnet sc-check --sc-cert node.pem
bacnet --sc-cert 'credentials/node identity.pem' sc-check \
  --sc-ca site-ca.pem --sc-key node.key --format table
bacnet sc-check --sc-cert node.pem --at-unix 1792108800 --json
```

The command runs synchronously before runtime, logging and network setup. It does
not create a client, resolve a host, open a socket, prompt for input or change
credentials/permissions. Each selected input is opened and bounded-read once.
The result describes those bytes; later connection startup may read other bytes.

`--sc-cert <FILE>` is required. `--sc-ca <FILE>` and `--sc-key <FILE>` are optional;
omission is explicitly unassessed, while an empty supplied file is an input error.
Paths can appear before or after `sc-check`, including spaces, Unicode and native
OS path representations. Filesystems may reject particular native byte sequences.
Identity entries precede peer-trust entries and keep original zero-based indices.
Identity index 0 is the leaf for dates and key comparison. No chain is sorted and
no trust anchor is selected. The trust file inventories peer-validation material;
it does not identify an authorized issuer for the local identity.

`--at-unix <SECONDS>` accepts the full signed 64-bit range, including negative
values. Omission samples the clock once. This is the only local command option.
`--format table` forces human output through a pipe; `--format json` forces JSON.
Otherwise the terminal gets human output and a pipe gets JSON. `--json` takes
precedence over `--format table`, but an invalid format is always a usage error.
Verbosity flags are accepted without initializing logging.

Explicit transport options are usage errors, even when their value equals the
default: `--interface`, `--port`, `--broadcast`, `--timeout`, `--min-interval-ms`,
`--foreign-bbmd`, `--ttl`, `--renewal`, `--ipv6`, `--ipv6-interface`,
`--device-instance`, `--sc`, `--sc-url`, `--sc-vmac`, and `--sc-device-uuid`.
Help remains available in builds without `sc-tls`; execution then returns 3 with
rebuild advice before opening any input.

## Reports and status

Human output includes every certificate's role/index, full SHA-256, metadata,
validity/expiry, SAN, KU, EKU and findings, followed by key observations and all
diagnostics. Dates use UTC plus exact Unix seconds, or a Unix fallback outside
the calendar formatter's range. Remaining/elapsed time includes days and a
sub-day remainder. Both formats retain escaped text and explicit truncation.

| Exit | Meaning |
| --- | --- |
| 0 | Completed without local errors; warnings and unassessed checks can remain. Also normal help/version success. |
| 1 | Completed with local errors; the complete report is still emitted. |
| 2 | Usage error: missing identity, invalid time/format or explicit transport options. Clap writes stderr; stdout has no result. |
| 3 | Input/I/O/feature execution failure, or output write/flush failure. |

An expired, future or invalid-interval identity leaf is a local error. The same
date observations on other inventory entries are warnings. Seven/thirty-day
expiry urgency is a warning and never changes exit 0 to 1 by itself. The exact
remaining seconds determine urgency: at most 604800 seconds is `alarm`, at most
2592000 is `warning`, only within the inclusive validity interval. Malformed DER
is a completed local error; malformed PEM prevents completion. A mismatched or
unusable supplied key is a local error, while an unknown comparison is a warning.

Exit 0 does not predict successful TLS or establish standards compliance. Issuer
authorization, certificate-path validation, expected peer-name matching,
revocation and remote acceptance remain unassessed. The command does not enforce
SAN/KU/EKU profile requirements, validate signatures or alter TLS startup policy.
TUI/profile use, local-issuer evidence and pre-dial policy remain in
[#968](https://github.com/jscott3201/rusty-bacnet/issues/968).

## JSON fields

After valid dispatch, JSON output is exactly one newline-terminated document.
No tracing, banner or human diagnostic shares stdout. JSON execution failures
use the same top-level fields and leave stderr empty. A write/flush failure
returns 3, attempts a short stderr explanation, and may leave partial stdout;
partial output is not a report. Closed pipes follow this rule without a panic.

| Field | Type and meaning |
| --- | --- |
| `outcome` | String: `completed` or `not_completed`. Local errors can accompany completion. |
| `summary` | Object with integer `errors`, `warnings`, `notices`, counting diagnostics; null on execution failure. |
| `inspection` | Object described below; null on execution failure. |
| `diagnostics` | Array of objects with string `code`, `severity`, `message`, nullable string `role`, nullable integer `index`. Empty on execution failure. |
| `unassessed` | Array: `issuer_authorization`, `certificate_path`, `peer_name_matching`, `revocation`, `remote_acceptance`. |
| `error` | Null when completed; otherwise an execution-error object. |

Inspection fields are integer `inspected_at_unix`, boolean `peer_trust_supplied`,
string `key_consistency`, nullable object `key_file_permissions`, and array
`certificates`. Key consistency is `not_assessed`, `match`, `mismatch`, `unusable`
or `unknown`. Permission objects contain booleans `group_readable` and
`world_readable`; omission and non-Unix platforms yield null. These are mode bits
of the opened key file; ACLs are not assessed.

Certificate fields are string `role` (`identity` or `peer_trust`), integer `index`,
64-character lowercase hex `sha256`, nullable object `metadata`, string `validity`,
string `expiry`, three extension objects (`san`, `key_usage`,
`extended_key_usage`), and array `findings`.

Metadata fields are strings `subject`, `issuer`, `public_key_algorithm_oid`,
`signature_algorithm_oid`; signed integers `not_before_unix`, `not_after_unix`;
and boolean `text_truncated`. Timestamps are exact signed 64-bit integers. Use an
integer-capable JSON consumer; floating-point conversion may lose precision.

Validity is `unavailable`, `invalid_interval`, `not_yet_valid`, `expired` or
`within_interval`. Expiry is `not_assessed`, `none`, `warning` or `alarm`.
Extension objects contain booleans `assessed`, `present` and string array `values`.
`assessed=false` cannot establish absence. Findings contain string `area` and
`kind`: areas are `certificate`, `san`, `key_usage`, `extended_key_usage`; kinds
are `malformed`, `duplicate`, `unsupported`, `truncated`.

The error object contains string `category` (`input`, `io`, `feature_unavailable`),
nullable string `role`, nullable integer `index`, nullable string `kind`, nullable
integer `os_code`, and bounded string `message`. Input kinds are `input_too_large`,
`certificate_too_large`, `too_many_certificates`, `empty_certificates`,
`malformed_pem`, `missing_private_key`, `clock_out_of_range`. Input errors retain
role/index when available and have null `os_code`. I/O errors retain role and
native OS code when available, with null `index`/`kind`. Feature errors have
null role/index/kind/OS code. Supplied paths, source-error text and credentials
are excluded. Parse/help output is ordinary Clap output, never a JSON result.

## Diagnostic codes

Codes and enum spellings identify conditions independently of English messages.
Object-key ordering, indentation and wording are not parsing contracts. Derived
diagnostics sort by identity, peer trust, then private key; within each role they
sort by index, area and code. Certificate indices retain input order.

| Code | Severity and meaning |
| --- | --- |
| `certificate.malformed` | Error: a decoded inventory item could not be interpreted. |
| `certificate.truncated` | Warning: extension-count work bound; extension presence may be unassessed. |
| `certificate.duplicate`, `certificate.unsupported` | Warning if supplied by the native report. |
| `san.malformed`, `san.duplicate`, `san.unsupported`, `san.truncated` | Warning for that extension's interpretation limitation. |
| `key_usage.malformed`, `key_usage.duplicate`, `key_usage.unsupported`, `key_usage.truncated` | Warning for that extension's interpretation limitation. |
| `extended_key_usage.malformed`, `extended_key_usage.duplicate`, `extended_key_usage.unsupported`, `extended_key_usage.truncated` | Warning for that extension's interpretation limitation. |
| `validity.invalid_interval`, `validity.not_yet_valid`, `validity.expired` | Error for identity index 0; warning for any other certificate. |
| `validity.unavailable` | Notice: no interval assessment was possible. |
| `expiry.alarm`, `expiry.warning` | Warning: seven/thirty-day renewal urgency. |
| `metadata.text_truncated` | Warning: bounded metadata display was truncated. |
| `san.not_assessed`, `key_usage.not_assessed`, `extended_key_usage.not_assessed` | Notice: presence/values were not fully assessed. |
| `key.not_assessed` | Notice: no key was supplied. |
| `key.mismatch`, `key.unusable` | Error: supplied key mismatch or unusable representation. |
| `key.unknown` | Warning: comparison did not establish a match. |
| `key_permissions.not_assessed` | Notice: Unix read bits unavailable or no key supplied. |
| `key_permissions.group_readable`, `key_permissions.world_readable` | Warning: the corresponding mode bit was observed. |
| `peer_trust.not_assessed` | Notice: no peer-trust file supplied. |

Absent SAN/KU/EKU and matching keys add no diagnostic. Metadata truncation is
separate from extension findings. All codes in the table are projected by the
command; some native enum combinations are reserved rather than currently
produced by the parser.

Complete illustrative documents are linked for
[success](sc-check-examples/success.json),
[five-day warning](sc-check-examples/warning.json),
[completed local error](sc-check-examples/local-error.json), and
[execution error](sc-check-examples/execution-error.json). They were produced
with an ephemeral example identity/key and explicit times; the certificate
fingerprint is illustrative, not an operational credential.

## Bounds

The existing inspection API bounds selected input to 1 MiB total, 64 certificates
and 128 KiB per decoded certificate. Each certificate permits 64 extensions and
16 KiB of selected extension data. Each extension display keeps at most 16 values,
each at most 256 escaped ASCII bytes; four metadata strings each permit 1024
bytes. Findings preserve truncation or unsupported interpretation explicitly.
JSON structure and escaping add bytes, so serialized output is not limited to
1 MiB. PEM, DER and private key contents are never output.

For API behavior and qualification limits, see the
[shared inspection guide](sc-offline-inspection.md). Consumer tests exercise
literal statuses and complete JSON, human output, bounded errors, native paths,
extension limits, hostile text, deterministic ordering and output failures.
