//! Rewriting a frame for the next hop (Clause 6.5.4).

use super::config::Options;
use super::counters::bump;
use super::host::Host;
use super::ingress::{Arrival, DropReason, Received};
use super::output::{Kind, Target};
use super::Storage;
use crate::message::reason;
use crate::npdu::{Address, Npdu};
use crate::table::Route;

/// The frame's SNET and SADR, or, when it has none, the arrival network and
/// the link source: the originator is on the arrival link.
fn source_of<'f>(at: &Arrival<'f>, npdu: &Npdu<'f>) -> Option<Address<'f>> {
    npdu.source.or(Some(Address {
        network: at.network,
        mac: at.source,
    }))
}

impl<const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>
    Storage<P, R, H, D, N>
{
    /// Send `npdu` on along `route`. Onto a directly attached network it
    /// loses its DNET, DADR and hop count and goes to the DADR, or as a
    /// broadcast for DLEN 0; toward another router it keeps them, with the
    /// hop count one lower, and goes to that router. SNET and SADR are
    /// added when missing, and the rest travels unchanged.
    ///
    /// One that arrives with a hop count of 0 goes nowhere, and so does one
    /// whose route names a port the router doesn't have. One that
    /// arrives with 1 leaves with 0, as in the hosted router (fix R6 in
    /// #1679 changes that later). With R1 on, one too long for the egress
    /// port draws reject reason 4 naming its DNET, or, when `answer` is
    /// off (a reject passing through), is only dropped.
    pub(super) fn forward<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        route: &Route,
        options: &Options,
        host: &mut impl Host,
        answer: bool,
    ) -> Received<'f> {
        if npdu.hop_count == 0 {
            bump(&mut self.counters.hop_count_drops);
            return Received::Dropped(DropReason::HopCount);
        }
        // A route written through `Router::table_mut` may name a port the
        // router doesn't have; the hosted router drops its traffic too.
        let Some(egress) = self.ports[..self.port_count].get(usize::from(route.port.0)) else {
            bump(&mut self.counters.unknown_port);
            return Received::Dropped(DropReason::UnknownPort);
        };
        let limits = egress.max_npdu;
        let (destination, hop_count, to) = if route.direct {
            let dadr = npdu
                .destination
                .map_or(&[][..], |destination| destination.mac);
            (None, 0, dadr)
        } else {
            (
                npdu.destination,
                npdu.hop_count - 1,
                route.next_hop.as_slice(),
            )
        };
        let rewritten = Npdu {
            destination,
            source: source_of(at, npdu),
            hop_count,
            ..*npdu
        };
        if options.fixes.egress_limits {
            let limit = limits.get(to.is_empty());
            if rewritten.encoded_len() > limit {
                bump(&mut self.counters.egress_too_long);
                let dnet = npdu.destination.map(|destination| destination.network);
                return match dnet.filter(|_| answer) {
                    Some(dnet) => {
                        self.reject(at, npdu.source, dnet, reason::MESSAGE_TOO_LONG, host);
                        Received::Handled
                    }
                    None => Received::Dropped(DropReason::TooLong),
                };
            }
        }
        if self.write_now(&rewritten, Kind::Forward, Target::Toward(route.port, to)) {
            Received::Handled
        } else {
            Received::Dropped(DropReason::Unwritable)
        }
    }

    /// Pass a global broadcast on to every port but the arrival one, with
    /// DNET 0xFFFF and DLEN 0 kept, SNET and SADR added when missing, and
    /// the hop count one lower; nothing when it has run out. With R1 on, a
    /// port that can't carry it is left out and counted when polled, with
    /// no reject (decision D21 on #1679).
    pub(super) fn forward_global(&mut self, at: &Arrival<'_>, npdu: &Npdu<'_>) {
        if npdu.hop_count == 0 {
            bump(&mut self.counters.hop_count_drops);
            return;
        }
        let rewritten = Npdu {
            source: source_of(at, npdu),
            hop_count: npdu.hop_count - 1,
            ..*npdu
        };
        self.write_now(&rewritten, Kind::Global, Target::AllBut(at.port));
    }
}
