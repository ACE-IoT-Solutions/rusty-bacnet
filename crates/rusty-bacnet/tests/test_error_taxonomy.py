"""Public exception hierarchy checks for routed and BVLC failures."""

import rusty_bacnet as rb


def test_network_reject_is_a_bacnet_error() -> None:
    assert issubclass(rb.BacnetNetworkRejectError, rb.BacnetError)


def test_bvlc_errors_have_a_specific_foreign_device_subclass() -> None:
    assert issubclass(rb.BacnetBvlcError, rb.BacnetError)
    assert issubclass(rb.BacnetForeignDeviceRegistrationError, rb.BacnetBvlcError)
