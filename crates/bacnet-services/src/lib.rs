#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

//! BACnet service request/response encode/decode per ASHRAE 135-2020.
//!
//! Each module covers one or more related BACnet services with request/response
//! structs and encode/decode functions. Service data at constructed boundaries
//! remains as raw bytes (`Vec<u8>`) — the application layer interprets
//! property values, not the service codec.
//!
//! The service codecs require allocation for decoded sequences and owned text,
//! but do not require an operating system. They can be used with
//! `--no-default-features --features alloc` on a `no_std` target:
//!
//! ```
//! extern crate alloc;
//!
//! use bacnet_services::who_is::WhoIsRequest;
//! use bytes::BytesMut;
//!
//! let request = WhoIsRequest::all();
//! let mut wire = BytesMut::new();
//! request.encode(&mut wire);
//! assert!(wire.is_empty());
//! ```

extern crate alloc;

pub(crate) use alloc::format;
pub(crate) use alloc::string::{String, ToString};
pub(crate) use alloc::vec::Vec;

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("bacnet-services requires either the `std` or `alloc` feature");

pub mod alarm_event;
pub mod alarm_summary;
pub mod audit;
pub mod common;
pub mod cov;
mod cov_decode;
pub mod cov_multiple;
pub mod device_mgmt;
pub mod enrollment_summary;
pub mod file;
pub mod life_safety;
pub mod list_manipulation;
pub mod object_mgmt;
pub mod private_transfer;
pub mod read_property;
pub mod read_range;
pub mod rpm;
pub mod schedule;
pub mod text_message;
pub mod virtual_terminal;
pub mod who_am_i;
pub mod who_has;
pub mod who_is;
pub mod wpm;
pub mod write_group;
pub mod write_property;
