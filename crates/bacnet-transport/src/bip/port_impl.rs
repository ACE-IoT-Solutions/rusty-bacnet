impl TransportPort for BipTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        if self.recv_task.is_some() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "BIP transport already started",
            )));
        }
        if self
            .foreign_device
            .as_ref()
            .is_some_and(|configuration| configuration.ttl == 0)
        {
            return Err(Error::Encoding(
                "foreign-device registration TTL must be non-zero".into(),
            ));
        }

        let socket2 = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .map_err(Error::Transport)?;

        set_socket_reuse_address(&socket2, self.reuse_port).map_err(Error::Transport)?;
        set_socket_reuse_port(&socket2, self.reuse_port).map_err(Error::Transport)?;
        socket2.set_broadcast(true).map_err(Error::Transport)?;
        socket2.set_nonblocking(true).map_err(Error::Transport)?;

        // Validate self.interface is a real local IP before binding the real
        // socket to 0.0.0.0 below. Previously the kernel enforced this when
        // we bound directly to self.interface (EADDRNOTAVAIL on typos); now
        // we probe with a throwaway bind on an ephemeral port so a
        // misconfigured interface fails fast at startup rather than silently
        // advertising an unowned IP in I-Am replies via local_mac. See
        // tests::start_fails_on_nonlocal_interface.
        if !self.interface.is_unspecified() {
            std::net::UdpSocket::bind(SocketAddrV4::new(self.interface, 0))
                .map_err(Error::Transport)?;

            #[cfg(unix)]
            if self.reuse_port {
                if let Some((_interface_name, _interface_index)) =
                    resolve_ipv4_interface(self.interface)
                {
                    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
                    socket2
                        .bind_device(Some(&_interface_name))
                        .map_err(Error::Transport)?;

                    #[cfg(any(
                        target_os = "ios",
                        target_os = "visionos",
                        target_os = "macos",
                        target_os = "tvos",
                        target_os = "watchos",
                        target_os = "illumos",
                        target_os = "solaris"
                    ))]
                    socket2
                        .bind_device_by_index_v4(std::num::NonZeroU32::new(_interface_index))
                        .map_err(Error::Transport)?;

                    #[cfg(not(any(
                        target_os = "android",
                        target_os = "fuchsia",
                        target_os = "linux",
                        target_os = "ios",
                        target_os = "visionos",
                        target_os = "macos",
                        target_os = "tvos",
                        target_os = "watchos",
                        target_os = "illumos",
                        target_os = "solaris"
                    )))]
                    let _ = (_interface_name, _interface_index);
                }
            }
        }

        // Always bind to INADDR_ANY so subnet- and limited-broadcast packets
        // (destination 10.x.y.255 / 255.255.255.255) reach this socket.  A
        // Linux UDP socket bound to a specific interface IP only receives
        // packets whose destination IP matches the bound IP, so binding to
        // self.interface would silently drop every inbound broadcast — see
        // tests::socket_is_broadcast_capable_and_binds_inaddr_any. `self.interface`
        // is still used below for the announced local MAC (line 318), so I-Am
        // responses continue to advertise the correct source IP.
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.port);
        socket2.bind(&bind_addr.into()).map_err(Error::Transport)?;

        let destination_receiver =
            DestinationReceiver::configure(&socket2, IpVersion::V4).map_err(Error::Transport)?;

        let std_socket: std::net::UdpSocket = socket2.into();
        let socket = UdpSocket::from_std(std_socket).map_err(Error::Transport)?;

        let wildcard_bind = self.interface.is_unspecified();
        let local_ip = if wildcard_bind {
            resolve_local_ip().unwrap_or(Ipv4Addr::LOCALHOST)
        } else {
            self.interface
        };
        let local_unicast_ips = if wildcard_bind {
            crate::local_addresses::ipv4()
        } else {
            vec![local_ip]
        };
        #[cfg(unix)]
        if wildcard_bind && local_unicast_ips.is_empty() {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "could not enumerate local IPv4 addresses for wildcard ingress",
            )));
        }

        let local_port = socket.local_addr().map_err(Error::Transport)?.port();
        self.port = local_port;

        self.local_mac = encode_bip_mac(local_ip.octets(), local_port);

        let socket = Arc::new(socket);
        self.socket = Some(Arc::clone(&socket));

        if let Some(config) = self.bbmd_config.take() {
            let mut state = BbmdState::new(local_ip.octets(), local_port);
            // Try loading persisted BDT; fall back to initial config BDT on
            // missing/unreadable files, structural decode failure, or semantic
            // validation/conflict failure. A valid persisted BDT wins; an
            // invalid configured fallback fails startup via `set_bdt` below.
            let initial_bdt = if let Some(ref path) = self.bdt_persist_path {
                match std::fs::read(path) {
                    Ok(data) => match BbmdState::decode_bdt(&data) {
                        Ok(entries) => {
                            let mut probe = BbmdState::new(local_ip.octets(), local_port);
                            match probe.set_bdt(entries) {
                                Ok(()) => {
                                    debug!(
                                        path = %path.display(),
                                        entries = probe.bdt().len(),
                                        "Loaded persisted BDT"
                                    );
                                    probe.bdt().to_vec()
                                }
                                Err(e) => {
                                    warn!(error = %e, "Persisted BDT invalid, using config");
                                    config.initial_bdt
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode persisted BDT, using config");
                            config.initial_bdt
                        }
                    },
                    Err(_) => config.initial_bdt,
                }
            } else {
                config.initial_bdt
            };
            if let Err(e) = state.set_bdt(initial_bdt) {
                return Err(Error::Encoding(format!("BDT configuration error: {e}")));
            }
            state.set_management_acl(config.management_acl);
            state.set_foreign_device_policy(config.foreign_device_policy);
            state.set_accept_foreign_devices(config.accept_foreign_devices);
            state
                .set_max_fdt_entries(config.max_fdt_entries)
                .expect("pre-start FDT capacity was validated");
            let bbmd = Arc::new(Mutex::new(state));
            if let Some(core) = &self.bbmd_control_core {
                core.attach(
                    &bbmd,
                    self.bdt_persist_path.clone(),
                    &self.management_limiter,
                    &self.fanout_counters,
                );
            }
            self.bbmd = Some(bbmd);
        }

        /// NPDU receive channel capacity for high-throughput UDP transports.
        const NPDU_CHANNEL_CAPACITY: usize = 256;

        let (npdu_tx, rx) = mpsc::channel(NPDU_CHANNEL_CAPACITY);

        let (fanout_tx, fanout_rx) = mpsc::channel(self.fanout_policy.queue_capacity.max(1));
        let fanout_task = tokio::spawn(fanout::run_fanout_worker(
            Arc::clone(&socket),
            fanout_rx,
            Arc::clone(&self.fanout_counters),
        ));
        self.fanout_task = Some(fanout_task);

        let fanout_dispatcher = fanout::FanoutDispatcher::new(
            fanout_tx,
            Arc::clone(&self.fanout_limiter),
            Arc::clone(&self.fanout_counters),
        );

        let recv_ctx = RecvContext {
            local_mac: self.local_mac,
            socket: Arc::clone(&socket),
            npdu_tx,
            bbmd: self.bbmd.clone(),
            broadcast_addr: self.broadcast_address,
            broadcast_port: self.port,
            pending_bvlc_response: self.pending_bvlc_response.clone(),
            bvlc_result_quarantine: Arc::clone(&self.bvlc_result_quarantine),
            management_limiter: Arc::clone(&self.management_limiter),
            fanout: Some(fanout_dispatcher),
            bvll_policy: self.bvll_policy.clone(),
            #[cfg(test)]
            force_dbtn_forward_failure: false,
        };

        let recv_task = tokio::spawn(async move {
            let mut recv_buf = vec![0u8; 2048];
            loop {
                match destination_receiver
                    .recv_from(&recv_ctx.socket, &mut recv_buf)
                    .await
                {
                    Ok(received) => {
                        let data = &recv_buf[..received.len];
                        match decode_bvll(data) {
                            Ok(msg) => {
                                if !original_destination_matches(
                                    msg.function,
                                    received.destination,
                                    local_ip,
                                    recv_ctx.broadcast_addr,
                                    &local_unicast_ips,
                                    wildcard_bind,
                                    received.os_group_delivery,
                                ) {
                                    debug!(
                                        function = msg.function.to_raw(),
                                        destination = %received.destination,
                                        "Dropping BVLL/IP destination mismatch"
                                    );
                                    continue;
                                }
                                let sender_addr =
                                    if let std::net::SocketAddr::V4(v4) = received.peer {
                                        (v4.ip().octets(), v4.port())
                                    } else {
                                        continue;
                                    };

                                handle_bvll_message(&msg, sender_addr, &recv_ctx).await;
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to decode BVLL frame");
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                        debug!(error = %e, "Dropping UDP datagram with invalid destination metadata");
                    }
                    Err(e) if recoverable_udp_receive_error(&e) => {
                        // An unconnected UDP socket can surface an asynchronous
                        // ICMP error from one unreachable peer through recvmsg.
                        // It describes one datagram, not the local socket's
                        // lifecycle; keep receiving so managed foreign-device
                        // registration can recover when its BBMD restarts.
                        debug!(error = %e, "Ignoring recoverable UDP peer receive error");
                    }
                    Err(e) => {
                        warn!(error = %e, "UDP recv error");
                        break;
                    }
                }
            }
        });

        self.recv_task = Some(recv_task);

        if let Some(bbmd) = self.bbmd.clone() {
            self.bbmd_fdt_purge_task = Some(Self::spawn_bbmd_fdt_purge_task(bbmd));
        }

        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let ttl = fd.ttl;
            let sock = self.socket.as_ref().unwrap().clone();
            let handle = self
                .foreign_device_registration
                .clone()
                .expect("foreign-device configuration initializes telemetry");
            let reg_task = tokio::spawn(
                foreign_device::RegistrationWorker {
                    socket: sock,
                    bbmd_addr,
                    ttl,
                    handle,
                    pending: Arc::clone(&self.pending_bvlc_response),
                    request_lock: Arc::clone(&self.bvlc_request_lock),
                    next_request_id: Arc::clone(&self.next_bvlc_request_id),
                    quarantine: Arc::clone(&self.bvlc_result_quarantine),
                    response_timeout: Self::BVLC_RESPONSE_TIMEOUT,
                }
                .run(),
            );
            self.registration_task = Some(reg_task);
        }

        self.start_committed = true;
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        if let Some(core) = &self.bbmd_control_core {
            core.stop();
        }
        for task in self.abort_background_tasks() {
            let _ = task.await;
        }
        Ok(())
    }

    fn abort(&mut self) {
        if let Some(core) = &self.bbmd_control_core {
            core.stop();
        }
        let _ = self.abort_background_tasks();
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        let socket = self.require_socket()?;

        let (ip, port) = decode_bip_mac(mac)?;
        let dest = SocketAddrV4::new(Ipv4Addr::from(ip), port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_UNICAST_NPDU, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        let socket = self.require_socket()?;

        if let Some(fd) = &self.foreign_device {
            let bbmd_addr = SocketAddrV4::new(fd.bbmd_ip, fd.bbmd_port);
            let mut buf = BytesMut::with_capacity(4 + npdu.len());
            encode_bvll(
                &mut buf,
                BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
                npdu,
            )?;
            socket
                .send_to(&buf, bbmd_addr)
                .await
                .map_err(Error::Transport)?;
            return Ok(());
        }

        let dest = SocketAddrV4::new(self.broadcast_address, self.port);

        let mut buf = BytesMut::with_capacity(4 + npdu.len());
        encode_bvll(&mut buf, BvlcFunction::ORIGINAL_BROADCAST_NPDU, npdu)?;

        socket.send_to(&buf, dest).await.map_err(Error::Transport)?;

        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn bip_network_port_observation(&self) -> Option<crate::port::BipNetworkPortObservation> {
        Some(crate::port::BipNetworkPortObservation::new(
            self.bbmd_control().map(|control| control.snapshot_reader()),
        ))
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        // Clause J.1.2's B/IP broadcast address is the configured broadcast
        // IP ("all 1's in the host portion") together with this port's UDP
        // port — a broadcast IP at a different port belongs to a different
        // B/IP network and must not be folded into this link's broadcast.
        mac.len() == 6
            && mac[..4] == self.broadcast_address.octets()
            && mac[4..] == self.port.to_be_bytes()
    }
}
impl Drop for BipTransport {
    fn drop(&mut self) {
        self.abort();
    }
}
