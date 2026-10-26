//! The website's capability tables, `website/src/data/support.json` (#1496).
//!
//! Everything in the file comes from code or from the files the release
//! builds from; nothing here is a hand-kept list of capabilities:
//!
//! - object types: one object of each from `bacnet_objects::catalogue`, put
//!   in a database and read back through the PICS generator, which says
//!   whether CreateObject and DeleteObject reach the type;
//! - services: the full server's `EXECUTED_SERVICES` and the PICS initiator
//!   list, the client's `SENT_SERVICES`, the shared endpoint's responder lists,
//!   and the Python client's methods in the type stub, matched to services by
//!   name ([`python_services`]);
//! - data links: the Cargo features of `bacnet-transport`, resolved through the
//!   manifests for the Python wheel (`rusty-bacnet`) and for each release
//!   target of the CLI (`TARGETS` in `scripts/release/third_party_notices.py`,
//!   which its own test ties to the release build scripts);
//! - the release the site describes, from `website/src/lib/site.mjs`.
//!
//! The file tracks `dev`: it is regenerated whenever one of those sources
//! changes, and the page says so. The freshness test in
//! `tests/support_tables.rs` fails when the committed file differs from a
//! fresh run; `cargo run -p bacnet-integration-tests --example support-tables`
//! rewrites it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bacnet_objects::database::ObjectDatabase;
use bacnet_server::pics::{DataLinkSupport, PicsConfig, PicsGenerator, service_display_name};
use bacnet_server::server::ServerConfig;
use bacnet_types::enums::{ConfirmedServiceChoice, ServiceSupported, UnconfirmedServiceChoice};
use serde_json::{Value, json};

/// Where the generated file goes, relative to the repository root.
pub const OUTPUT: &str = "website/src/data/support.json";

/// Words the generated tables never use (#1496): a capability table is not a
/// conformance result.
pub const FORBIDDEN_WORDS: &[&str] = &["conformant", "compliant", "certified"];

/// The data links and the `bacnet-transport` feature that builds each; `None`
/// is always built.
const DATA_LINKS: &[(&str, DataLinkSupport, Option<&str>)] = &[
    ("bip", DataLinkSupport::BipV4, None),
    ("bip6", DataLinkSupport::BipV6, Some("ipv6")),
    ("sc", DataLinkSupport::BacnetSc, Some("sc-tls")),
    ("mstp", DataLinkSupport::Mstp, Some("serial")),
    ("ethernet", DataLinkSupport::Ethernet, Some("ethernet")),
];

/// `bacnet-transport` features that add no data link. A new feature that is
/// in neither list fails generation until it is classified.
const NON_LINK_FEATURES: &[&str] = &["default", "serial-gpio", "test-support"];

/// Method-name endings that still send the service the name starts with, as
/// in `who_has_by_id` or `acknowledge_alarm_request`.
const PYTHON_METHOD_SUFFIXES: &[&str] = &[
    "",
    "_typed",
    "_request",
    "_directed",
    "_by_id",
    "_by_name",
    "_replies",
    "_from_device",
    "_to_device",
];

type Result<T> = std::result::Result<T, String>;

fn read(repo: &Path, path: &str) -> Result<String> {
    std::fs::read_to_string(repo.join(path))
        .map(|text| text.replace("\r\n", "\n"))
        .map_err(|error| format!("read {path}: {error}"))
}

/// The whole file, as written: pretty JSON with a final newline.
pub fn generate(repo: &Path) -> Result<String> {
    let release = site_release(&read(repo, "website/src/lib/site.mjs")?)?;
    let stub = read(repo, "crates/rusty-bacnet/rusty_bacnet.pyi")?;
    let manifests = Manifests::load(repo)?;
    let cli_targets = cli_targets(&read(repo, "scripts/release/third_party_notices.py")?)?;
    let document = json!({
        "generatedBy": "crates/bacnet-integration-tests/support_tables.rs; run `cargo run -p bacnet-integration-tests --example support-tables`, do not edit",
        "release": release,
        "tracks": "dev",
        "objectTypes": object_types()?,
        "excludedObjectTypes": bacnet_objects::catalogue::EXCLUDED_TYPES
            .iter()
            .map(|t| Ok(json!({ "id": t.to_raw(), "name": name_of(t.name(), t.to_raw())? })))
            .collect::<Result<Vec<_>>>()?,
        "services": services(&python_services(&stub)?)?,
        "cliTargets": cli_targets.keys().collect::<Vec<_>>(),
        "dataLinks": data_links(&manifests, &cli_targets)?,
    });
    let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())? + "\n";
    let lower = text.to_lowercase();
    if let Some(word) = FORBIDDEN_WORDS.iter().find(|word| lower.contains(*word)) {
        return Err(format!("the generated tables must not say {word:?}"));
    }
    Ok(text)
}

fn name_of(name: Option<&str>, raw: impl std::fmt::Display) -> Result<&str> {
    name.ok_or_else(|| format!("value {raw} has no name"))
}

/// `export const release = '0.14.0';` in site.mjs.
fn site_release(site: &str) -> Result<String> {
    site.lines()
        .find_map(|line| line.trim().strip_prefix("export const release = '"))
        .and_then(|rest| rest.strip_suffix("';"))
        .map(str::to_owned)
        .ok_or_else(|| "site.mjs exports no release".to_owned())
}

/// Each catalogue type, with whether CreateObject and DeleteObject reach it,
/// as the PICS generator reports for a database holding the catalogue.
fn object_types() -> Result<Vec<Value>> {
    let mut db = ObjectDatabase::new();
    for object in bacnet_objects::catalogue::representatives() {
        db.add(object).map_err(|error| error.to_string())?;
    }
    let server = ServerConfig::default();
    let config = PicsConfig::default();
    let pics = PicsGenerator::new(&db, &server, &config).generate();
    pics.supported_object_types
        .iter()
        .map(|support| {
            let t = support.object_type;
            Ok(json!({
                "id": t.to_raw(),
                "name": name_of(t.name(), t.to_raw())?,
                "create": support.createable,
                "delete": support.deleteable,
            }))
        })
        .collect()
}

/// Whether a service bit is a confirmed or an unconfirmed service.
fn kind(service: ServiceSupported) -> Option<&'static str> {
    let confirmed = (0..=u8::MAX).any(|raw| {
        ServiceSupported::from_confirmed_choice(ConfirmedServiceChoice::from_raw(raw))
            == Some(service)
    });
    let unconfirmed = (0..=u8::MAX).any(|raw| {
        ServiceSupported::from_unconfirmed_choice(UnconfirmedServiceChoice::from_raw(raw))
            == Some(service)
    });
    match (confirmed, unconfirmed) {
        (true, false) => Some("confirmed"),
        (false, true) => Some("unconfirmed"),
        _ => None,
    }
}

/// One row per service that some surface executes, initiates or sends,
/// confirmed services first, each group by PICS name.
fn services(python: &BTreeSet<u8>) -> Result<Vec<Value>> {
    let bits = |list: &[ServiceSupported]| -> BTreeSet<u8> {
        list.iter().map(|service| service.to_raw()).collect()
    };
    let executed = bits(bacnet_objects::device::EXECUTED_SERVICES);
    let initiated = bits(PicsGenerator::INITIATED_SERVICES);
    let sent = bits(bacnet_client::client::SENT_SERVICES);
    let responder = bits(bacnet_endpoint::RESPONDER_SERVICES);
    let opt_in: BTreeMap<u8, &str> = bacnet_endpoint::RESPONDER_OPT_IN_SERVICES
        .iter()
        .map(|(service, method)| (service.to_raw(), *method))
        .collect();
    let mut rows = Vec::new();
    for raw in 0..=u8::MAX {
        let used = [&executed, &initiated, &sent, &responder, python]
            .iter()
            .any(|set| set.contains(&raw))
            || opt_in.contains_key(&raw);
        if !used {
            continue;
        }
        let service = ServiceSupported::from_raw(raw);
        let label = service_display_name(service);
        if label == "Unknown" {
            return Err(format!("service bit {raw} has no PICS name"));
        }
        let kind =
            kind(service).ok_or_else(|| format!("{label}: neither confirmed nor unconfirmed"))?;
        let endpoint = if responder.contains(&raw) {
            json!("always")
        } else {
            opt_in.get(&raw).map_or(Value::Null, |method| json!(method))
        };
        rows.push((
            (kind != "confirmed", label),
            json!({
                "bit": raw,
                "name": name_of(service.name(), raw)?,
                "label": label,
                "kind": kind,
                "serverExecutes": executed.contains(&raw),
                "serverInitiates": initiated.contains(&raw),
                "clientSends": sent.contains(&raw),
                "endpoint": endpoint,
                "pythonSends": python.contains(&raw),
            }),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows.into_iter().map(|(_, row)| row).collect())
}

/// The services the Python client's methods send, from the type stub: a
/// method of `BACnetClient` sends a service when its name is the service's
/// name in snake case (`read_property`, `vt_open`), alone or followed by one
/// of [`PYTHON_METHOD_SUFFIXES`].
pub fn python_services(stub: &str) -> Result<BTreeSet<u8>> {
    let start = stub
        .find("\nclass BACnetClient:")
        .ok_or("the stub has no BACnetClient class")?;
    let body = &stub[start + 1..];
    let end = body[1..].find("\nclass ").map_or(body.len(), |i| i + 1);
    let methods: BTreeSet<&str> = body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("    ")?;
            let line = line.strip_prefix("async ").unwrap_or(line);
            let name = line.strip_prefix("def ")?;
            name.split('(').next()
        })
        .collect();
    if !methods.contains("read_property") {
        return Err("the stub's BACnetClient has no read_property method".into());
    }
    Ok((0..=u8::MAX)
        .filter(|raw| {
            let Some(name) = ServiceSupported::from_raw(*raw).name() else {
                return false;
            };
            let snake = name.to_lowercase();
            PYTHON_METHOD_SUFFIXES
                .iter()
                .any(|suffix| methods.contains(format!("{snake}{suffix}").as_str()))
        })
        .collect())
}

/// `TARGETS` in third_party_notices.py: each CLI release target and the
/// features it builds with on top of the defaults.
fn cli_targets(script: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let start = script
        .find("\nTARGETS = {\n")
        .ok_or("no TARGETS in third_party_notices.py")?;
    let body = &script[start + "\nTARGETS = {\n".len()..];
    let body = &body[..body.find("\n}").ok_or("TARGETS is not closed")?];
    let mut targets = BTreeMap::new();
    for line in body.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        let [_, target, _, features, _] = parts[..] else {
            return Err(format!("unexpected TARGETS line {line:?}"));
        };
        let features = features
            .split(',')
            .filter(|f| !f.is_empty())
            .map(str::to_owned)
            .collect();
        targets.insert(target.to_owned(), features);
    }
    if targets.is_empty() {
        return Err("TARGETS is empty".into());
    }
    Ok(targets)
}

fn data_links(manifests: &Manifests, cli: &BTreeMap<String, Vec<String>>) -> Result<Vec<Value>> {
    let transport = manifests.get("bacnet-transport")?;
    for feature in transport.features.keys() {
        let mapped = DATA_LINKS
            .iter()
            .any(|(_, _, f)| *f == Some(feature.as_str()));
        if !mapped && !NON_LINK_FEATURES.contains(&feature.as_str()) {
            return Err(format!(
                "bacnet-transport feature {feature} is neither a data link nor listed as none"
            ));
        }
    }
    let python = manifests.transport_features("rusty-bacnet", &[])?;
    let cli: BTreeMap<&String, BTreeSet<String>> = cli
        .iter()
        .map(|(target, features)| {
            Ok((
                target,
                manifests.transport_features("bacnet-cli", features)?,
            ))
        })
        .collect::<Result<_>>()?;
    DATA_LINKS
        .iter()
        .map(|(id, link, feature)| {
            if let Some(feature) = feature
                && !transport.features.contains_key(*feature)
            {
                return Err(format!("bacnet-transport has no feature {feature}"));
            }
            let built = |features: &BTreeSet<String>| feature.is_none_or(|f| features.contains(f));
            Ok(json!({
                "id": id,
                "label": link.to_string(),
                "feature": feature,
                "python": built(&python),
                "cli": cli
                    .iter()
                    .map(|(target, features)| ((*target).clone(), json!(built(features))))
                    .collect::<serde_json::Map<_, _>>(),
            }))
        })
        .collect()
}

/// One normal dependency on another workspace crate.
struct Dependency {
    package: String,
    optional: bool,
    default_features: bool,
    features: Vec<String>,
}

/// The parts of a workspace member's manifest that decide its features.
struct Manifest {
    features: BTreeMap<String, Vec<String>>,
    dependencies: BTreeMap<String, Dependency>,
}

/// Every workspace member, by package name.
struct Manifests(BTreeMap<String, Manifest>);

fn table<'a>(value: &'a toml::Value, key: &str) -> Option<&'a toml::Table> {
    value.get(key)?.as_table()
}

fn strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

impl Manifests {
    fn load(repo: &Path) -> Result<Self> {
        let parse = |path: &str| -> Result<toml::Value> {
            toml::from_str::<toml::Table>(&read(repo, path)?)
                .map(toml::Value::Table)
                .map_err(|e| format!("{path}: {e}"))
        };
        let root = parse("Cargo.toml")?;
        let workspace = table(&root, "workspace").ok_or("no [workspace]")?;
        let shared = workspace
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .ok_or("no [workspace.dependencies]")?;
        let mut members = BTreeMap::new();
        for member in strings(workspace.get("members")) {
            let manifest = parse(&format!("{member}/Cargo.toml"))?;
            let name = table(&manifest, "package")
                .and_then(|p| p.get("name")?.as_str())
                .ok_or(format!("{member} has no package name"))?
                .to_owned();
            let features = table(&manifest, "features")
                .map(|t| {
                    t.iter()
                        .map(|(k, v)| (k.clone(), strings(Some(v))))
                        .collect()
                })
                .unwrap_or_default();
            // Normal dependencies, including target-specific ones.
            let mut sections: Vec<&toml::Table> =
                table(&manifest, "dependencies").into_iter().collect();
            if let Some(targets) = table(&manifest, "target") {
                sections.extend(targets.values().filter_map(|t| table(t, "dependencies")));
            }
            let mut dependencies = BTreeMap::new();
            for section in sections {
                for (key, spec) in section {
                    let inherited =
                        spec.get("workspace").and_then(toml::Value::as_bool) == Some(true);
                    let base = if inherited {
                        shared.get(key)
                    } else {
                        Some(spec)
                    };
                    let is_local = base.is_some_and(|b| b.get("path").is_some());
                    if !is_local {
                        continue;
                    }
                    let base = base.expect("checked above");
                    let mut features = strings(base.get("features"));
                    if inherited {
                        features.extend(strings(spec.get("features")));
                    }
                    let flag = |value: &toml::Value, key: &str| {
                        value.get(key).and_then(toml::Value::as_bool)
                    };
                    dependencies.insert(
                        key.clone(),
                        Dependency {
                            package: key.clone(),
                            optional: flag(spec, "optional").unwrap_or(false),
                            default_features: flag(spec, "default-features")
                                .or_else(|| flag(base, "default-features"))
                                .unwrap_or(true),
                            features,
                        },
                    );
                }
            }
            members.insert(
                name,
                Manifest {
                    features,
                    dependencies,
                },
            );
        }
        Ok(Self(members))
    }

    fn get(&self, package: &str) -> Result<&Manifest> {
        self.0
            .get(package)
            .ok_or_else(|| format!("no workspace member {package}"))
    }

    /// The `bacnet-transport` features a build of `package` with its default
    /// features and `features` turns on.
    fn transport_features(&self, package: &str, features: &[String]) -> Result<BTreeSet<String>> {
        let mut state = Resolver {
            manifests: self,
            crates: BTreeSet::new(),
            features: BTreeSet::new(),
            deps: BTreeSet::new(),
        };
        state.add_crate(package, true)?;
        for feature in features {
            state.add_feature(package, feature)?;
        }
        Ok(state
            .features
            .into_iter()
            .filter(|(crate_name, _)| crate_name == "bacnet-transport")
            .map(|(_, feature)| feature)
            .collect())
    }
}

/// A small version of Cargo's feature resolution, over workspace crates only.
struct Resolver<'a> {
    manifests: &'a Manifests,
    crates: BTreeSet<String>,
    features: BTreeSet<(String, String)>,
    /// Enabled dependencies, as (crate, dependency key).
    deps: BTreeSet<(String, String)>,
}

impl Resolver<'_> {
    fn add_crate(&mut self, package: &str, default: bool) -> Result<()> {
        if self.crates.insert(package.to_owned()) {
            let manifest = self.manifests.get(package)?;
            for (key, dependency) in &manifest.dependencies {
                if !dependency.optional {
                    self.add_dependency(package, key)?;
                }
            }
        }
        if default {
            self.add_feature(package, "default")?;
        }
        Ok(())
    }

    fn add_dependency(&mut self, package: &str, key: &str) -> Result<()> {
        let manifest = self.manifests.get(package)?;
        let Some(dependency) = manifest.dependencies.get(key) else {
            return Ok(()); // Not a workspace crate: its features don't matter here.
        };
        if !self.deps.insert((package.to_owned(), key.to_owned())) {
            return Ok(());
        }
        self.add_crate(&dependency.package, dependency.default_features)?;
        for feature in &dependency.features {
            self.add_feature(&dependency.package, feature)?;
        }
        // Weak features (`dep?/feature`) named earlier wait for the dependency.
        let waiting: Vec<(String, String)> = self
            .features
            .iter()
            .filter(|(crate_name, _)| crate_name == package)
            .cloned()
            .collect();
        for (_, feature) in waiting {
            for entry in manifest.features.get(&feature).into_iter().flatten() {
                if let Some((dep, sub)) = entry.split_once("?/")
                    && dep == key
                {
                    self.add_feature(&dependency.package, sub)?;
                }
            }
        }
        Ok(())
    }

    fn add_feature(&mut self, package: &str, feature: &str) -> Result<()> {
        if !self
            .features
            .insert((package.to_owned(), feature.to_owned()))
        {
            return Ok(());
        }
        let manifest = self.manifests.get(package)?;
        let Some(entries) = manifest.features.get(feature) else {
            // An optional dependency's implicit feature, or "default" when none is declared.
            if manifest
                .dependencies
                .get(feature)
                .is_some_and(|d| d.optional)
            {
                self.add_dependency(package, feature)?;
            }
            return Ok(());
        };
        for entry in entries {
            if let Some(dep) = entry.strip_prefix("dep:") {
                self.add_dependency(package, dep)?;
            } else if let Some((dep, sub)) = entry.split_once("?/") {
                if self.deps.contains(&(package.to_owned(), dep.to_owned()))
                    && let Some(dependency) = manifest.dependencies.get(dep)
                {
                    self.add_feature(&dependency.package, sub)?;
                }
            } else if let Some((dep, sub)) = entry.split_once('/') {
                if let Some(dependency) = manifest.dependencies.get(dep) {
                    self.add_dependency(package, dep)?;
                    self.add_feature(&dependency.package, sub)?;
                }
            } else {
                self.add_feature(package, entry)?;
            }
        }
        Ok(())
    }
}
