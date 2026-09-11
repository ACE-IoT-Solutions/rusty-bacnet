use super::*;

pub(super) fn validate_persisted_path(
    attachment_id: crate::AttachmentId,
    transport: &crate::TransportConfig,
    path: &crate::DevicePath,
) -> Result<(), RuntimeError> {
    let expected_mac_len = match transport {
        crate::TransportConfig::Bip(_) | crate::TransportConfig::Sc(_) => 6,
        crate::TransportConfig::Mstp(_) => 1,
    };
    let (mac, routed) = match path {
        crate::DevicePath::Direct { mac } => (mac, None),
        crate::DevicePath::Routed {
            ingress_mac,
            dnet,
            dadr,
        } => (ingress_mac, Some((*dnet, dadr))),
    };
    if mac.len() != expected_mac_len {
        return Err(RuntimeError::invalid_attachment_config(
            attachment_id,
            format!("path MAC must be exactly {expected_mac_len} bytes for this transport"),
        ));
    }
    if let Some((dnet, dadr)) = routed {
        if dnet == 0 || dnet == u16::MAX {
            return Err(RuntimeError::invalid_attachment_config(
                attachment_id,
                "routed DNET must be in 1..=65534",
            ));
        }
        if dadr.is_empty() || dadr.len() > u8::MAX as usize {
            return Err(RuntimeError::invalid_attachment_config(
                attachment_id,
                "routed DADR must contain 1..=255 bytes",
            ));
        }
    }
    Ok(())
}

pub(super) fn decode_object_list(
    raw: &[u8],
    max_objects: usize,
) -> Result<Vec<(u16, u32)>, RuntimeError> {
    let mut offset = 0;
    let mut objects = Vec::new();
    while offset < raw.len() {
        let (value, next) = decode_application_value(raw, offset)
            .map_err(|error| RuntimeError::decode(format!("invalid object-list value: {error}")))?;
        if next <= offset {
            return Err(RuntimeError::decode("object-list decoder made no progress"));
        }
        let PropertyValue::ObjectIdentifier(identifier) = value else {
            return Err(RuntimeError::decode(
                "object-list contained a non-object-identifier value",
            ));
        };
        let object_type = u16::try_from(identifier.object_type().to_raw())
            .map_err(|_| RuntimeError::decode("object type exceeds runtime range"))?;
        objects.push((object_type, identifier.instance_number()));
        if objects.len() > max_objects {
            return Err(RuntimeError::invalid_config(format!(
                "device object-list exceeds configured maximum {max_objects}"
            )));
        }
        offset = next;
    }
    Ok(objects)
}

pub(super) async fn enumerate_indexed(
    transport: &crate::RuntimeTransport,
    device: crate::DeviceKey,
    path: &crate::DevicePath,
    whole: &crate::PropertyRead,
    max_objects: usize,
) -> Result<Vec<(u16, u32)>, RuntimeError> {
    let mut count_read = whole.clone();
    count_read.array_index = Some(0);
    let raw = transport
        .read_one(device.attachment_id, path, &count_read)
        .await?;
    let (value, consumed) = decode_application_value(&raw, 0)
        .map_err(|error| RuntimeError::decode(format!("invalid object-list count: {error}")))?;
    if consumed != raw.len() {
        return Err(RuntimeError::decode(
            "object-list count contained trailing bytes",
        ));
    }
    let PropertyValue::Unsigned(count) = value else {
        return Err(RuntimeError::decode(
            "object-list index zero was not Unsigned",
        ));
    };
    let count = usize::try_from(count)
        .map_err(|_| RuntimeError::invalid_config("object-list count exceeds platform size"))?;
    if count > max_objects {
        return Err(RuntimeError::invalid_config(format!(
            "device object-list count {count} exceeds configured maximum {max_objects}"
        )));
    }
    let mut objects = Vec::with_capacity(count);
    for index in 1..=count {
        let mut indexed = whole.clone();
        indexed.input_index = index;
        indexed.array_index = Some(u32::try_from(index).map_err(|_| {
            RuntimeError::invalid_config("object-list index exceeds BACnet array range")
        })?);
        let raw = transport
            .read_one(device.attachment_id, path, &indexed)
            .await?;
        let mut decoded = decode_object_list(&raw, 1)?;
        if decoded.len() != 1 {
            return Err(RuntimeError::decode(
                "indexed object-list read returned zero elements",
            ));
        }
        objects.push(decoded.remove(0));
    }
    Ok(objects)
}

pub(super) async fn work_loop(
    inner: std::sync::Weak<RuntimeInner>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        let Some(runtime_inner) = inner.upgrade() else {
            return;
        };
        if *stop.borrow() {
            cancel_queued(&runtime_inner).await;
            return;
        }
        if runtime_inner.work_queue.lock().await.is_empty() {
            tokio::select! {
                _ = runtime_inner.work_notify.notified() => {}
                _ = stop.changed() => {
                    cancel_queued(&runtime_inner).await;
                    return;
                }
            }
            continue;
        }
        let permit = tokio::select! {
            permit = Arc::clone(&runtime_inner.work_slots).acquire_owned() => {
                match permit {
                    Ok(permit) => permit,
                    Err(_) => return,
                }
            }
            _ = stop.changed() => {
                cancel_queued(&runtime_inner).await;
                return;
            }
        };
        let Some(dispatch) = runtime_inner
            .work_queue
            .lock()
            .await
            .next(std::time::Instant::now())
        else {
            drop(permit);
            continue;
        };
        let scheduled = match dispatch {
            Dispatch::Expired(scheduled) => {
                scheduled.payload.fail(RuntimeError::deadline_exceeded(
                    "scheduled operation expired before dispatch",
                ));
                drop(permit);
                continue;
            }
            Dispatch::Ready(scheduled) => scheduled,
        };
        let id = scheduled.id;
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        runtime_inner.inflight.lock().await.insert(id, cancel_tx);
        let task_inner = Arc::clone(&runtime_inner);
        let mut command = scheduled.payload;
        let _ = runtime_inner
            .supervisor
            .spawn(move |mut global_stop| async move {
                let _permit = permit;
                let runtime = BacnetRuntime {
                    inner: Arc::clone(&task_inner),
                };
                let failure = tokio::select! {
                    biased;
                    _ = global_stop.changed() => Some(RuntimeError::cancelled("runtime stopped during operation")),
                    _ = cancel_rx.changed() => Some(RuntimeError::cancelled("operation cancelled in flight")),
                    _ = command.execute(&runtime) => None,
                };
                if let Some(error) = failure {
                    command.fail(error);
                }
                task_inner.inflight.lock().await.remove(&id);
            })
            .await;
    }
}

pub(super) async fn cancel_queued(inner: &RuntimeInner) {
    let mut queue = inner.work_queue.lock().await;
    while let Some(dispatch) = queue.next(std::time::Instant::now()) {
        let scheduled = match dispatch {
            Dispatch::Ready(scheduled) | Dispatch::Expired(scheduled) => scheduled,
        };
        scheduled
            .payload
            .fail(RuntimeError::cancelled("runtime stopped before dispatch"));
    }
}

pub(super) fn validate_unique_indices(
    indices: impl IntoIterator<Item = usize>,
    label: &str,
) -> Result<(), RuntimeError> {
    let mut seen = std::collections::BTreeSet::new();
    if indices.into_iter().all(|index| seen.insert(index)) {
        Ok(())
    } else {
        Err(RuntimeError::invalid_config(format!(
            "{label} input_index values must be unique"
        )))
    }
}

pub(super) fn read_failure(
    input_index: usize,
    device: crate::DeviceKey,
    error: &RuntimeError,
) -> ReadOutcome {
    ReadOutcome {
        input_index,
        device,
        path: None,
        raw_value: None,
        source: None,
        error: Some(crate::batch::outcome_error(error)),
    }
}
