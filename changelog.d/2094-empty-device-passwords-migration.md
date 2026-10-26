---
section: Migration notes
---
- **Empty DCC and reinitialize passwords (#2094):** pass `None` instead of an
  empty string. The request encoders return an encode error, a
  `ServerConfig`, endpoint or server builder with an empty `dcc_password` or
  `reinit_password` fails to start, the Python constructors and client calls
  raise ValueError, and the CLI refuses `--password ""`.
