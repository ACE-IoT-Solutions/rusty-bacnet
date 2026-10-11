# Extended MS/TP primary-source dossier

Date reviewed: 2026-10-11

Decision: official ASHRAE addenda provide sufficient primary evidence to
implement and test the X1/X2 wire primitives. The absent consolidated
Standard 135-2020 PDF remains required before raising conformance status.

## Primary sources

- [ANSI/ASHRAE Addendum an to Standard 135-2012](https://bacnet.org/wp-content/uploads/sites/4/2022/08/Add-135-2012-an-at-au-av-aw-ax-az-Final-1.pdf),
  approved 2014-07-03. This introduces the Clause 9.10 COBS procedures,
  CRC-32K and Annex X algorithms/vectors.
- [ANSI/ASHRAE Addendum ce to Standard 135-2020](https://bacnet.org/wp-content/uploads/sites/4/2023/10/135_2020_ce_20220121.pdf),
  approved 2022-01-21. This confirms the 135-2020 DER/DNER frame-type and
  decoded-payload rules.

## Implementation requirements

| ID | Requirement | Source anchor |
|---|---|---|
| X-N1 | `0x20` is Extended Data Expecting Reply; `0x21` is Extended Data Not Expecting Reply. | 135-2020ce §§9.3.9–9.3.10 |
| X-N2 | DER/DNER carry 502–1497 decoded data octets; standard `0x05`/`0x06` remain at most 501. | 135-2020ce §§9.3.6–9.3.10 |
| X-N3 | Raw frame types `0x20..=0x7f` use the COBS/CRC-32K framing envelope, but only `0x20`/`0x21` are BACnet extended NPDU types. | 135-2012an §9.3 |
| X-N4 | Header Length is `encoded_data_length + encoded_crc_length - 2`; encoded CRC is five octets, so Length is `encoded_data_length + 3`. | 135-2012an §§9.3, 9.10.2 |
| X-N5 | Generic COBS header Length is 5–2043. BACnet extended maximum Length is 1506 and encoded fields require at most 1508 octets. | 135-2012an §§9.5.3–9.5.4 |
| X-N6 | Standard COBS zero elimination applies. Code `0xff` represents 254 data octets without an implicit zero or final phantom zero. | 135-2012an §9.10.1 |
| X-N7 | XOR every COBS output octet with `0x55`. Encode the data and CRC fields separately. CRC covers the transmitted encoded-data octets. | 135-2012an §9.10.2 and Annex X.1 |
| X-N8 | CRC-32K uses initial `0xffff_ffff`, reflected polynomial `0xeb31_d82e`, final complement, little-endian transmission and receive residue `0x0843_323b`. | 135-2012an §9.6.3 and Annex G.3 |
| X-N9 | A zero unmasked COBS code or a code extending beyond the field is malformed. The five-octet encoded CRC must decode to exactly four octets. | 135-2012an §9.10.3 and Annex X.2 |
| X-N10 | The receiver consumes `Length + 2` encoded-field octets and must skip invalid/oversize frames deterministically for resynchronization. | 135-2012an §§9.5.4.4, 9.5.4.8–9.5.4.9 |

## Official seed vectors

- CRC-32K input `01 22 30`: running CRC `83 dd 5a 41`; complemented
  little-endian transmitted CRC `be a5 22 7c`; receive residue `08 43 32 3b`
  (135-2012an Annex G.3.1).
- Client bytes `48 65 6c 6c 6f 20 57 6f 72 6c 64 0a 00`: encoded data
  `58 1d 30 39 39 3a 75 02 3a 27 39 31 5f 54`; separately encoded CRC field
  `50 60 82 25 19`; header Length 17 (135-2012an Annex X.1).
- Annex X.4 supplies a full `55 ff 21 ...` extended DNER frame fixture for the
  later one-shot and streaming codec tests.

## Code implications

- Add allocation-free COBS/XOR and CRC-32K primitives in
  `bacnet-mstp-codec` before changing frame dispatch.
- Add named `0x20` and `0x21` variants only. Structurally valid
  `0x22..=0x7f` frames must not be delivered as BACnet NPDUs.
- Extended decode needs caller-owned scratch or in-place transformation; the
  immutable borrowed standard decoder cannot expose decoded COBS bytes.
- Keep standard and extended storage profiles explicit. Required extended
  maxima are 1497 decoded data octets, 1508 encoded-field octets and 1516
  complete unpadded wire octets.

## Remaining evidence gap

The configured `_spec/2020_ASHRAE_Standard-135-BACnet-Data-Communication-Protocol.pdf`
is absent. Before claiming consolidated 135-2020 conformance, reconcile Clause
9.3, 9.5, 9.6, 9.10 and Annexes G/X against that edition and any later adopted
addenda. Independent interoperability evidence also remains required.
