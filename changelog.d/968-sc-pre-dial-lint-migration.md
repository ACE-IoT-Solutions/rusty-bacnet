---
section: Migration notes
---
- **BACnet/SC CLI (#968):** an expired or not-yet-valid operational
  certificate, or a key that does not match it, is now refused before the dial
  with a "SC credential check refused the dial" error instead of failing at the
  hub; renew or fix the files. In the TUI, a profile with check warnings
  connects only after `y` accepts them. See
  [Before a dial](docs/sc-check.md#before-a-dial).
