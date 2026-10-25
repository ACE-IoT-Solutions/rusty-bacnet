//! The read operations every front end shares, over one client and one set
//! of request limits.
//!
//! [`Reader`] is the reading strategy for one device: each request waits
//! for its turn at the device's gate and the global rate cap
//! ([`RequestLimits`](crate::limits::RequestLimits)), a property list goes
//! by ReadPropertyMultiple with a ReadProperty fallback, and the
//! Object_List falls back to element-by-element reads. The TUI's browser,
//! watch list and write previews use it directly, with their own stop
//! flags.

mod read;

use bacnet_types::enums::PropertyIdentifier;

pub use self::read::{
    Halt, ListRead, MAX_OBJECTS, ReadVia, Reader, RpmFailure, Wanted, rpm_failure, unreachable,
};

/// The Device properties a device summary reads (Clause 12.11): who made
/// it, what it runs, and what it can do. The TUI's device card shows them.
pub const DEVICE_PROPERTIES: [PropertyIdentifier; 13] = [
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::VENDOR_NAME,
    PropertyIdentifier::VENDOR_IDENTIFIER,
    PropertyIdentifier::MODEL_NAME,
    PropertyIdentifier::FIRMWARE_REVISION,
    PropertyIdentifier::APPLICATION_SOFTWARE_VERSION,
    PropertyIdentifier::PROTOCOL_VERSION,
    PropertyIdentifier::PROTOCOL_REVISION,
    PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED,
    PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED,
    PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
    PropertyIdentifier::SEGMENTATION_SUPPORTED,
    PropertyIdentifier::DATABASE_REVISION,
];
