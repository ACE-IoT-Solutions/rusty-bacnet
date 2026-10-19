---
section: Added
---
- Two opt-in router hardening options: rejects that change a learned route only from its next router (`next_hop_rejects`), and suppression of a global broadcast repeated within 2 s (`duplicates`). Ignored flow control and dropped repeats are counted (#1698).
