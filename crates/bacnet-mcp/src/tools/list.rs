//! `bacnet_list_objects`: a device's Object_List, paged.

use std::sync::{Arc, PoisonError};

use bacnet_ops::engine::{MAX_OBJECTS, Wanted, unreachable};
use bacnet_ops::format::decode_all;
use bacnet_ops::parse::parse_object_type;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::{Spec, next, schema, start};
use crate::context::{CachedList, Call, Ctx, Device, DeviceArg, LIST_TTL};
use crate::errors::ToolError;
use crate::untrusted::envelope;
use crate::value::{object_text, row_text};

pub(super) const NAME: &str = "bacnet_list_objects";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "List a device's objects",
    description: "List one device's objects from its Object_List, paged (read whole, or element \
                  by element when it can't come in one answer, and kept for 5 minutes for the \
                  next pages). Filter by object type; names: true also reads each listed \
                  object's name, one request each, so its pages are smaller. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// Objects on a page that reads names, one request each.
const NAMED_PAGE: usize = 50;

fn input() -> Value {
    schema::input(
        json!({
            "device": schema::device(),
            "types": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only these object types (ai, analog-output, trend-log, ...)",
            },
            "names": {
                "type": "boolean",
                "default": false,
                "description": "Also read each listed object's Object_Name",
            },
            "cursor": schema::cursor(),
        }),
        &["device"],
    )
}

fn output() -> Value {
    schema::output(json!({
        "device": { "type": "integer" },
        "objects": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "object": { "type": "string" },
                    "name": schema::envelope(),
                },
                "required": ["object"],
            },
        },
        "total": {
            "type": ["integer", "null"],
            "description": "Objects matching the filter; null until the list is all read",
        },
        "listed": { "type": ["integer", "null"], "description": "Objects in the Object_List" },
        "list": {
            "type": "object",
            "description": "How the list was read; when not complete, call again with \
                            next_cursor to go on reading where it stopped",
            "properties": {
                "cached": { "type": "boolean" },
                "indexed": { "type": "boolean" },
                "complete": { "type": "boolean" },
                "read": { "type": "integer" },
                "elements": { "type": ["integer", "null"] },
                "next_element": { "type": ["integer", "null"] },
                "stopped": { "type": ["string", "null"] },
            },
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    device: DeviceArg,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    names: bool,
    cursor: Option<String>,
}

/// How the Object_List was had.
pub(super) struct Listing {
    pub(super) objects: Arc<Vec<ObjectIdentifier>>,
    cached: bool,
    /// Read element by element: the element count, and the next element.
    indexed: Option<(u32, u32)>,
    /// Why an element read stopped early, if one did.
    stopped: Option<String>,
}

impl Listing {
    /// Whether every entry has been read.
    pub(super) fn complete(&self) -> bool {
        self.indexed.is_none_or(|(total, next)| next > total)
    }

    /// How far the read has got, for a result.
    pub(super) fn progress(&self) -> Value {
        let (total, next) = self.indexed.unwrap_or((0, 0));
        json!({
            "cached": self.cached,
            "indexed": self.indexed.is_some(),
            "complete": self.complete(),
            "read": self.objects.len(),
            "elements": self.indexed.map(|_| total),
            "next_element": (!self.complete()).then_some(next),
            "stopped": self.stopped,
        })
    }
}

/// The object identifiers in an application-tagged value.
fn identifiers(data: &[u8]) -> Vec<ObjectIdentifier> {
    decode_all(data)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| match value {
            PropertyValue::ObjectIdentifier(object) => Some(object),
            _ => None,
        })
        .collect()
}

/// `device`'s Object_List, as far as this call's budget gets it.
///
/// A fresh complete list comes from the cache. Otherwise the list is read
/// whole, or, when the device can't send it in one answer, element by
/// element (element 0 holds the count). An element read stops when the
/// call's budget is spent (after at least one element) or an element
/// fails, and what was read is kept with where to go on, so the next call
/// continues rather than starts again.
pub(super) async fn listing<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    device: &Device,
) -> Result<Listing, ToolError> {
    let engine = ctx.engine()?;
    let failed = |e: &Error| ToolError::device(device.instance, e);
    let kept = {
        let lists = ctx.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists
            .get(&device.instance)
            .filter(|kept| kept.at.elapsed() < LIST_TTL)
            .cloned()
    };
    if let Some(kept) = &kept
        && kept.complete()
    {
        return Ok(Listing {
            objects: Arc::clone(&kept.objects),
            cached: true,
            indexed: kept.indexed,
            stopped: None,
        });
    }
    let list_object = ObjectIdentifier::new(ObjectType::DEVICE, device.instance)
        .map_err(|e| ToolError::invalid("device", e.to_string()))?;
    let list = PropertyIdentifier::OBJECT_LIST;
    let (mut objects, total, mut next, cached) = match kept {
        Some(kept) => {
            let (total, next) = kept.indexed.unwrap_or((0, 1));
            (kept.objects.as_ref().clone(), total, next, true)
        }
        None => match engine
            .read_property(device.instance, list_object, list, None)
            .await
        {
            Ok(data) => {
                let objects = Arc::new(identifiers(&data));
                store(ctx, device.instance, Arc::clone(&objects), None);
                return Ok(Listing {
                    objects,
                    cached: false,
                    indexed: None,
                    stopped: None,
                });
            }
            Err(e) if unreachable(&e) => return Err(failed(&e)),
            Err(whole) => {
                let count = engine
                    .read_property(device.instance, list_object, list, Some(0))
                    .await
                    .map_err(|_| failed(&whole))?;
                let total = match decode_all(&count).as_deref() {
                    Some([PropertyValue::Unsigned(n)]) => u32::try_from(*n).unwrap_or(u32::MAX),
                    _ => return Err(failed(&whole)),
                };
                if total > MAX_OBJECTS {
                    return Err(ToolError::Failed(format!(
                        "Object_List claims {total} objects; at most {MAX_OBJECTS} are read"
                    )));
                }
                (Vec::new(), total, 1, false)
            }
        },
    };
    let mut stopped = None;
    let mut read_here = 0;
    while next <= total {
        if read_here > 0 && call.out_of_time() {
            break;
        }
        match engine
            .read_property(device.instance, list_object, list, Some(next))
            .await
        {
            Ok(data) => {
                objects.extend(identifiers(&data));
                next += 1;
                read_here += 1;
            }
            Err(e) if read_here == 0 && objects.is_empty() && unreachable(&e) => {
                return Err(failed(&e));
            }
            Err(e) => {
                stopped = Some(format!("element {next} of {total}: {e}"));
                break;
            }
        }
    }
    let objects = Arc::new(objects);
    store(
        ctx,
        device.instance,
        Arc::clone(&objects),
        Some((total, next)),
    );
    Ok(Listing {
        objects,
        cached,
        indexed: Some((total, next)),
        stopped,
    })
}

/// Keep what was read of `device`'s list.
fn store<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: u32,
    objects: Arc<Vec<ObjectIdentifier>>,
    indexed: Option<(u32, u32)>,
) {
    ctx.lists
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            device,
            CachedList {
                at: Instant::now(),
                objects,
                indexed,
            },
        );
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let types = args
        .types
        .iter()
        .map(|name| parse_object_type(name).map_err(|e| ToolError::invalid("types", e)))
        .collect::<Result<Vec<ObjectType>, _>>()?;
    let device = ctx.device(call, &args.device).await?;
    let first = start(ctx, NAME, args.cursor.as_deref(), raw)? as usize;
    let listing = listing(ctx, call, &device).await?;
    if !listing.complete() {
        // Not all read yet: say how far it got, and hand back a cursor
        // for the same page, which goes on reading from there.
        return Ok(json!({
            "device": device.instance,
            "objects": [],
            "total": null,
            "listed": null,
            "list": listing.progress(),
            "truncated": true,
            "next_cursor": next(ctx, NAME, raw, first as u64),
            "limits": ctx.limits(None),
        }));
    }
    let matching: Vec<ObjectIdentifier> = listing
        .objects
        .iter()
        .copied()
        .filter(|object| types.is_empty() || types.contains(&object.object_type()))
        .collect();
    let page = if args.names {
        ctx.policy.caps.objects.min(NAMED_PAGE)
    } else {
        ctx.policy.caps.objects
    };
    let engine = ctx.engine()?;
    let cap = ctx.policy.caps.text;
    let mut objects = Vec::new();
    let mut resume = None;
    for (at, object) in matching.iter().enumerate().skip(first) {
        if at >= first + page || (args.names && at > first && call.out_of_time()) {
            resume = Some(at);
            break;
        }
        let text = object_text(object.object_type(), object.instance_number());
        let mut row = json!({ "object": text });
        if args.names {
            let name = engine
                .read(
                    device.instance,
                    *object,
                    Wanted::List(&[PropertyIdentifier::OBJECT_NAME]),
                )
                .await
                .ok()
                .and_then(|read| row_text(&read.rows, PropertyIdentifier::OBJECT_NAME));
            let source = format!("device {} {text} object-name", device.instance);
            row["name"] = name.map_or(Value::Null, |name| envelope(&name, source, cap));
        }
        objects.push(row);
    }
    Ok(json!({
        "device": device.instance,
        "objects": objects,
        "total": matching.len(),
        "listed": listing.objects.len(),
        "list": listing.progress(),
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at as u64)),
        "limits": ctx.limits(Some(page)),
    }))
}
