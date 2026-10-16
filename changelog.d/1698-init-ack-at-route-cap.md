---
section: Fixed
---
- At the 256-route cap, an Initialize-Routing-Table-Ack refreshes the
  networks the router's table holds, as an I-Am-Router-To-Network does; a
  network it doesn't hold is still refused and counted (#1698).
