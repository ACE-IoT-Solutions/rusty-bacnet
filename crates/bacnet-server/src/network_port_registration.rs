//! The registered B/IP or B/IPv6 Network Port's address checks and post-bind
//! publication, shared by the full server and the shared endpoint (#1940),
//! so both owners refuse the same addresses and publish the same fields.
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::{Bip6Port, BipPort};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use std::net::{SocketAddrV4, SocketAddrV6};

/// The configured B/IP address a port registers, refused unless it names
/// one concrete unicast interface that is not the link's broadcast address.
pub fn bip_address(port: &BipPort, broadcast: Option<SocketAddrV4>) -> Result<SocketAddrV4, Error> {
    let address = port.endpoint;
    let ip = *address.ip();
    if ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || broadcast.is_some_and(|broadcast| *broadcast.ip() == ip)
    {
        return Err(invalid(
            "registered B/IP requires a concrete unicast interface",
        ));
    }
    Ok(address)
}

/// The configured B/IPv6 address a port registers: one concrete unicast
/// address, as a B/IP link's must be.
pub fn bip6_address(port: &Bip6Port) -> Result<SocketAddrV6, Error> {
    let address = port.endpoint;
    let ip = *address.ip();
    let refused = if ip.is_unspecified() || ip.is_multicast() {
        Some("registered B/IPv6 requires a concrete unicast interface")
    } else if ip.to_ipv4_mapped().is_some() {
        Some("registered B/IPv6 refuses an IPv4-mapped address: register IPv4 as B/IP")
    } else if ip.is_unicast_link_local() {
        // A link-local address needs IPv6_Zone_Index, which isn't modeled.
        Some("registered B/IPv6 refuses a link-local address: no IPv6_Zone_Index is modeled")
    } else {
        None
    };
    match refused {
        Some(message) => Err(invalid(message)),
        None => Ok(address),
    }
}

/// Publish a started B/IP port's bind, mode and multicast group.
pub fn publish_bip(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    port: BipPort,
    capacity: u32,
) -> Result<SocketAddrV4, Error> {
    let address = port.endpoint;
    db.publish_bip_port_internal(
        oid,
        address.ip().octets(),
        address.port(),
        capacity,
        port.mode,
    )?;
    // A port that can use B/IP multicast serves its group (#941).
    db.set_network_port_multicast_internal(oid, port.multicast)?;
    Ok(address)
}

/// Publish a started B/IPv6 port's bind, VMAC, group and mode; the bound
/// address and VMAC it published.
pub fn publish_bip6(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    port: Bip6Port,
    capacity: u32,
) -> Result<(SocketAddrV6, [u8; 3]), Error> {
    let vmac = port
        .vmac
        .ok_or_else(|| invalid("started B/IPv6 port reports no VMAC"))?;
    let address = port.endpoint;
    db.publish_bip6_port_internal(
        oid,
        address.ip().octets(),
        address.port(),
        vmac,
        port.multicast_group.octets(),
        capacity,
        port.mode,
    )?;
    Ok((address, vmac))
}

fn invalid(message: &str) -> Error {
    Error::Encoding(message.into())
}
