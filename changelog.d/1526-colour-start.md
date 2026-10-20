---
section: Added
---
- **Wire:** Color and Color Temperature outputs are driven to their default at
  the server's start, a zero default keeps the Present_Value they keep in
  storage (`with_output_persistence`), and In_Progress reads NOT_CONTROLLED
  until the output is driven (#1526).
