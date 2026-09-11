use super::*;

pub(super) fn spawn_dispatch_task<T: TransportPort + 'static>(
    mut apdu_rx: mpsc::Receiver<bacnet_network::layer::ReceivedApdu>,
    network_dispatch: Arc<NetworkLayer<T>>,
    db_dispatch: Arc<RwLock<ObjectDatabase>>,
    cov_dispatch: Arc<RwLock<CovSubscriptionTable>>,
    seg_ack_dispatch: Arc<segmented_send::SegmentedSendRegistry>,
    seg_send_permits_dispatch: Arc<Semaphore>,
    cov_in_flight_dispatch: Arc<Semaphore>,
    server_tsm_dispatch: Arc<Mutex<ServerTsm>>,
    notification_transactions_dispatch: Arc<NotificationTransactions>,
    confirmed_request_tracker_dispatch: Arc<ConfirmedRequestTracker>,
    device_bindings_dispatch: Arc<RwLock<DeviceBindingTable>>,
    comm_state_dispatch: Arc<AtomicU8>,
    dcc_timer_dispatch: Arc<Mutex<Option<JoinHandle<()>>>>,
    dcc_outcomes_dispatch: Arc<dcc_outcomes::DccOutcomes>,
    config_dispatch: Arc<ServerConfig>,
    clock_dispatch: Option<Arc<ServerClock>>,
    discovery_limiter_dispatch: Arc<DiscoveryLimiter>,
    requests: Arc<request_tasks::RequestTasks>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
            let mut seg_receivers: HashMap<SegKey, SegmentedRequestState> = HashMap::new();
            let mut notifications_open = true;
            let mut ingress_open = true;

            loop {
                let received = tokio::select! {
                    result = notification_transactions_dispatch.join_next(), if notifications_open => {
                        notifications_open = result.is_some();
                        NotificationTransactions::observe(result);
                        continue;
                    }
                    result = requests.join_next(), if !requests.is_empty() => {
                        super::request_tasks::RequestTasks::observe(result);
                        continue;
                    }
                    received = apdu_rx.recv(), if ingress_open => match received {
                        Some(received) => received,
                        None => {
                            // Local/periodic notification producers outlive ingress.
                            // Keep both join consumers active, but never poll EOF again.
                            ingress_open = false;
                            seg_receivers.clear();
                            continue;
                        }
                    },
                    else => break,
                };
                let now = Instant::now();
                super::segmented_receive::expire_segmented_requests(&mut seg_receivers, now);

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        let source_mac = received.source_mac.clone();
                        let source_network = received.source_network.clone();

                        // Clause 5.4.5.2 AbortPDU_Received: a peer's Abort
                        // ('server' = FALSE) ends any reassembly session for
                        // its transaction. A side effect, not a short
                        // circuit — the PDU still reaches `dispatch`, whose
                        // Abort arm cancels in-flight segmented response
                        // senders and records server-TSM results (#377).
                        if let Apdu::Abort(ref abt) = decoded {
                            if !abt.sent_by_server {
                                let key = segmented_transaction_key(
                                    source_mac.as_slice(),
                                    source_network.as_ref(),
                                    abt.invoke_id,
                                );
                                seg_receivers.remove(&key);
                            }
                        }

                        let mut received = Some(received);
                        let handled = if let Apdu::ConfirmedRequest(ref req) = decoded {
                            if req.segmented {
                                let seq = req.sequence_number.unwrap_or(0);
                                let key = segmented_transaction_key(
                                    source_mac.as_slice(),
                                    source_network.as_ref(),
                                    req.invoke_id,
                                );

                                // Clause 5.4.5.1
                                // ConfirmedSegmentedReceivedNotSupported: a
                                // device that does not support segmented
                                // reception answers segment traffic with this
                                // Abort instead of reassembling — the
                                // configured Segmentation value is the
                                // advertisement peers plan transfers around
                                // (#381).
                                let receives_segments = config_dispatch.segmentation_supported
                                    == Segmentation::BOTH
                                    || config_dispatch.segmentation_supported
                                        == Segmentation::RECEIVE;
                                if !receives_segments {
                                    BACnetServer::<T>::send_server_abort(
                                        &network_dispatch,
                                        &source_mac,
                                        source_network.as_ref(),
                                        req.invoke_id,
                                        AbortReason::SEGMENTATION_NOT_SUPPORTED,
                                    )
                                    .await;
                                    continue;
                                }

                                let mut ack_to_send: Option<SegmentAckPdu> = None;
                                let mut final_total: Option<usize> = None;

                                // The live session is consulted before the
                                // `seq == 0` open path: Clause 20.1.2.7 wraps
                                // the sequence number modulo 256, so segment
                                // 256 of a long request arrives as another
                                // `seq == 0` — treating it as a fresh initial
                                // segment would silently replace the session
                                // and reassemble only the tail (#364).
                                let saved_bytes =
                                    super::segmented_receive::saved_request_payload_bytes(
                                        &seg_receivers,
                                    );
                                if let Some(state) = seg_receivers.get_mut(&key) {
                                    // Clause 5.4.5.2 restarts SegmentTimer
                                    // for accepted, duplicate and
                                    // out-of-order segments alike, so the
                                    // refresh precedes the ordering checks.
                                    state.last_activity = Instant::now();
                                    if seq != state.expected_seq {
                                        ack_to_send =
                                            super::segmented_receive::classify_non_next_segment(
                                                state,
                                                req.invoke_id,
                                                seq,
                                            );
                                    } else {
                                        // In-order NEW segment: duplicates
                                        // and gaps returned above, so a
                                        // retransmission can never trip the
                                        // cap (Clause 5.4.5.2
                                        // DuplicateSegmentReceived requires
                                        // duplicates be discarded, not
                                        // punished).
                                        if state.accepted_segments >= MAX_REQUEST_SEGMENTS {
                                            // Clause 5.4.5.2 has no overflow
                                            // transition; SendAbort ('server'
                                            // = TRUE, reason a local matter)
                                            // is its one generic escape, and
                                            // Clause 18.10's BUFFER_OVERFLOW
                                            // — "a buffer capacity has been
                                            // exceeded" — is the fit (#364).
                                            warn!(
                                                invoke_id = req.invoke_id,
                                                accepted = state.accepted_segments,
                                                "Segmented request exceeds reassembly capacity, aborting"
                                            );
                                            seg_receivers.remove(&key);
                                            BACnetServer::<T>::send_server_abort(
                                                &network_dispatch,
                                                &source_mac,
                                                source_network.as_ref(),
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
                                        if let Err(e) = state.payload.save_new(
                                            seq,
                                            req.service_request.clone(),
                                            saved_bytes,
                                        ) {
                                            // An unsaveable segment ends the
                                            // session the same way — leaving
                                            // it dangling told the peer
                                            // nothing while this side could
                                            // never complete (#364).
                                            warn!(error = %e, "Rejecting unsaveable segment");
                                            seg_receivers.remove(&key);
                                            BACnetServer::<T>::send_server_abort(
                                                &network_dispatch,
                                                &source_mac,
                                                source_network.as_ref(),
                                                req.invoke_id,
                                                AbortReason::BUFFER_OVERFLOW,
                                            )
                                            .await;
                                            continue;
                                        }
                                        state.last_progress = Instant::now();
                                        state.accepted_segments += 1;
                                        state.expected_seq = seq.wrapping_add(1);
                                        state.last_acked_seq = seq;
                                        state.window_pos += 1;
                                        let should_ack = !req.more_follows
                                            || state.window_pos >= state.actual_window_size;
                                        if should_ack {
                                            state.window_pos = 0;
                                            state.initial_sequence_number = state.last_acked_seq;
                                            state.duplicate_count = 0;
                                            ack_to_send = Some(SegmentAckPdu {
                                                negative_ack: false,
                                                sent_by_server: true,
                                                invoke_id: req.invoke_id,
                                                sequence_number: seq,
                                                actual_window_size: state.actual_window_size,
                                            });
                                        }
                                        if !req.more_follows {
                                            // The count, not `seq + 1`: the
                                            // wire sequence number is modulo
                                            // 256 (Clause 20.1.2.7) and says
                                            // nothing about how many segments
                                            // were accepted (#364).
                                            final_total = Some(state.accepted_segments);
                                        }
                                    }
                                } else if seq == 0 {
                                    let proposed_window_size =
                                        req.proposed_window_size.unwrap_or(0);
                                    if !(1..=127).contains(&proposed_window_size) {
                                        warn!(
	                                            invoke_id = req.invoke_id,
	                                            proposed_window_size,
	                                            "Rejecting segmented request with invalid proposed window size"
	                                        );
                                        BACnetServer::<T>::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            req.invoke_id,
                                            AbortReason::WINDOW_SIZE_OUT_OF_RANGE,
                                        )
                                        .await;
                                        continue;
                                    }

                                    // New sessions only; global capacity precedes peer quota.
                                    if let Some(reason) =
                                        super::segmented_receive::segmented_request_admission_error(
                                            &seg_receivers,
                                            &key,
                                        )
                                    {
                                        BACnetServer::<T>::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            req.invoke_id,
                                            reason,
                                        )
                                        .await;
                                        continue;
                                    }

                                    let mut payload =
                                        super::segmented_receive::RequestPayload::new(req);
                                    if let Err(e) = payload.save_new(
                                        seq,
                                        req.service_request.clone(),
                                        saved_bytes,
                                    ) {
                                        // No session exists to drop on this
                                        // path; the Abort is what tells the
                                        // peer instead of leaving it to time
                                        // out (#364).
                                        warn!(error = %e, "Rejecting unsaveable segment");
                                        BACnetServer::<T>::send_server_abort(
                                            &network_dispatch,
                                            &source_mac,
                                            source_network.as_ref(),
                                            req.invoke_id,
                                            AbortReason::BUFFER_OVERFLOW,
                                        )
                                        .await;
                                        continue;
                                    }
                                    let actual_window_size = proposed_window_size;
                                    let mut state = SegmentedRequestState {
                                        payload,
                                        last_activity: Instant::now(),
                                        last_progress: Instant::now(),
                                        expected_seq: 1,
                                        initial_sequence_number: 0,
                                        duplicate_count: 0,
                                        last_acked_seq: 0,
                                        window_pos: 1,
                                        actual_window_size,
                                        accepted_segments: 1,
                                    };
                                    let should_ack =
                                        !req.more_follows || state.window_pos >= actual_window_size;
                                    if should_ack {
                                        state.window_pos = 0;
                                        state.initial_sequence_number = state.last_acked_seq;
                                        state.duplicate_count = 0;
                                        ack_to_send = Some(SegmentAckPdu {
                                            negative_ack: false,
                                            sent_by_server: true,
                                            invoke_id: req.invoke_id,
                                            sequence_number: seq,
                                            actual_window_size,
                                        });
                                    }
                                    if !req.more_follows {
                                        final_total = Some(1);
                                    }
                                    seg_receivers.insert(key.clone(), state);
                                } else {
                                    warn!(
	                                        invoke_id = req.invoke_id,
	                                        seq = seq,
	                                        "Received non-initial segment without prior segment 0, aborting"
	                                    );
                                    BACnetServer::<T>::send_server_abort(
                                        &network_dispatch,
                                        &source_mac,
                                        source_network.as_ref(),
                                        req.invoke_id,
                                        AbortReason::INVALID_APDU_IN_THIS_STATE,
                                    )
                                    .await;
                                    continue;
                                }

                                if let Some(seg_ack) = ack_to_send {
                                    let seg_ack = Apdu::SegmentAck(seg_ack);
                                    let mut ack_buf = BytesMut::new();
                                    encode_apdu(&mut ack_buf, &seg_ack)
                                        .expect("valid APDU encoding");
                                    if let Err(e) = BACnetServer::<T>::send_confirmed_response_apdu(
                                        &network_dispatch,
                                        &ack_buf,
                                        &source_mac,
                                        source_network.as_ref(),
                                    )
                                    .await
                                    {
                                        warn!(
                                            error = %e,
                                            "Failed to send SegmentAck for segmented request"
                                        );
                                    }
                                }

                                if let Some(total) = final_total {
                                    if let Some(state) = seg_receivers.remove(&key) {
                                        match state.payload.complete(total) {
                                            Ok(reassembled) => {
                                                debug!(
                                                    invoke_id = reassembled.invoke_id,
                                                    segments = total,
                                                    payload_len = reassembled.service_request.len(),
                                                    "Reassembled segmented ConfirmedRequest"
                                                );
                                                BACnetServer::<T>::dispatch(
                                                    &db_dispatch,
                                                    &network_dispatch,
                                                    &cov_dispatch,
                                                    &seg_ack_dispatch,
                                                    &seg_send_permits_dispatch,
                                                    &cov_in_flight_dispatch,
                                                    &server_tsm_dispatch,
                                                    &notification_transactions_dispatch,
                                                    &confirmed_request_tracker_dispatch,
                                                    &device_bindings_dispatch,
                                                    &comm_state_dispatch,
                                                    &dcc_timer_dispatch,
                                                    &dcc_outcomes_dispatch,
                                                    &config_dispatch,
                                                    &clock_dispatch,
                                                    &discovery_limiter_dispatch,
                                                    &requests,
                                                    &source_mac,
                                                    Apdu::ConfirmedRequest(reassembled),
                                                    received.take().unwrap_or_else(|| {
                                                        warn!("received consumed twice - using empty fallback");
                                                        bacnet_network::layer::ReceivedApdu {
                                                            apdu: bytes::Bytes::new(),
                                                            source_mac: bacnet_types::MacAddr::new(),
                                                            source_network: None,
                                                            link_layer_group: false,
                                                            is_group: false,
                                                            data_attributes: Vec::new(),
                                                            transport_meta: None,
                                                            reply_tx: None,
                                                        }
                                                    }),
                                                )
                                                .await;
                                            }
                                            Err(e) => {
                                                warn!(
                                                    error = %e,
                                                    "Failed to reassemble segmented request"
                                                );
                                            }
                                        }
                                    }
                                }

                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if !handled {
                            BACnetServer::<T>::dispatch(
                                &db_dispatch,
                                &network_dispatch,
                                &cov_dispatch,
                                &seg_ack_dispatch,
                                &seg_send_permits_dispatch,
                                &cov_in_flight_dispatch,
                                &server_tsm_dispatch,
                                &notification_transactions_dispatch,
                                &confirmed_request_tracker_dispatch,
                                &device_bindings_dispatch,
                                &comm_state_dispatch,
                                &dcc_timer_dispatch,
                                &dcc_outcomes_dispatch,
                                &config_dispatch,
                                &clock_dispatch,
                                &discovery_limiter_dispatch,
                                &requests,
                                &source_mac,
                                decoded,
                                received.take().unwrap_or_else(|| {
                                    warn!("received consumed twice — using empty fallback");
                                    bacnet_network::layer::ReceivedApdu {
                                        apdu: bytes::Bytes::new(),
                                        source_mac: bacnet_types::MacAddr::new(),
                                        source_network: None,
                                        link_layer_group: false,
                                        is_group: false,
                                        data_attributes: Vec::new(),
                                        transport_meta: None,
                                        reply_tx: None,
                                    }
                                }),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Server failed to decode received APDU");
                    }
                }
            }
    })
}
