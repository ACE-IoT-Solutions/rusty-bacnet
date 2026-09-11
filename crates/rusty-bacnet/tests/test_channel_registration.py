"""Installed-wheel coverage for restored Channel registration."""

import unittest

import rusty_bacnet as bacnet


class ChannelRegistrationTests(unittest.TestCase):
    def test_add_channel_retains_a_pre_start_registration(self) -> None:
        server = bacnet.BACnetServer(320001)
        before = server._pending_registration_count()
        server.add_channel(1, "Floor channel", 7)
        self.assertEqual(server._pending_registration_count(), before + 1)

    def test_reserved_channel_number_is_rejected_without_registration(self) -> None:
        server = bacnet.BACnetServer(320002)
        before = server._pending_registration_count()
        with self.assertRaises(bacnet.BacnetProtocolError):
            server.add_channel(1, "Invalid channel", 0)
        self.assertEqual(server._pending_registration_count(), before)


if __name__ == "__main__":
    unittest.main()
