//! The `[mcp]` table as a server policy, and the first-use authorization.

use super::*;

/// A scripted person at the terminal.
struct Typed {
    interactive: bool,
    answer: Option<&'static str>,
    asked: Vec<String>,
}

impl Confirm for Typed {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn ask(&mut self, prompt: &str) -> Option<String> {
        self.asked.push(prompt.into());
        self.answer.map(|a| format!("{a}\n"))
    }
}

fn typed(interactive: bool, answer: Option<&'static str>) -> Typed {
    Typed {
        interactive,
        answer,
        asked: Vec::new(),
    }
}

/// Whether the stored profile's consent stands.
fn authorized(store: &Store) -> bool {
    store
        .load("site")
        .unwrap()
        .mcp
        .is_some_and(|mcp| mcp.authorized())
}

fn store_with(section: Option<McpSection>) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let profile = Profile {
        mcp: section.map(Box::new),
        ..Profile::default()
    };
    store.save("site", &profile, true).unwrap();
    (dir, store)
}

#[test]
fn the_table_becomes_the_policy() {
    let section = McpSection {
        networks: Some(vec![0, 2001]),
        devices: Some(vec!["1000-1999".into(), "5".into()]),
        slow_networks: vec![2001],
        max_rate: Some(10),
        call_budget_s: Some(45),
        ..McpSection::default()
    };
    let mapped = policy(&section, false).unwrap();
    assert_eq!(mapped.networks, Some([0, 2001].into()));
    assert_eq!(mapped.devices, Some(vec![(1000, 1999), (5, 5)]));
    assert_eq!(mapped.rates.global, 10);
    assert_eq!(mapped.rates.principal, Rates::default().principal);
    assert_eq!(mapped.caps.budget, Duration::from_secs(45));
    assert!(!mapped.passive && mapped.slow_networks.contains(&2001));
    assert_eq!(
        policy(&McpSection::default(), true).unwrap(),
        Policy {
            passive: true,
            ..Policy::default()
        }
    );
    let bad = McpSection {
        principal_rate: Some(0),
        ..McpSection::default()
    };
    assert!(policy(&bad, false).unwrap_err().starts_with("[mcp]"));
}

/// The profile name typed at a terminal records consent and the table it
/// covered; anything else, or no terminal, records nothing.
#[test]
fn authorization_needs_a_person_to_type_the_name() {
    let section = McpSection {
        networks: Some(vec![0]),
        ..McpSection::default()
    };
    let (_dir, store) = store_with(Some(section.clone()));
    for mut person in [
        typed(false, Some("site")),
        typed(true, Some("other")),
        typed(true, None),
    ] {
        assert!(authorize(&store, "site", &mut person).is_err());
        assert!(!authorized(&store));
    }
    let mut person = typed(true, Some("site"));
    authorize(&store, "site", &mut person).unwrap();
    assert!(
        person.asked[0].contains("networks: [0]"),
        "{}",
        person.asked[0]
    );
    let stored = *store.load("site").unwrap().mcp.unwrap();
    assert!(stored.authorized_at_unix.is_some());
    assert_eq!(stored.authorized.as_deref(), Some(&section));
}

/// Widening the table after authorizing clears the authorization when the
/// profile is next loaded; narrowing it doesn't.
#[test]
fn widening_reach_needs_authorizing_again() {
    let (_dir, store) = store_with(Some(McpSection {
        networks: Some(vec![0]),
        ..McpSection::default()
    }));
    authorize(&store, "site", &mut typed(true, Some("site"))).unwrap();
    let mut profile = store.load("site").unwrap();
    profile.mcp.as_mut().unwrap().devices = Some(vec!["1-10".into()]);
    store.save("site", &profile, false).unwrap();
    assert!(authorized(&store));
    profile.mcp.as_mut().unwrap().networks = None;
    store.save("site", &profile, false).unwrap();
    assert!(!authorized(&store));
    assert!(unauthorized("site").contains("bacnet mcp authorize --profile site"));
}
