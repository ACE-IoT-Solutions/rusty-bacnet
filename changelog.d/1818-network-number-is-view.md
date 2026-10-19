---
section: Changed
---
- The non-router network layer reads a Network-Number-Is through the router
  core's `message::NetworkNumberIs` view, the one parser of that payload;
  data of any length but three octets is still refused (#1818).
