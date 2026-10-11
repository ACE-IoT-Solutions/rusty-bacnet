// ===========================================================================
// BVLC enums for BACnet/IPv6 (Annex U). The BACnet/IPv4 function and result
// codes of Annex J live in `bacnet-bip-codec`.
// ===========================================================================

bacnet_enum! {
    /// BACnet/IPv6 BVLC function codes (Annex U, Table U-1).
    pub struct Bvlc6Function(u8);

    const BVLC_RESULT = 0x00;
    const ORIGINAL_UNICAST_NPDU = 0x01;
    const ORIGINAL_BROADCAST_NPDU = 0x02;
    const ADDRESS_RESOLUTION = 0x03;
    const FORWARDED_ADDRESS_RESOLUTION = 0x04;
    const ADDRESS_RESOLUTION_ACK = 0x05;
    const VIRTUAL_ADDRESS_RESOLUTION = 0x06;
    const VIRTUAL_ADDRESS_RESOLUTION_ACK = 0x07;
    const FORWARDED_NPDU = 0x08;
    const REGISTER_FOREIGN_DEVICE = 0x09;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY = 0x0A;
    const DISTRIBUTE_BROADCAST_TO_NETWORK = 0x0C;
}

bacnet_enum! {
    /// BACnet/IPv6 BVLC-Result codes (Annex U.2.1.1).
    pub struct Bvlc6ResultCode(u16);

    const SUCCESSFUL_COMPLETION = 0x0000;
    const ADDRESS_RESOLUTION_NAK = 0x0030;
    const VIRTUAL_ADDRESS_RESOLUTION_NAK = 0x0060;
    const REGISTER_FOREIGN_DEVICE_NAK = 0x0090;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK = 0x00A0;
    const DISTRIBUTE_BROADCAST_TO_NETWORK_NAK = 0x00C0;
}
