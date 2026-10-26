//! Passwords in the decode tree (#2074): DeviceCommunicationControl and
//! ReinitializeDevice carry one, which the named fields show as hidden and
//! the tag walk hides wherever it may sit.

use bacnet_encoding::tags::{TagClass, decode_tag};
use bacnet_types::enums::ConfirmedServiceChoice as Confirmed;

use super::{Doc, Field, Params};

/// Where a password-bearing request keeps its password.
#[derive(Debug, Clone, Copy)]
pub(super) struct Secret {
    /// The context tag of the password.
    tag: u8,
    /// The context tags of its other fields, short primitives (a
    /// duration, an enumeration) shown as the tag walk writes them.
    plain: &'static [u8],
}

impl Params {
    /// Where a password-bearing request keeps its password.
    pub(super) fn secret(self) -> Option<Secret> {
        match self {
            Self::Request(Confirmed::DEVICE_COMMUNICATION_CONTROL) => Some(Secret {
                tag: 2,
                plain: &[0, 1],
            }),
            Self::Request(Confirmed::REINITIALIZE_DEVICE) => Some(Secret {
                tag: 1,
                plain: &[0],
            }),
            _ => None,
        }
    }
}

/// What the password field shows, on either path.
pub(super) const PASSWORD: &str = "Password: (hidden)";

/// What any other field that may hold a password shows on the tag walk.
const MAYBE_PASSWORD: &str = "Hidden: may hold a password";

/// On the tag walk of a password-bearing request, whose decoder refused
/// it or saw only a segment: hide every value at any depth that may hold
/// the password. Only the request's own short fields at the top level
/// ([`Secret::plain`]) and the tags of constructed values stay.
pub(super) fn hide_passwords(fields: &mut [Field], doc: &Doc<'_>, secret: Secret, depth: usize) {
    for field in fields {
        if !field.children.is_empty() {
            hide_passwords(&mut field.children, doc, secret, depth + 1);
            continue;
        }
        let header = field
            .range
            .start
            .checked_sub(doc.at)
            .and_then(|at| decode_tag(doc.data, at).ok())
            .map(|(header, _)| header);
        let label = match header {
            Some(header) if header.is_opening || header.is_closing => continue,
            Some(header) if depth == 0 && header.class == TagClass::Context => {
                if header.number == secret.tag {
                    PASSWORD
                } else if secret.plain.contains(&header.number) && header.length <= 4 {
                    continue;
                } else {
                    MAYBE_PASSWORD
                }
            }
            _ => MAYBE_PASSWORD,
        };
        field.label = label.into();
        field.device_text = false;
    }
}
