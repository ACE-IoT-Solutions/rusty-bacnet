use super::*;
use crate::sc_tls::boxed;
use rcgen::{CertificateParams, Issuer, KeyPair};
use rustls::pki_types::PrivatePkcs8KeyDer;
use std::time::Duration;

pub(super) type ClientWs = WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

// Keep a paused Tokio clock from auto-advancing while real loopback I/O waits
// for the reactor. The wall-clock bound diagnoses a broken barrier.
pub(super) async fn poll_io<F: std::future::Future>(future: F) -> F::Output {
    tokio::pin!(future);
    let started = std::time::Instant::now();
    loop {
        if let std::task::Poll::Ready(value) = futures_util::poll!(&mut future) {
            return value;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "loopback/barrier did not complete"
        );
        tokio::task::yield_now().await;
    }
}

/// Lets the tasks that the last clock advance woke run before a test checks
/// that a deadline has not fired yet. The current-thread runtime polls the
/// test body before tasks woken in the same scheduler pass, so a check made
/// straight after `advance` cannot see an expiry. Each yield lets the queued
/// tasks run once; one is enough for a hub deadline expiry today, and the
/// rest leave room.
pub(super) async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

pub(super) async fn until(predicate: impl Fn() -> bool) {
    poll_io(async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

/// How many times a fixture's connect may run before a local port collision
/// fails it.
const CONNECT_ATTEMPTS: usize = 5;

/// The real time a fixture waits after a local port collision.
const CONNECT_PAUSE: Duration = Duration::from_millis(50);

/// Opens a fixture's outbound loopback TCP connection. The SC hub test
/// support dials through here: [`TestTls::websocket`] and the heartbeat
/// fixtures' `LiveClient::open` (#1629).
///
/// The OS picks the connection's source port. After many short-lived loopback
/// connections the port it picks can still be in use, and Windows then fails
/// the connect with `WSAEADDRINUSE`; on Linux and macOS a host out of
/// ephemeral ports fails it with `AddrNotAvailable`. Neither says anything
/// about the listener, so the connect runs again on a new socket, up to
/// [`CONNECT_ATTEMPTS`] times. Any other error fails at once.
pub(super) async fn connect_loopback(address: SocketAddr) -> tokio::net::TcpStream {
    match retry_local_port(|| tokio::net::TcpStream::connect(address)).await {
        Ok(tcp) => tcp,
        Err(err) if local_port_collision(&err) => panic!(
            "no free local port to connect to {address} in {CONNECT_ATTEMPTS} attempts: {err}"
        ),
        Err(err) => panic!("connect to {address} failed: {err}"),
    }
}

/// Whether a connect failed at its own end: the source port the OS gave it
/// was taken, or there was none to give. A connect's errors all come from the
/// OS, so the kind is enough.
fn local_port_collision(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::AddrNotAvailable
    )
}

/// Runs `dial` again while it fails with a local port collision, up to
/// [`CONNECT_ATTEMPTS`] runs, and returns the last run's result.
///
/// The pause between runs is a thread sleep on Tokio's blocking pool. Many
/// callers run on a paused clock, some inside [`poll_io`], whose yields keep
/// that clock from auto-advancing. A Tokio `sleep` would never end there, and
/// elsewhere its auto-advance would move the clock past hub deadlines a test
/// pins to the millisecond. Tokio holds a paused clock still while a
/// `spawn_blocking` task runs (see `tokio::time::pause`), so the pause is real
/// time on either clock, and it leaves the runtime's thread free.
async fn retry_local_port<T, F>(mut dial: impl FnMut() -> F) -> std::io::Result<T>
where
    F: std::future::Future<Output = std::io::Result<T>>,
{
    let mut attempt = 1;
    loop {
        match dial().await {
            Err(err) if attempt < CONNECT_ATTEMPTS && local_port_collision(&err) => {
                attempt += 1;
                tokio::task::spawn_blocking(|| std::thread::sleep(CONNECT_PAUSE))
                    .await
                    .unwrap();
            }
            result => return result,
        }
    }
}

/// Binds the address a stopped hub listened on, the tests' proof that the hub
/// closed its listener. The port is free in between, so another process can
/// take it first (#1095). `None` means one did, and the test goes again from
/// its first start, where the hub gets a fresh port; the last run keeps the
/// bind's error, so a hub that really kept its listener fails every run.
///
/// Tokio sets SO_REUSEADDR on this bind on Unix, which bounds what it shows
/// there:
/// - It binds beside connections the hub accepted on the port, established or
///   in TIME_WAIT, so it proves the listener is gone but not that every
///   connection is; the tests check connections from the peer side.
/// - On macOS only an unconnected socket at this exact address refuses it. A
///   socket another process holds on the wildcard address, or a connection
///   using the port, does not: that is the macOS shadowing that
///   `bip::bbmd_start_tests::start_on_free_port` describes, with the roles
///   swapped. Such a holder goes unnoticed and the run passes, which hides no
///   hub fault, since the hub's own listener sat on this exact address and
///   would still refuse the bind.
pub(super) async fn rebind(address: SocketAddr, attempt: usize) -> Option<TcpListener> {
    match TcpListener::bind(address).await {
        Ok(listener) => Some(listener),
        Err(err) if crate::port_ownership::lost_port(attempt, &err) => None,
        Err(err) => panic!("the stopped hub released {address}: {err}"),
    }
}

pub(super) fn request(vmac: Vmac, uuid: DeviceUuid) -> Message {
    let mut wire = crate::sc_frame::connect_test_support::valid_connect(6, vmac);
    wire[10..26].copy_from_slice(&uuid);
    Message::Binary(wire.into())
}

pub(super) struct DeadlinePeer {
    pub ws: ClientWs,
    pub sink: Arc<Mutex<WsSink>>,
    pub deadline: Arc<super::deadlines::ConnectDeadline>,
    pub task: JoinHandle<()>,
    pub active: Arc<std::sync::atomic::AtomicUsize>,
}

impl DeadlinePeer {
    pub async fn new(clients: Clients, duration: Duration) -> Self {
        let (server, ws, address, accepted) = TestTls::new().pair().await;
        let verified_leaf = super::certificate_bindings::VerifiedLeaf::from_verified_chain(
            server.get_ref().get_ref().1.peer_certificates(),
        );
        let (write, read) = server.split();
        let sink = Arc::new(Mutex::new(write));
        let deadline = Arc::new(super::deadlines::ConnectDeadline::new(accepted + duration));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let admission = super::connection::Admission::new(active.clone(), Duration::from_secs(10));
        // Direct-harness pair performs real mutual TLS with a CA-verified
        // client certificate; only the accept-loop path is bypassed.
        let runtime = Arc::new(super::admission::AdmissionRuntime::default());
        let operation = super::deadlines::serve(
            super::context::PeerConnection {
                addr: address,
                read,
                write: sink.clone(),
                verified_leaf,
            },
            super::context::HubConnectionContext {
                hub: ([0x10; 6], [0x10; 16]),
                clients,
                admission: runtime,
                graceful: super::tasks::Tasks::new().graceful_ctx(),
                timing: super::timing::HubTiming::new(super::ScHubProbePolicy::default()),
            },
            deadline.clone(),
            || {},
        );
        let task = tokio::spawn(async move {
            let _admission = admission;
            operation.await;
        });
        Self {
            ws,
            sink,
            deadline,
            task,
            active,
        }
    }

    pub async fn next(&mut self) -> Message {
        poll_io(self.ws.next()).await.unwrap().unwrap()
    }
}

impl Drop for DeadlinePeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) struct TestTls {
    pub acceptor: TlsAcceptor,
    pub hub_config: ScHubTlsConfig,
    pub client: Arc<rustls::ClientConfig>,
    pub node: crate::sc_tls::ScNodeTlsConfig,
}

impl TestTls {
    pub async fn pair(
        &self,
    ) -> (
        WebSocketStream<TlsStream>,
        ClientWs,
        SocketAddr,
        tokio::time::Instant,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        // The handshakes are boxed here and in `websocket`: tests poll on a
        // 2 MiB test thread, and unboxed, these futures put about 100 KB more
        // in each caller's frame (#953).
        let server = boxed(|| async {
            let (tcp, peer) = listener.accept().await.unwrap();
            crate::sc_tls::disable_nagle(&tcp); // as the hub's accept does
            let tls = self.acceptor.accept(tcp).await.unwrap();
            #[allow(clippy::result_large_err)]
            let ws = tokio_tungstenite::accept_hdr_async(tls, |_: &_, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                response.headers_mut().insert("Sec-WebSocket-Protocol", crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL.parse().unwrap());
                Ok(response)
            }).await.unwrap();
            (ws, peer, tokio::time::Instant::now())
        });
        let ((server, peer, accepted), client) =
            tokio::join!(server, boxed(|| self.websocket(address)));
        (server, client, peer, accepted)
    }

    pub async fn websocket(
        &self,
        address: SocketAddr,
    ) -> WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        let tcp = connect_loopback(address).await;
        let tls = boxed(|| self.connect_tls(tcp)).await;
        let request = tokio_tungstenite::tungstenite::client::ClientRequestBuilder::new(
            format!("wss://localhost:{}", address.port())
                .parse()
                .unwrap(),
        )
        .with_sub_protocol(crate::sc_frame::BACNET_SC_HUB_SUBPROTOCOL);
        boxed(|| tokio_tungstenite::client_async(request, tls))
            .await
            .unwrap()
            .0
    }

    pub async fn connect_tls(
        &self,
        tcp: tokio::net::TcpStream,
    ) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
        // Like production SC sockets (#900): with Nagle on, a test that sends
        // two messages before reading waits on the hub's delayed ACK.
        tcp.set_nodelay(true).unwrap();
        tokio_rustls::TlsConnector::from(self.client.clone())
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                tcp,
            )
            .await
            .unwrap()
    }

    pub fn new() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);
        let server_key = KeyPair::generate().unwrap();
        let server_cert = CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .signed_by(&server_key, &issuer)
            .unwrap();
        let client_key = KeyPair::generate().unwrap();
        let client_cert = CertificateParams::new(vec!["bacnet-client".into()])
            .unwrap()
            .signed_by(&client_key, &issuer)
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(ca_cert.der().clone()).unwrap();
        let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots.clone()))
            .build()
            .unwrap();
        let server = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(
                vec![server_cert.der().clone()],
                PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
            )
            .unwrap();
        let client = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![client_cert.der().clone()],
                PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
            )
            .unwrap();
        Self {
            acceptor: TlsAcceptor::from(Arc::new(server)),
            hub_config: ScHubTlsConfig::from_der(
                vec![ca_cert.der().clone()],
                vec![server_cert.der().clone()],
                PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
            )
            .unwrap(),
            client: Arc::new(client),
            node: crate::sc_tls::ScNodeTlsConfig::from_der(
                vec![ca_cert.der().clone()],
                vec![client_cert.der().clone()],
                PrivatePkcs8KeyDer::from(client_key.serialize_der()).into(),
            )
            .unwrap(),
        }
    }
}

/// Raises this process's soft descriptor limit so a test can hold `needed`
/// sockets at once. The limit is process-wide and libtest runs sibling tests
/// in the same process, so headroom is reserved for them too. A hard limit
/// that is too low fails here instead of as EMFILE in some later socket call.
#[cfg(unix)]
#[allow(unsafe_code)]
pub(super) fn reserve_descriptors(needed: libc::rlim_t) {
    const SIBLING_HEADROOM: libc::rlim_t = 1024;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit only writes the rlimit it is given.
    let read = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    assert_eq!(read, 0, "{}", std::io::Error::last_os_error());
    let wanted = needed.saturating_add(SIBLING_HEADROOM);
    if limit.rlim_cur >= wanted {
        return;
    }
    limit.rlim_cur = wanted.min(limit.rlim_max);
    // SAFETY: setrlimit only reads the rlimit it is given. Raising the soft
    // limit up to the hard limit needs no privilege.
    let raised = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    assert_eq!(raised, 0, "{}", std::io::Error::last_os_error());
    assert!(
        limit.rlim_cur >= needed,
        "test needs {needed} descriptors but the hard limit is {}",
        limit.rlim_max
    );
}

#[cfg(not(unix))]
pub(super) fn reserve_descriptors(_needed: u64) {}

mod connect_loopback_tests {
    use super::{local_port_collision, retry_local_port, CONNECT_ATTEMPTS};
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// OS error codes, each with whether it is a local port collision:
    /// WSAEADDRINUSE (the code #1629 reported), WSAEADDRNOTAVAIL,
    /// WSAECONNREFUSED and WSAECONNRESET.
    #[cfg(windows)]
    const OS_CODES: [(i32, bool); 4] =
        [(10048, true), (10049, true), (10061, false), (10054, false)];

    /// OS error codes, each with whether it is a local port collision.
    #[cfg(unix)]
    const OS_CODES: [(i32, bool); 4] = [
        (libc::EADDRINUSE, true),
        (libc::EADDRNOTAVAIL, true),
        (libc::ECONNREFUSED, false),
        (libc::ECONNRESET, false),
    ];

    /// Runs a dial through the retry, its first runs failing with `errors` in
    /// turn and the rest connecting. Returns how many runs there were and the
    /// result.
    async fn dial(errors: &[ErrorKind]) -> (usize, std::io::Result<()>) {
        let runs = AtomicUsize::new(0);
        let result = retry_local_port(|| {
            let outcome = match errors.get(runs.fetch_add(1, Ordering::Relaxed)) {
                Some(&kind) => Err(Error::from(kind)),
                None => Ok(()),
            };
            async move { outcome }
        })
        .await;
        (runs.into_inner(), result)
    }

    #[test]
    fn only_the_address_in_use_family_is_a_local_port_collision() {
        for (code, collision) in OS_CODES {
            let err = Error::from_raw_os_error(code);
            assert_eq!(local_port_collision(&err), collision, "{err:?}");
        }
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
        ] {
            assert!(!local_port_collision(&kind.into()), "{kind:?}");
        }
    }

    /// A collision connects again and a real failure fails at once, on a
    /// paused clock that holds still across the pauses.
    #[tokio::test(start_paused = true)]
    async fn a_collision_connects_again_and_a_refusal_fails_at_once() {
        // A pending timer, standing in for a hub deadline: a pause that let
        // the clock auto-advance would fire it.
        let deadline = tokio::spawn(tokio::time::sleep(std::time::Duration::from_millis(1)));
        tokio::task::yield_now().await;
        let start = tokio::time::Instant::now();

        let (runs, result) = dial(&[ErrorKind::AddrInUse, ErrorKind::AddrNotAvailable]).await;
        assert_eq!(runs, 3);
        result.unwrap();

        let (runs, result) = dial(&[ErrorKind::ConnectionRefused]).await;
        assert_eq!(runs, 1);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ConnectionRefused);

        let (runs, result) = dial(&[ErrorKind::AddrInUse, ErrorKind::ConnectionRefused]).await;
        assert_eq!(runs, 2);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ConnectionRefused);

        let (runs, result) = dial(&[ErrorKind::AddrInUse; CONNECT_ATTEMPTS + 1]).await;
        assert_eq!(runs, CONNECT_ATTEMPTS);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::AddrInUse);

        assert_eq!(
            tokio::time::Instant::now(),
            start,
            "a pause moved the clock"
        );
        assert!(!deadline.is_finished(), "a pause fired a pending timer");
    }
}
