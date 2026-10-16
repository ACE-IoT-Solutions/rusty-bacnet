use super::*;

#[test]
fn exact_time_boundaries_and_extreme_seconds_do_not_overflow() {
    for (now, expected) in [
        (0, ScExpiry::Warning),
        (1, ScExpiry::Alarm),
        (604_801, ScExpiry::Alarm),
    ] {
        assert_eq!(
            assess_time(0, 604_801, now),
            (ScValidity::WithinInterval, expected)
        );
    }
    assert_eq!(
        assess_time(0, 2_592_001, 0),
        (ScValidity::WithinInterval, ScExpiry::None)
    );
    assert_eq!(
        assess_time(0, 2_592_001, 1),
        (ScValidity::WithinInterval, ScExpiry::Warning)
    );
    assert_eq!(
        assess_time(2, 1, 1),
        (ScValidity::InvalidInterval, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(0, 1, -1),
        (ScValidity::NotYetValid, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(0, 1, 2),
        (ScValidity::Expired, ScExpiry::NotAssessed)
    );
    assert_eq!(
        assess_time(i64::MIN, i64::MAX, i64::MIN),
        (ScValidity::WithinInterval, ScExpiry::None)
    );
    assert_eq!(
        assess_time(i64::MIN, i64::MAX, i64::MAX),
        (ScValidity::WithinInterval, ScExpiry::Alarm)
    );
}

#[test]
fn unknown_consistency_is_never_promoted_to_match() {
    #[derive(Debug)]
    struct OpaqueKey;
    impl rustls::sign::SigningKey for OpaqueKey {
        fn choose_scheme(
            &self,
            _: &[rustls::SignatureScheme],
        ) -> Option<Box<dyn rustls::sign::Signer>> {
            None
        }
        fn algorithm(&self) -> rustls::SignatureAlgorithm {
            rustls::SignatureAlgorithm::ED25519
        }
    }
    // Exercise Rustls's real unavailable-SPKI branch, not constructor success.
    let key = CertifiedKey::new(
        vec![CertificateDer::from(vec![0])],
        std::sync::Arc::new(OpaqueKey),
    );
    assert_eq!(classify_match(key.keys_match()), ScKeyConsistency::Unknown);
    assert_eq!(
        classify_match(Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::BadEncoding
        ))),
        ScKeyConsistency::Unknown
    );
}
