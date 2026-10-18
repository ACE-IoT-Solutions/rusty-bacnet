//! A requested write turned into a preview, from the properties read just
//! before it: the value to send, the pre-image, the slot's occupancy, what
//! Present_Value will do, and the confirmation tier. Pure: no I/O.
//!
//! Tiers (`docs/design/tui.md` section 5):
//!
//! | Change | Tier |
//! |---|---|
//! | priority 9 to 16, or a non-commandable property | T1 |
//! | priorities 3, 4, 7 and 8; Out_Of_Service; relinquishing another commander's slot; a batch | T2 |
//! | priorities 1, 2 and 5 (life safety and critical equipment, Table 19-1) | T3 |
//! | priority 6, kept for minimum on and off times (Clause 19.2.3) | blocked, or T3 with `dangerous = true` |

use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use super::{Effect, Gate, Occupancy, Planned, Preview, Stored, Tier, WriteSpec};
use crate::tui::point::PRIORITIES;
use crate::tui::worker::browse::format::{decode_all, value_text, RawRow};

/// What is read before a write, besides the target property.
pub(crate) const PRE_READ: [PropertyIdentifier; 4] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::PRIORITY_ARRAY,
    PropertyIdentifier::RELINQUISH_DEFAULT,
    PropertyIdentifier::OUT_OF_SERVICE,
];

/// The priority a commandable write without one gets (Clause 19.2.1).
pub(crate) const DEFAULT_PRIORITY: u8 = 16;

/// Why priority 6 is refused.
pub(crate) const PRIORITY_6: &str = "priority 6 is reserved for minimum on and off times \
     (Clause 19.2.3); a profile with dangerous = true allows it";

/// The tier of one write.
pub(crate) fn gate(
    property: PropertyIdentifier,
    priority: Option<u8>,
    relinquish: bool,
    occupancy: Occupancy,
    dangerous: bool,
) -> Gate {
    Gate::Allowed(match priority {
        Some(6) if !dangerous => return Gate::Blocked(PRIORITY_6.into()),
        Some(1 | 2 | 5 | 6) => Tier::T3,
        Some(3 | 4 | 7 | 8) => Tier::T2,
        Some(_) if relinquish && occupancy == Occupancy::Other => Tier::T2,
        Some(_) => Tier::T1,
        None if property == PropertyIdentifier::OUT_OF_SERVICE => Tier::T2,
        None => Tier::T1,
    })
}

/// The tier of a whole change: its strongest write's, at least T2 for a
/// batch; blocked when any write is.
pub(crate) fn overall(items: &[Planned], batch: bool) -> Gate {
    let mut tier = if batch { Tier::T2 } else { Tier::T1 };
    for item in items {
        match &item.gate {
            Gate::Blocked(why) => return Gate::Blocked(format!("{}: {why}", item.label())),
            Gate::Allowed(t) => tier = tier.max(*t),
        }
    }
    Gate::Allowed(tier)
}

/// Build the preview of `specs` from `rows`, one list per spec. `ours`
/// says whether the ledger holds a write to that spec's slot.
pub(crate) fn preview(
    specs: &[WriteSpec],
    rows: &[Vec<RawRow>],
    ours: impl Fn(&WriteSpec, Option<u8>) -> bool,
    dangerous: bool,
    batch: bool,
) -> Result<Preview, String> {
    let device = specs.first().ok_or("nothing to write")?.device;
    let items = specs
        .iter()
        .zip(rows)
        .map(|(spec, rows)| plan(spec, rows, &ours, dangerous))
        .collect::<Result<Vec<_>, _>>()?;
    let gate = overall(&items, batch);
    Ok(Preview {
        device,
        items,
        batch,
        gate,
    })
}

fn raw(rows: &[RawRow], property: PropertyIdentifier) -> Option<&[u8]> {
    rows.iter()
        .find(|(p, _)| *p == property)
        .and_then(|(_, value)| value.as_deref().ok())
}

fn first(data: &[u8]) -> Option<PropertyValue> {
    decode_all(data)?.into_iter().next()
}

/// Give a typed number the datatype the property already holds:
/// `parse_value` reads `42` as Unsigned, which an analog Present_Value
/// would refuse as the wrong datatype.
pub(crate) fn coerce(value: PropertyValue, like: Option<&PropertyValue>) -> PropertyValue {
    use PropertyValue as V;
    match (like, value) {
        (Some(V::Real(_)), V::Unsigned(n)) => V::Real(n as f32),
        (Some(V::Real(_)), V::Signed(n)) => V::Real(n as f32),
        (Some(V::Double(_)), V::Unsigned(n)) => V::Double(n as f64),
        (Some(V::Double(_)), V::Signed(n)) => V::Double(f64::from(n)),
        (Some(V::Double(_)), V::Real(n)) => V::Double(f64::from(n)),
        (Some(V::Enumerated(_)), V::Unsigned(n)) if n <= u64::from(u32::MAX) => {
            V::Enumerated(n as u32)
        }
        (Some(V::Signed(_)), V::Unsigned(n)) if n <= i32::MAX as u64 => V::Signed(n as i32),
        (_, value) => value,
    }
}

fn stored(property: PropertyIdentifier, value: &PropertyValue) -> Result<Stored, String> {
    let mut buf = BytesMut::new();
    encode_property_value(&mut buf, value).map_err(|e| format!("cannot encode the value: {e}"))?;
    Ok(Stored {
        text: value_text(property, &buf),
        bytes: buf.to_vec(),
    })
}

/// The slot in control and Present_Value's text for `slots`, falling back
/// to Relinquish_Default when every slot is NULL (Clause 19.2).
fn effective(slots: &[PropertyValue], default: Option<&str>) -> (Option<u8>, String) {
    match slots.iter().position(|v| *v != PropertyValue::Null) {
        Some(i) => {
            let text =
                stored(PropertyIdentifier::PRESENT_VALUE, &slots[i]).map_or_else(|e| e, |s| s.text);
            (Some(i as u8 + 1), text)
        }
        None => (None, default.unwrap_or("?").to_string()),
    }
}

/// One write's preview.
fn plan(
    spec: &WriteSpec,
    rows: &[RawRow],
    ours: &impl Fn(&WriteSpec, Option<u8>) -> bool,
    dangerous: bool,
) -> Result<Planned, String> {
    let slots = raw(rows, PropertyIdentifier::PRIORITY_ARRAY)
        .and_then(decode_all)
        .filter(|values| values.len() == PRIORITIES);
    let commandable = spec.property == PropertyIdentifier::PRESENT_VALUE
        && spec.index.is_none()
        && slots.is_some();
    let current = if spec.index.is_none() {
        raw(rows, spec.property)
    } else {
        None
    };
    let default = raw(rows, PropertyIdentifier::RELINQUISH_DEFAULT);
    let mut notes = Vec::new();
    let value = match &spec.value {
        None if !commandable => {
            return Err(
                "relinquishing needs a commandable Present_Value (one with a Priority_Array)"
                    .into(),
            )
        }
        None => PropertyValue::Null,
        Some(text) => {
            let like = current.or(default).and_then(first);
            coerce(crate::parse::parse_value(text)?, like.as_ref())
        }
    };
    let value = stored(spec.property, &value)?;
    let out_of_service = match raw(rows, PropertyIdentifier::OUT_OF_SERVICE).and_then(first) {
        Some(PropertyValue::Boolean(on)) => Some(on),
        _ => None,
    };
    let default_text = default.map(|d| value_text(PropertyIdentifier::PRESENT_VALUE, d));
    let (priority, before, occupancy, effect) = match slots {
        Some(slots) if commandable => {
            let priority = spec.priority.unwrap_or(DEFAULT_PRIORITY);
            if !(1..=16).contains(&priority) {
                return Err(format!("priority must be 1-16, got {priority}"));
            }
            let slot = &slots[usize::from(priority) - 1];
            let before = match slot {
                PropertyValue::Null => None,
                held => Some(stored(PropertyIdentifier::PRESENT_VALUE, held)?),
            };
            let occupancy = match &before {
                None => Occupancy::Empty,
                Some(_) if ours(spec, Some(priority)) => Occupancy::Ours,
                Some(_) => Occupancy::Other,
            };
            let (held_by, now) = effective(&slots, default_text.as_deref());
            let mut after = slots.clone();
            let new = first(&value.bytes).unwrap_or(PropertyValue::Null);
            after[usize::from(priority) - 1] = new;
            let (_, then) = effective(&after, default_text.as_deref());
            let effect = if now == then {
                Effect::Unchanged {
                    held_by: held_by.filter(|slot| *slot < priority),
                }
            } else {
                Effect::Changes {
                    from: now,
                    to: then,
                }
            };
            (Some(priority), before, occupancy, effect)
        }
        _ => {
            if spec.priority.is_some() {
                notes.push("not commandable: the priority is not sent (Clause 19.2.1)".into());
            }
            let before = match current {
                Some(bytes) => Some(Stored {
                    text: value_text(spec.property, bytes),
                    bytes: bytes.to_vec(),
                }),
                None => {
                    notes.push("the old value could not be read, so this can't be reverted".into());
                    None
                }
            };
            let effect = if spec.property == PropertyIdentifier::PRESENT_VALUE {
                if out_of_service == Some(false) {
                    notes.push(
                        "Out_Of_Service is FALSE: a device usually refuses this write".into(),
                    );
                }
                Effect::Changes {
                    from: before.as_ref().map_or("?".into(), |b| b.text.clone()),
                    to: value.text.clone(),
                }
            } else {
                Effect::NotPresentValue
            };
            (None, before, Occupancy::NotCommandable, effect)
        }
    };
    if occupancy == Occupancy::Other {
        notes.push(
            "the slot holds another commander's value; the write replaces it and they are not told"
                .into(),
        );
    }
    let gate = gate(
        spec.property,
        priority,
        spec.value.is_none(),
        occupancy,
        dangerous,
    );
    Ok(Planned {
        spec: spec.clone(),
        priority,
        value,
        commandable,
        before,
        occupancy,
        effect,
        out_of_service,
        notes,
        gate,
    })
}

#[cfg(test)]
mod tests {
    use bacnet_types::enums::ObjectType;
    use bacnet_types::primitives::ObjectIdentifier;

    use super::*;

    fn encoded(values: &[PropertyValue]) -> Vec<u8> {
        let mut buf = BytesMut::new();
        for value in values {
            encode_property_value(&mut buf, value).unwrap();
        }
        buf.to_vec()
    }

    /// AO:1 of the lab: 55 at priority 10, Relinquish_Default 0.
    fn ao1() -> Vec<RawRow> {
        let mut slots = vec![PropertyValue::Null; 16];
        slots[9] = PropertyValue::Real(55.0);
        vec![
            (
                PropertyIdentifier::PRESENT_VALUE,
                Ok(encoded(&[PropertyValue::Real(55.0)])),
            ),
            (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
            (
                PropertyIdentifier::RELINQUISH_DEFAULT,
                Ok(encoded(&[PropertyValue::Real(0.0)])),
            ),
            (
                PropertyIdentifier::OUT_OF_SERVICE,
                Ok(encoded(&[PropertyValue::Boolean(false)])),
            ),
        ]
    }

    fn spec(value: Option<&str>, priority: u8) -> WriteSpec {
        WriteSpec {
            device: 100,
            object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
            value: value.map(str::to_string),
            priority: Some(priority),
        }
    }

    fn one(spec: WriteSpec) -> Planned {
        let preview = preview(&[spec], &[ao1()], |_, _| false, false, false).unwrap();
        preview.items.into_iter().next().unwrap()
    }

    #[test]
    fn a_weaker_priority_leaves_present_value_alone() {
        let planned = one(spec(Some("42"), 12));
        assert_eq!(planned.effect, Effect::Unchanged { held_by: Some(10) });
        assert_eq!(planned.occupancy, Occupancy::Empty);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T1));
        // The integer was sent as the REAL the object holds.
        assert_eq!(planned.value.bytes, encoded(&[PropertyValue::Real(42.0)]));
    }

    #[test]
    fn priority_8_changes_present_value_and_needs_t2() {
        let planned = one(spec(Some("42"), 8));
        assert_eq!(
            planned.effect,
            Effect::Changes {
                from: "55".into(),
                to: "42".into()
            }
        );
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
        assert_eq!(planned.before, None, "slot 8 was empty");
    }

    #[test]
    fn tiers_follow_the_priority() {
        let tier = |p| {
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(p),
                false,
                Occupancy::Empty,
                false,
            )
        };
        for p in [1, 2, 5] {
            assert_eq!(tier(p), Gate::Allowed(Tier::T3), "priority {p}");
        }
        for p in [3, 4, 7, 8] {
            assert_eq!(tier(p), Gate::Allowed(Tier::T2), "priority {p}");
        }
        for p in 9..=16 {
            assert_eq!(tier(p), Gate::Allowed(Tier::T1), "priority {p}");
        }
        assert!(matches!(tier(6), Gate::Blocked(_)));
        assert_eq!(
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(6),
                false,
                Occupancy::Empty,
                true
            ),
            Gate::Allowed(Tier::T3),
            "dangerous allows it at T3"
        );
        assert_eq!(
            gate(
                PropertyIdentifier::PRESENT_VALUE,
                Some(12),
                true,
                Occupancy::Other,
                false
            ),
            Gate::Allowed(Tier::T2),
            "relinquishing someone else's slot"
        );
        assert_eq!(
            gate(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                false,
                Occupancy::NotCommandable,
                false
            ),
            Gate::Allowed(Tier::T2)
        );
    }

    #[test]
    fn relinquishing_the_slot_in_control_falls_back_and_marks_another_writer() {
        let planned = one(spec(None, 10));
        assert_eq!(planned.occupancy, Occupancy::Other);
        assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
        assert_eq!(
            planned.effect,
            Effect::Changes {
                from: "55".into(),
                to: "0".into()
            }
        );
        assert_eq!(planned.before.unwrap().text, "55");
        let ours = preview(&[spec(None, 10)], &[ao1()], |_, _| true, false, false).unwrap();
        assert_eq!(ours.items[0].occupancy, Occupancy::Ours);
        assert_eq!(ours.gate, Gate::Allowed(Tier::T1));
    }

    #[test]
    fn a_batch_takes_its_strongest_tier_and_any_block() {
        let batch = preview(
            &[spec(Some("1"), 12), spec(Some("2"), 13)],
            &[ao1(), ao1()],
            |_, _| false,
            false,
            true,
        )
        .unwrap();
        assert_eq!(batch.gate, Gate::Allowed(Tier::T2));
        let blocked = preview(
            &[spec(Some("1"), 12), spec(Some("2"), 6)],
            &[ao1(), ao1()],
            |_, _| false,
            false,
            true,
        )
        .unwrap();
        assert!(matches!(blocked.gate, Gate::Blocked(_)));
    }

    #[test]
    fn a_non_commandable_property_keeps_its_old_value_as_the_pre_image() {
        let mut rows = ao1();
        rows.push((
            PropertyIdentifier::DESCRIPTION,
            Ok(encoded(&[PropertyValue::CharacterString("old".into())])),
        ));
        let mut description = spec(Some("\"new\""), 12);
        description.property = PropertyIdentifier::DESCRIPTION;
        let planned = preview(&[description], &[rows], |_, _| false, false, false)
            .unwrap()
            .items
            .remove(0);
        assert_eq!(planned.priority, None);
        assert_eq!(planned.before.unwrap().text, "\"old\"");
        assert_eq!(planned.effect, Effect::NotPresentValue);
        let mut relinquish = spec(None, 12);
        relinquish.property = PropertyIdentifier::DESCRIPTION;
        assert!(preview(&[relinquish], &[ao1()], |_, _| false, false, false).is_err());
    }
}
