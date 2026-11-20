"""ACE mixed-router endpoint identity includes the resolved address."""

import unittest

import rusty_bacnet as rb
from test_bip_interface_name import loopback_name


class MixedRouterInterfaceNameTests(unittest.TestCase):
    def test_named_and_literal_router_ports_have_identical_address_identity(self):
        named = rb.RouterBipPort(1, interface=loopback_name(), port=47808)
        literal = rb.RouterBipPort(2, interface="127.0.0.1", port=47808)
        self.assertEqual(named.interface, literal.interface)
        with self.assertRaisesRegex(ValueError, "duplicate B/IP router endpoint"):
            rb.AceMixedRouter.from_ports([named, literal])
