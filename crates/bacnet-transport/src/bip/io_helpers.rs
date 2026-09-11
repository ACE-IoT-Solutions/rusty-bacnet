/// Send a BVLC-Result to a destination.
async fn send_bvlc_result(socket: &UdpSocket, dest: ([u8; 4], u16), code: BvlcResultCode) {
    let payload = code.to_raw().to_be_bytes().to_vec();
    let mut buf = BytesMut::with_capacity(6);
    if let Err(e) = encode_bvll(&mut buf, BvlcFunction::BVLC_RESULT, &payload) {
        warn!(error = %e, "Failed to encode BVLC-Result");
        return;
    }
    let addr = SocketAddrV4::new(Ipv4Addr::from(dest.0), dest.1);
    let _ = socket.send_to(&buf, addr).await;
}

#[cfg(test)]
fn should_force_dbtn_forward_failure(ctx: &RecvContext) -> bool {
    ctx.force_dbtn_forward_failure
}

#[cfg(not(test))]
fn should_force_dbtn_forward_failure(_ctx: &RecvContext) -> bool {
    false
}

async fn send_forwarded_npdu(
    socket: &UdpSocket,
    dest: SocketAddrV4,
    orig_ip: [u8; 4],
    orig_port: u16,
    npdu: &[u8],
) -> bool {
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
        warn!(error = %e, "Failed to encode Forwarded-NPDU");
        return false;
    }

    match socket.send_to(&buf, dest).await {
        Ok(_) => true,
        Err(e) => {
            warn!(error = %e, dest = %dest, "Failed to send Forwarded-NPDU");
            false
        }
    }
}

/// Forward an NPDU as Forwarded-NPDU to a list of targets.
///
/// Yields between sends for large target lists to avoid starving the recv loop
/// when there are many FDT entries (up to 512).
async fn forward_npdu(
    socket: &UdpSocket,
    npdu: &[u8],
    orig_ip: [u8; 4],
    orig_port: u16,
    targets: &[([u8; 4], u16)],
) -> bool {
    if targets.is_empty() {
        return true;
    }
    let mut buf = BytesMut::with_capacity(10 + npdu.len());
    if let Err(e) = encode_bvll_forwarded(&mut buf, orig_ip, orig_port, npdu) {
        warn!(error = %e, "Failed to encode Forwarded-NPDU");
        return false;
    }
    let frame = buf.freeze();
    let mut all_sent = true;

    for (i, &(ip, port)) in targets.iter().enumerate() {
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);
        if let Err(e) = socket.send_to(&frame, dest).await {
            all_sent = false;
            warn!(error = %e, dest = %dest, "Failed to forward NPDU");
        }
        // Yield every 32 sends to let the recv loop process incoming packets
        if i % 32 == 31 {
            tokio::task::yield_now().await;
        }
    }
    all_sent
}

/// Resolve the local IPv4 address by connecting a UDP socket to a remote
/// address and reading back the local address. This doesn't actually send
/// any packets.
pub(super) fn resolve_local_ip() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Connect to a public IP — doesn't actually send anything
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(*v4.ip()),
        _ => None,
    }
}
