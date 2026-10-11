//! Static component sizes, not stack high-water or firmware image estimates.

use bacnet_mstp_core::{ExtendedMasterCore, HeaplessQueues, StandardMasterCore};
use bacnet_router_core::{
    BroadcastFanoutResult, BroadcastPolicy, DuplicateCache, NetworkNumber, OperationalDiagnostics,
    RouteDiscovery, RouteEntry, RouteTable, RouterControlAction, RouterCore,
};
use std::mem::size_of;

fn profile<const R: usize, const C: usize, const P: usize, const Q: usize>() {
    let routes = size_of::<RouteTable<R, R>>();
    let router = size_of::<RouterCore<R, R>>();
    let discovery = size_of::<RouteDiscovery<R>>();
    let cache = size_of::<DuplicateCache<C>>();
    let policy = size_of::<BroadcastPolicy<C>>();
    let diagnostics = size_of::<OperationalDiagnostics<P, R, C>>();
    let actions = size_of::<[BroadcastFanoutResult; P]>();
    let control_action = size_of::<RouterControlAction<'static>>();
    let network_scratch = 2 * size_of::<[NetworkNumber; R]>();
    let route_scratch = 2 * size_of::<[RouteEntry; R]>();
    let control_wire_scratch = 1 + 4 * R;
    let standard_queues = size_of::<HeaplessQueues<Q, Q, 501, 2>>();
    let extended_queues = size_of::<HeaplessQueues<Q, Q, 1497, 2>>();
    let standard_core = size_of::<StandardMasterCore<'static, HeaplessQueues<Q, Q, 501, 2>>>();
    let extended_core = size_of::<ExtendedMasterCore<'static, HeaplessQueues<Q, Q, 1497, 2>>>();
    // Router contains route storage and its forwarding counters. Broadcast
    // policy contains the cache; MS/TP core contains queues and transmit storage.
    // Add each contained structure only once in the illustrative total.
    let shared = router
        + discovery
        + policy
        + diagnostics
        + actions
        + control_action
        + network_scratch
        + route_scratch
        + control_wire_scratch;
    let standard_total = shared + standard_core + 501 + 511;
    let extended_total = shared + extended_core + 1497 + 1516;
    println!("memory R={R} C={C} P={P} Q={Q}: routes={routes} router={router} discovery={discovery} cache={cache} policy={policy} diagnostics={diagnostics} fanout_actions={actions} control_action={control_action} network_scratch={network_scratch} route_scratch={route_scratch} control_wire_scratch={control_wire_scratch} standard_queues={standard_queues} extended_queues={extended_queues} standard_core={standard_core} extended_core={extended_core} core_delta={} standard_composition={standard_total} extended_composition={extended_total}", extended_core - standard_core);
}

pub fn print() {
    println!(
        "memory_bytes target_pointer_width={} (Rust host layout)",
        usize::BITS
    );
    profile::<2, 8, 2, 1>();
    profile::<8, 32, 4, 4>();
    profile::<32, 128, 8, 8>();
}
