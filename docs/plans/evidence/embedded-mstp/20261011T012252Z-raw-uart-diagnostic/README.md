# Raw-UART passive diagnostic: 2026-10-11 01:22Z

This bounded diagnostic tested passive factory image
`8f784ce3c5c284532655649c0d64a8e61c94a5db9341378df29f8c057d323519`
on node F. The runner verified USB identity and the known C rollback image
`3c7ae9896efb646b6e8d41c8a3c5bd71ea74cb68072d871a7dae5d6ed9392460`
before flashing. It stopped the dashboard to release the serial port and
restarted it after rollback.

Over the 40-second capture, node F reported:

- UART receive bytes: 976 to 7784;
- valid frames: 122 to 973;
- read, malformed-frame, and codec errors: zero throughout;
- DE: low throughout;
- free heap: 8716772 bytes throughout;
- minimum free heap: 8716244 bytes throughout.

Peer E's receive counter advanced from 1931342 to 1939524 bytes over the same
interval, independently confirming live MS/TP traffic. The raw ESP-IDF UART1
adapter therefore resolves the zero-byte receive failure seen with the
high-level HAL path.

The EXIT-guarded rollback wrote and verified the known C factory image. F then
returned `ok`; 26 seconds after rollback it had received 3598 bytes and
recorded 76 token events. The dashboard service was active again.

This run is intentionally not an acceptance `pass`. The inventory still
pointed to observer C's former address; C was subsequently found healthy at
`192.168.9.110`. No independent logic-analyzer DE trace was captured. The
active image remained unflashed. A repeat passive run must include those
artifacts before the fail-closed evaluator may advance the Phase 6 checklist.

Captured files in this directory are the timestamped F serial log, preflight,
candidate and rollback flash logs, before/after peer snapshots, and restored-F
snapshot. The exact guarded runner is retained as `run-passive-diagnostic.sh`.
Empty `serial-owner.txt` and `serial-capture.err` files prove that no process
retained the port after dashboard shutdown and that capture reported no
host-side errors.
