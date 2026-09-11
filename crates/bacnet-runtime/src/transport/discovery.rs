use super::*;

impl RuntimeTransport {
    pub(crate) async fn stop(
        &mut self,
        attachment_id: crate::AttachmentId,
    ) -> Result<(), RuntimeError> {
        match self {
            Self::Bip(client) => client
                .stop()
                .await
                .map_err(|error| RuntimeError::attachment_start(attachment_id, error)),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client
                .stop()
                .await
                .map_err(|error| RuntimeError::attachment_start(attachment_id, error)),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client
                .stop()
                .await
                .map_err(|error| RuntimeError::attachment_start(attachment_id, error)),
        }
    }

    pub(crate) async fn discover(
        &self,
        attachment_id: crate::AttachmentId,
        request: &crate::DiscoveryRequest,
    ) -> AttachmentDiscovery {
        match self {
            Self::Bip(client) => {
                if request.clear_existing {
                    client.clear_devices().await;
                }
                let devices = async {
                    client.who_is(request.low_limit, request.high_limit).await?;
                    tokio::time::sleep(request.observation_window).await;
                    Ok::<_, bacnet_types::error::Error>(client.discovered_devices().await)
                };
                let devices = devices.await;
                let mut errors = Vec::new();
                let devices = devices.unwrap_or_else(|error| {
                    errors.push(RuntimeError::operation(attachment_id, error));
                    Vec::new()
                });
                AttachmentDiscovery {
                    devices,
                    routers: Vec::new(),
                    errors,
                }
            }
            #[cfg(feature = "sc")]
            Self::Sc(client) => {
                if request.clear_existing {
                    client.clear_devices().await;
                }
                let devices = async {
                    client.who_is(request.low_limit, request.high_limit).await?;
                    tokio::time::sleep(request.observation_window).await;
                    Ok::<_, bacnet_types::error::Error>(client.discovered_devices().await)
                };
                let devices = devices.await;
                let mut errors = Vec::new();
                let devices = devices.unwrap_or_else(|error| {
                    errors.push(RuntimeError::operation(attachment_id, error));
                    Vec::new()
                });
                AttachmentDiscovery {
                    devices,
                    routers: Vec::new(),
                    errors,
                }
            }
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => {
                if request.clear_existing {
                    client.clear_devices().await;
                }
                let devices = async {
                    client.who_is(request.low_limit, request.high_limit).await?;
                    tokio::time::sleep(request.observation_window).await;
                    Ok::<_, bacnet_types::error::Error>(client.discovered_devices().await)
                };
                let devices = devices.await;
                let mut errors = Vec::new();
                let devices = devices.unwrap_or_else(|error| {
                    errors.push(RuntimeError::operation(attachment_id, error));
                    Vec::new()
                });
                AttachmentDiscovery {
                    devices,
                    routers: Vec::new(),
                    errors,
                }
            }
        }
    }

    pub(crate) async fn topology(
        &self,
        attachment_id: crate::AttachmentId,
        seeds: Vec<Vec<u8>>,
        include_fdt: bool,
        max_bbmds: usize,
    ) -> AttachmentTopologyResult {
        match self {
            Self::Bip(client) => {
                let mut queue: VecDeque<Vec<u8>> = seeds.into();
                let mut visited = BTreeSet::new();
                let mut bbmds = Vec::new();
                let mut errors = Vec::new();
                let mut truncated = false;
                while let Some(target) = queue.pop_front() {
                    if visited.contains(&target) {
                        continue;
                    }
                    if visited.len() >= max_bbmds {
                        truncated = true;
                        break;
                    }
                    visited.insert(target.clone());
                    let bdt = match client.read_bdt(&target).await {
                        Ok(entries) => entries
                            .into_iter()
                            .map(|entry| BdtRecord {
                                ip: entry.ip,
                                port: entry.port,
                                broadcast_mask: entry.broadcast_mask,
                            })
                            .collect::<Vec<_>>(),
                        Err(error) => {
                            errors.push(RuntimeError::operation(attachment_id, error));
                            continue;
                        }
                    };
                    for peer in &bdt {
                        let mac = peer.mac();
                        if !visited.contains(&mac) {
                            queue.push_back(mac);
                        }
                    }
                    let fdt = if include_fdt {
                        match client.read_fdt(&target).await {
                            Ok(entries) => entries
                                .into_iter()
                                .map(|entry| FdtRecord {
                                    ip: entry.ip,
                                    port: entry.port,
                                    ttl: entry.ttl,
                                    seconds_remaining: entry.seconds_remaining,
                                })
                                .collect(),
                            Err(error) => {
                                errors.push(RuntimeError::operation(attachment_id, error));
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };
                    bbmds.push(BbmdSnapshot {
                        mac: target,
                        bdt,
                        fdt,
                    });
                }
                AttachmentTopologyResult {
                    local_mac: client.local_mac().to_vec(),
                    bbmds,
                    routers: Vec::new(),
                    truncated,
                    errors,
                }
            }
            #[cfg(feature = "sc")]
            Self::Sc(client) => {
                let mut errors = Vec::new();
                if !seeds.is_empty() || include_fdt {
                    errors.push(RuntimeError::invalid_attachment_config(
                        attachment_id,
                        "BBMD/FDT topology operations are only available on B/IP attachments",
                    ));
                }
                AttachmentTopologyResult {
                    local_mac: client.local_mac().to_vec(),
                    bbmds: Vec::new(),
                    routers: Vec::new(),
                    truncated: false,
                    errors,
                }
            }
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => {
                let mut errors = Vec::new();
                if !seeds.is_empty() || include_fdt {
                    errors.push(RuntimeError::invalid_attachment_config(
                        attachment_id,
                        "BBMD/FDT topology operations are only available on B/IP attachments",
                    ));
                }
                AttachmentTopologyResult {
                    local_mac: client.local_mac().to_vec(),
                    bbmds: Vec::new(),
                    routers: Vec::new(),
                    truncated: false,
                    errors,
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn seed_device(&self, instance: u32, mac: &[u8]) -> Result<(), RuntimeError> {
        match self {
            Self::Bip(client) => client
                .add_device(instance, mac)
                .await
                .map_err(|error| RuntimeError::operation(crate::AttachmentId::from(0), error)),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client
                .add_device(instance, mac)
                .await
                .map_err(|error| RuntimeError::operation(crate::AttachmentId::from(0), error)),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client
                .add_device(instance, mac)
                .await
                .map_err(|error| RuntimeError::operation(crate::AttachmentId::from(0), error)),
        }
    }

    /// Transport-native local MAC bytes.
    pub fn local_mac(&self) -> &[u8] {
        match self {
            Self::Bip(client) => client.local_mac(),
            #[cfg(feature = "mstp")]
            Self::Mstp(client) => client.local_mac(),
            #[cfg(feature = "sc")]
            Self::Sc(client) => client.local_mac(),
        }
    }

    pub(crate) fn must_stop_before_replacement(
        current: &TransportConfig,
        desired: &TransportConfig,
    ) -> bool {
        match (current, desired) {
            (TransportConfig::Bip(current), TransportConfig::Bip(desired)) => {
                current.interface == desired.interface && current.port == desired.port
            }
            (TransportConfig::Mstp(current), TransportConfig::Mstp(desired)) => {
                current.device == desired.device
            }
            _ => false,
        }
    }
}
