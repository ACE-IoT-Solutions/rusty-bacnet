//! Shared transport preparation for concrete and transport-erased clients.
use super::*;
use bacnet_transport::{any::AnyTransport, mstp::SerialPort};

impl BipClientBuilder {
    fn prepare(self) -> (ClientConfig, BipTransport, ClientOptions) {
        let mut transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        transport.set_share_port_by_address(self.config.share_port_by_address);
        transport.set_mode(self.mode);
        (self.config, transport, self.options)
    }

    /// Build and start the client, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetClient<BipTransport>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Build and start a configured B/IP client with a transport-erased type.
    ///
    /// Uses the same settings and validation as [`Self::build`]. Choose
    /// [`NoSerial`](bacnet_transport::mstp::NoSerial) when MS/TP is not needed;
    /// no serial-port value is constructed or required.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::{any::AnyTransport, mstp::NoSerial};
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let mut client: BACnetClient<AnyTransport<NoSerial>> = BACnetClient::bip_builder()
    ///     .port(0)
    ///     .apdu_retries(2)
    ///     .build_any::<NoSerial>().await?;
    /// client.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn build_any<S: SerialPort>(self) -> Result<BACnetClient<AnyTransport<S>>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }
}

#[cfg(feature = "ipv6")]
impl Bip6ClientBuilder {
    fn prepare(self) -> (ClientConfig, Bip6Transport, ClientOptions) {
        let transport = Bip6Transport::new(self.interface, self.config.port, self.device_instance);
        (self.config, transport, self.options)
    }

    /// Build and start the client, constructing a Bip6Transport from the config.
    pub async fn build(self) -> Result<BACnetClient<Bip6Transport>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Build and start a configured B/IPv6 client with a transport-erased type.
    ///
    /// Uses the same interface, device identity and client options as
    /// [`Self::build`]. No serial-port value is required.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::{any::AnyTransport, mstp::NoSerial};
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let mut client: BACnetClient<AnyTransport<NoSerial>> = BACnetClient::bip6_builder()
    ///     .device_instance(123)
    ///     .build_any::<NoSerial>().await?;
    /// client.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn build_any<S: SerialPort>(self) -> Result<BACnetClient<AnyTransport<S>>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }
}

#[cfg(feature = "sc-tls")]
impl ScClientBuilder {
    pub(super) fn validate(&self) -> Result<(), Error> {
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        self.validate_identity()?;
        self.options.validate()?;
        self.config.validate_request_settings()?;
        self.config.validate_window()
    }

    async fn prepare(
        self,
    ) -> Result<
        (
            ClientConfig,
            bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>,
            ClientOptions,
        ),
        Error,
    > {
        self.validate()?;
        let tls_config =
            self.tls_config.as_ref().cloned().ok_or_else(|| {
                Error::Encoding("SC client builder: tls_config is required".into())
            })?;
        let ws = Box::pin(bacnet_transport::sc_tls::TlsWebSocket::connect(
            &self.hub_url,
            tls_config.clone(),
        ))
        .await?;
        let mut transport = self.sc_transport(ws);
        if let Some(rc) = self.reconnect {
            let hub_url = self.hub_url;
            transport = transport
                .with_connector(move || {
                    let hub_url = hub_url.clone();
                    let tls_config = tls_config.clone();
                    async move {
                        bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config).await
                    }
                })
                .with_reconnect(rc);
        }
        Ok((self.config, transport, self.options))
    }

    /// Connect to the hub and start the client.
    ///
    /// Reconnect configuration is validated before TLS lookup or dialing, and
    /// again when the transport starts. An error still consumes this builder
    /// and drops its inputs; this does not promise generic endpoint rollback.
    ///
    /// The client's [`transport()`](BACnetClient::transport) lends the hub
    /// connection-state watch, accepted Hub certificate observation and NPDU drop counts:
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::ScConnectionState;
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    ///
    /// async fn watch_hub(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
    ///     let client = BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([0x02, 0, 0, 0, 0, 0x01])
    ///         .device_uuid([0x42; 16])
    ///         .build()
    ///         .await?;
    ///     // An owned receiver: it can live in its own task.
    ///     let mut state = client.transport().connection_state_changes();
    ///     tokio::spawn(async move {
    ///         while state.changed().await.is_ok() {
    ///             let _connected = *state.borrow_and_update() == ScConnectionState::Connected;
    ///         }
    ///     });
    ///     // Owned historical TLS data, present only for an accepted active Hub.
    ///     let _peer = client.transport().hub_peer_info().await;
    ///     // A snapshot: poll it as often as needed.
    ///     let _full_queue_drops = client.transport().npdu_drop_counts().full_drops;
    ///     Ok(())
    /// }
    /// ```
    pub async fn build(
        self,
    ) -> Result<
        BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        Box::pin(async move {
            let (config, transport, options) = Box::pin(self.prepare()).await?;
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Connect to the hub and start a transport-erased BACnet/SC client.
    ///
    /// Shares [`Self::build`]'s validation, TLS policy, identity, heartbeat and
    /// optional reconnect configuration. Identity, reconnect and client options
    /// are checked before dialing; heartbeat timing is checked when the transport
    /// starts. No serial-port value is required.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::{any::AnyTransport, mstp::NoSerial, sc::ScReconnectConfig};
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    /// async fn connect(tls: ScNodeTlsConfig) -> Result<BACnetClient<AnyTransport<NoSerial>>,
    ///     bacnet_types::error::Error> {
    ///     BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([2, 0, 0, 0, 0, 1])
    ///         .device_uuid([0x42; 16])
    ///         .reconnect(ScReconnectConfig::default())
    ///         .build_any::<NoSerial>().await
    /// }
    /// ```
    pub async fn build_any<S: SerialPort>(self) -> Result<BACnetClient<AnyTransport<S>>, Error> {
        Box::pin(async move {
            let (config, transport, options) = Box::pin(self.prepare()).await?;
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }
}
