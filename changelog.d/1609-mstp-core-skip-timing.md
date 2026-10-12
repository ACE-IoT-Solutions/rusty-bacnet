---
section: Changed
---
- The MS/TP core counts a silence or receive error in the middle of a
  dropped frame the codec skips as an invalid frame, as the receive
  machine's skip state does, and counts an extended frame it can't take at
  its last octet instead of at its header (#1609, #1611).
