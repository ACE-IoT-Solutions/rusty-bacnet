//! The UI-free operations core of the `bacnet` tool, shared by its one-shot
//! commands, its terminal UI and its MCP server (#2064).
//!
//! Everything here works on a [`BACnetClient`](bacnet_client::client::BACnetClient)
//! and plain data, with no terminal, no ratatui and no MCP dependency, so
//! every front end reads targets, values and devices the same way and
//! obeys the same request limits:
//!
//! - [`class`]: what each command does to a device (read, write or device
//!   management), the table the front ends' safety levels share;
//! - [`target`] and [`destination`]: the target grammar (`192.168.1.10`,
//!   `1234`, `2:1234`, `bip6:0x0004D2`, `mstp:5`, ...) and how a target
//!   becomes a service destination, routed when it must be;
//! - [`parse`]: the object, property and value grammar (`ai:1`,
//!   `present-value[3]`, `72.5@8`);
//! - [`format`](mod@format) and [`label`]: values as display text, and how objects,
//!   properties and priorities are named;
//! - [`limits`]: the global request-rate cap, the one-request-per-device
//!   gate, rates per network and per-caller accounts;
//! - [`decode`]: frames, NPDUs and APDUs decoded into summaries and
//!   field trees, with no traffic;
//! - [`engine`]: the read operations (properties, the object list, a device
//!   summary, trend log pages and alarms) and Who-Is over those limits.
//!
//! The write safety core (plans, ledger, audit) moves here in a later
//! phase; see `docs/design/mcp.md`.

pub mod class;
pub mod decode;
pub mod destination;
pub mod engine;
pub mod format;
pub mod label;
pub mod limits;
pub mod parse;
pub mod target;
