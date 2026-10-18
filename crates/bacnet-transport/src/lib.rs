#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod any;
pub mod bbmd;
pub mod bip;
#[cfg(feature = "ipv6")]
#[cfg_attr(docsrs, doc(cfg(feature = "ipv6")))]
pub mod bip6;
pub mod bvll;
mod direct_response;
#[cfg(feature = "ethernet")]
#[cfg_attr(docsrs, doc(cfg(feature = "ethernet")))]
pub mod ethernet;
mod local_addresses;
pub mod loopback;
pub mod mstp;
#[cfg(feature = "serial")]
#[cfg_attr(docsrs, doc(cfg(feature = "serial")))]
pub mod mstp_serial;
mod npdu_channel;
pub mod port;
mod port_ownership;
mod runtime_clock;
pub mod sc;
pub mod sc_frame;
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub mod sc_hub;
mod sc_limits;
#[cfg(feature = "sc-tls")]
#[cfg_attr(docsrs, doc(cfg(feature = "sc-tls")))]
pub mod sc_tls;
#[cfg(feature = "sc-tls")]
mod tcp_connect;
#[cfg(feature = "sc-tls")]
mod tls_reject;
mod udp_metadata;
pub mod warn_throttle;
#[cfg(windows)]
mod windows_adapters;

#[cfg(test)]
mod log_capture;
#[cfg(test)]
#[path = "rb07_provenance_tests.rs"]
mod rb07_provenance_tests;
