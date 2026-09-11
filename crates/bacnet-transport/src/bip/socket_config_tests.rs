use super::*;

#[test]
fn reuse_port_defaults_off_and_builder_is_explicit() {
    let direct = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    assert!(!direct.reuse_port);

    let built = BipTransport::builder(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .reuse_port(true)
        .build();

    #[cfg(all(
        unix,
        not(any(
            target_os = "solaris",
            target_os = "illumos",
            target_os = "cygwin",
            target_os = "nuttx",
            target_os = "wasi"
        ))
    ))]
    assert!(built.unwrap().reuse_port);

    #[cfg(not(all(
        unix,
        not(any(
            target_os = "solaris",
            target_os = "illumos",
            target_os = "cygwin",
            target_os = "nuttx",
            target_os = "wasi"
        ))
    )))]
    assert!(matches!(
        built,
        Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::Unsupported
    ));
}

#[cfg(all(
    unix,
    not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "cygwin",
        target_os = "nuttx",
        target_os = "wasi"
    ))
))]
#[tokio::test]
async fn opted_in_transports_share_an_explicit_port() {
    // A wildcard interface avoids requiring privileged interface-binding
    // capabilities while still exercising both shared-port socket options.
    let mut first = BipTransport::builder(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST)
        .reuse_port(true)
        .build()
        .unwrap();
    let _first_rx = first.start().await.unwrap();
    let port = first.port;

    let mut second = BipTransport::builder(Ipv4Addr::UNSPECIFIED, port, Ipv4Addr::BROADCAST)
        .reuse_port(true)
        .build()
        .unwrap();
    let _second_rx = second.start().await.unwrap();
    assert_eq!(second.port, port);

    second.stop().await.unwrap();
    first.stop().await.unwrap();
}

#[tokio::test]
async fn default_binding_is_exclusive_and_policy_is_immutable_after_start() {
    let mut first = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _first_rx = first.start().await.unwrap();
    let port = first.port;

    let mut second = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::BROADCAST);
    assert!(matches!(
        second.start().await,
        Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::AddrInUse
    ));
    assert!(matches!(
        first.set_reuse_port(true),
        Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::InvalidInput
    ));

    first.stop().await.unwrap();
    assert!(matches!(
        first.set_reuse_port(false),
        Err(Error::Transport(error)) if error.kind() == std::io::ErrorKind::InvalidInput
    ));
}

#[cfg(unix)]
#[test]
fn configured_loopback_interface_can_be_resolved() {
    let (name, index) = resolve_ipv4_interface(Ipv4Addr::LOCALHOST)
        .expect("the loopback IPv4 address must belong to a local interface");
    assert!(!name.is_empty());
    assert_ne!(index, 0);
}
