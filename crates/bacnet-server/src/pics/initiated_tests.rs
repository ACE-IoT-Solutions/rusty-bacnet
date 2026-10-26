//! How the PICS initiator column relates to what `bacnet-client` sends.
//!
//! [`PicsGenerator::INITIATED_SERVICES`] lists the announcements and
//! notifications the server builds and sends itself. `bacnet-client` sends
//! requests on an application's behalf
//! ([`SENT_SERVICES`](bacnet_client::client::SENT_SERVICES), which its own
//! tests tie to its methods). The server never sends through the client, so
//! the client list can't vouch for the initiator column, and the two don't
//! overlap. The conformance page shows them as separate columns for that
//! reason. If a service ever belongs in both, update this test and the page's
//! wording together.

use std::collections::BTreeSet;

use bacnet_client::client::SENT_SERVICES;
use bacnet_types::enums::ServiceSupported;

use super::*;

fn bits(services: &[ServiceSupported]) -> BTreeSet<u8> {
    services.iter().map(|service| service.to_raw()).collect()
}

#[test]
fn the_server_initiates_nothing_the_client_sends() {
    let initiated = bits(PicsGenerator::INITIATED_SERVICES);
    assert_eq!(
        initiated.len(),
        PicsGenerator::INITIATED_SERVICES.len(),
        "no initiated service listed twice"
    );
    let both: Vec<_> = initiated
        .intersection(&bits(SENT_SERVICES))
        .map(|bit| service_display_name(ServiceSupported::from_raw(*bit)))
        .collect();
    assert!(
        both.is_empty(),
        "the PICS initiator column and the client's sent list overlap: {both:?}"
    );
}

#[test]
fn every_initiated_service_has_a_pics_name() {
    for service in PicsGenerator::INITIATED_SERVICES {
        assert_ne!(service_display_name(*service), "Unknown", "{service:?}");
    }
}
