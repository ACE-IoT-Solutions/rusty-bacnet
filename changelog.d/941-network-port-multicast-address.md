---
section: Added
---
- Every registered NORMAL B/IP Network Port on an explicit interface now
  lists BACnet_IP_Multicast_Address (0.0.0.0 while off), writable with
  ReinitializeDevice activation, which moves the running node to the new
  group, and read-only without a handler (#941).
