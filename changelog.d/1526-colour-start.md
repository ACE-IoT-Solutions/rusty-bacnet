---
section: Added
---
- **Wire:** Color and Color Temperature outputs are driven to their default at
  the server's start; a zero default keeps the stored Present_Value
  (`with_output_persistence`). Objects run without a server now read
  In_Progress NOT_CONTROLLED, not IDLE, until `start_output` or a write (#1526).
