#![doc = include_str!("../README.md")]

// Tests always build with std, so feature-off test runs need no imports of
// their own.
#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

#[cfg(not(any(feature = "std", feature = "alloc")))]
compile_error!("bacnet-services needs the `std` or the `alloc` feature");

// `format!` and `vec!` without std's prelude.
#[macro_use]
extern crate alloc;

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
pub mod structured_error;
pub mod text_message;
pub mod virtual_terminal;
pub mod who_am_i;
pub mod who_has;
pub mod who_is;
pub mod wpm;
pub mod write_group;
pub mod write_property;

#[cfg(test)]
mod malformed_member_tests;
#[cfg(test)]
mod reject_reason_tests;
#[cfg(test)]
mod truncated_member_tests;
