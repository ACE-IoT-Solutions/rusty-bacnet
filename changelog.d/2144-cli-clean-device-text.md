---
section: Security
---
- The one-shot CLI's table output cleans device strings as the terminal UI
  does, so a value, object name or event message can no longer send escape
  sequences, break rows or reorder text with a bidirectional override. JSON
  output keeps the exact text (#2144).
