//! A profile's `[mcp]` table: what `bacnet mcp serve` may reach on the
//! site, and how fast (#2066). Every key is optional; an absent table lets
//! the server reach every network and device at the default rates, with no
//! broad discovery.
//!
//! ```toml
//! [mcp]
//! networks = [0, 2001]          # 0 is the local network
//! devices = ["1000-1999", "5"]
//! slow_networks = [2001]        # MS/TP behind a router: 2 requests a second
//! broad_discovery = false
//! ```
//!
//! The TUI round-trips the table without using it. Its first-use
//! authorization, separate from the TUI's, is recorded in it by `bacnet mcp
//! authorize` (`authorized_at_unix`, and the reach it covered as
//! `[mcp.authorized]`) and cleared when the table widens the server's
//! reach.

use serde::{Deserialize, Serialize};

/// The `[mcp]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct McpSection {
    /// Networks the server may reach (0 for the local one); absent for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) networks: Option<Vec<u16>>,
    /// Device instances it may reach, as `low-high` or one instance;
    /// absent for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) devices: Option<Vec<String>>,
    /// Networks held to `slow_rate`, such as a 9,600-baud MS/TP trunk.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) slow_networks: Vec<u16>,
    /// Networks held to `fast_rate`, such as a fast MS/TP trunk.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) fast_networks: Vec<u16>,
    /// Whether a global or unbounded Who-Is may go, one a minute.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) broad_discovery: bool,
    /// Requests a second across everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_rate: Option<u32>,
    /// Requests a second to any one network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_rate: Option<u32>,
    /// Requests a second to a slow network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slow_rate: Option<u32>,
    /// Requests a second to a fast MS/TP network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fast_rate: Option<u32>,
    /// Requests a second for the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) principal_rate: Option<u32>,
    /// Requests the agent may send at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) principal_burst: Option<u32>,
    /// Property values one read returns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_values: Option<usize>,
    /// Devices on a discovery page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page_devices: Option<usize>,
    /// Objects on an object list page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page_objects: Option<usize>,
    /// Characters kept of a device-supplied string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_text: Option<usize>,
    /// Seconds a call may wait on the limits and the network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) call_budget_s: Option<u64>,
    /// When `bacnet mcp authorize` recorded consent to serve MCP on this
    /// profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorized_at_unix: Option<u64>,
    /// The reach that consent covered (this table without its own
    /// consent); a table that widens it needs authorizing again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) authorized: Option<Box<McpSection>>,
}

/// One `devices` entry: `low-high` or one instance.
pub(crate) fn device_range(text: &str) -> Result<(u32, u32), String> {
    let bad = || format!("[mcp] devices: '{text}' is not low-high or a device instance");
    let (low, high) = match text.split_once('-') {
        Some((low, high)) => (
            low.trim().parse().map_err(|_| bad())?,
            high.trim().parse().map_err(|_| bad())?,
        ),
        None => {
            let one = text.trim().parse().map_err(|_| bad())?;
            (one, one)
        }
    };
    if low > high || high > 4_194_303 {
        return Err(bad());
    }
    Ok((low, high))
}

impl McpSection {
    /// Check what the TUI can check without the MCP server: the device
    /// ranges parse. The server checks the rest when it starts.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for text in self.devices.iter().flatten() {
            device_range(text)?;
        }
        Ok(())
    }

    /// The device ranges, parsed.
    fn ranges(&self) -> Option<Vec<(u32, u32)>> {
        self.devices.as_ref().map(|list| {
            list.iter()
                .filter_map(|text| device_range(text).ok())
                .collect()
        })
    }

    /// Whether consent covers this table's reach now.
    pub(crate) fn authorized(&self) -> bool {
        self.authorized_at_unix.is_some()
    }

    /// Record consent, given at `at`, to this table's reach.
    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn authorize(&mut self, at: u64) {
        let mut covered = self.clone();
        covered.authorized_at_unix = None;
        covered.authorized = None;
        self.authorized_at_unix = Some(at);
        self.authorized = Some(Box::new(covered));
    }

    /// Drop a consent that doesn't cover this table's reach.
    pub(crate) fn revoke_if_raised(&mut self) {
        let covered = self.authorized.as_deref().cloned().unwrap_or_default();
        if self.authorized() && self.raises(&covered) {
            self.authorized_at_unix = None;
            self.authorized = None;
        }
    }

    /// True when `self` lets the server reach something `from` did not: a
    /// network or device outside `from`'s lists, or broad discovery.
    pub(crate) fn raises(&self, from: &Self) -> bool {
        let networks = match (&self.networks, &from.networks) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(now), Some(was)) => now.iter().any(|n| !was.contains(n)),
        };
        let devices = match (self.ranges(), from.ranges()) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(now), Some(was)) => now
                .iter()
                .any(|&(low, high)| !was.iter().any(|&(l, h)| l <= low && high <= h)),
        };
        networks || devices || (self.broad_discovery && !from.broad_discovery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_parses_and_round_trips() {
        let text = "networks = [0, 2001]\ndevices = [\"1000-1999\", \"5\"]\n\
                    slow_networks = [2001]\nmax_rate = 10\n";
        let section: McpSection = toml::from_str(text).unwrap();
        section.validate().unwrap();
        assert_eq!(section.networks, Some(vec![0, 2001]));
        assert_eq!(section.max_rate, Some(10));
        let again: McpSection = toml::from_str(&toml::to_string(&section).unwrap()).unwrap();
        assert_eq!(again, section);
        assert!(toml::from_str::<McpSection>("writes = true").is_err());
        let bad = McpSection {
            devices: Some(vec!["9-1".into()]),
            ..McpSection::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn widening_reach_raises_and_narrowing_does_not() {
        let narrow = McpSection {
            networks: Some(vec![0]),
            devices: Some(vec!["100-199".into()]),
            ..McpSection::default()
        };
        let all = McpSection::default();
        assert!(
            !narrow.raises(&all),
            "narrowing never needs a new authorization"
        );
        assert!(all.raises(&narrow));
        let more_networks = McpSection {
            networks: Some(vec![0, 7]),
            ..narrow.clone()
        };
        assert!(more_networks.raises(&narrow));
        let more_devices = McpSection {
            devices: Some(vec!["100-250".into()]),
            ..narrow.clone()
        };
        assert!(more_devices.raises(&narrow));
        let broad = McpSection {
            broad_discovery: true,
            ..narrow.clone()
        };
        assert!(broad.raises(&narrow));
        let slower = McpSection {
            max_rate: Some(1),
            ..narrow.clone()
        };
        assert!(!slower.raises(&narrow));
    }
}
