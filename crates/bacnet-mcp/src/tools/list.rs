//! `bacnet_list_objects`: a device's Object_List, paged.

use std::sync::{Arc, PoisonError};

use bacnet_ops::engine::Wanted;
use bacnet_ops::parse::parse_object_type;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use super::{Spec, next, schema, start};
use crate::context::{Call, Ctx, Device, DeviceArg, LIST_TTL};
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
        "total": { "type": "integer", "description": "Objects matching the filter" },
        "listed": { "type": "integer", "description": "Objects in the Object_List" },
        "list": {
            "type": "object",
            "properties": {
                "cached": { "type": "boolean" },
                "indexed": { "type": "boolean" },
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
struct Listing {
    objects: Arc<Vec<ObjectIdentifier>>,
    cached: bool,
    indexed: bool,
    stopped: Option<String>,
}

/// `device`'s Object_List: from the cache while it is fresh, else read.
pub(super) async fn object_list<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: &Device,
) -> Result<(Arc<Vec<ObjectIdentifier>>, bool), ToolError> {
    let listing = listing(ctx, device).await?;
    Ok((listing.objects, listing.cached))
}

async fn listing<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    device: &Device,
) -> Result<Listing, ToolError> {
    let cached = {
        let lists = ctx.lists.lock().unwrap_or_else(PoisonError::into_inner);
        lists
            .get(&device.instance)
            .filter(|(at, _)| at.elapsed() < LIST_TTL)
            .map(|(_, objects)| Arc::clone(objects))
    };
    if let Some(objects) = cached {
        return Ok(Listing {
            objects,
            cached: true,
            indexed: false,
            stopped: None,
        });
    }
    let read = ctx
        .engine()?
        .list_objects(device.instance)
        .await
        .map_err(|e| ToolError::device(device.instance, &e))?;
    let objects = Arc::new(read.objects);
    // A list cut short isn't kept: the next page reads it again.
    if read.stopped.is_none() {
        ctx.lists
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(device.instance, (Instant::now(), Arc::clone(&objects)));
    }
    Ok(Listing {
        objects,
        cached: false,
        indexed: read.indexed,
        stopped: read.stopped,
    })
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
    let listing = listing(ctx, &device).await?;
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
        "list": {
            "cached": listing.cached,
            "indexed": listing.indexed,
            "stopped": listing.stopped,
        },
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at as u64)),
        "limits": ctx.limits(Some(page)),
    }))
}
