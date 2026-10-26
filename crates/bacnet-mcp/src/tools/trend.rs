//! `bacnet_read_trend`: a Trend Log or Trend Log Multiple buffer, by
//! ReadRange, paged across the device's answers and across calls.

use bacnet_client::log_reader::{LogCursor, LogPage};
use bacnet_ops::parse::parse_object_specifier;
use bacnet_services::read_range::LogRecords;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::trend_data::{Entry, Summary, stamp_text};
use super::{MAX_STRUCTURED, Spec, next, schema, start};
use crate::context::{Call, Ctx, DeviceArg};
use crate::errors::ToolError;
use crate::value::object_text;

pub(super) const NAME: &str = "bacnet_read_trend";

pub(super) const SPEC: Spec = Spec {
    name: NAME,
    title: "Read a trend log",
    description: "Read a Trend Log or Trend Log Multiple's records (ReadRange), oldest first: \
                  from (records logged after this local time, YYYY-MM-DDTHH:MM[:SS]), to (up \
                  to this time), or from the oldest; at most max_records a call (default \
                  500), then next_cursor goes on. Each record has its time, sequence number, \
                  values in typed form, and flagged when its status flags show a problem or \
                  it records a change in the log's own status. The result summarises the \
                  records it read: count, first and last time, and for each column min, max \
                  and mean of numbers and counts of states. summary: true reads the whole \
                  window on the server (up to 20000 records) and returns only the summary. \
                  Each request is charged. Read-only.",
    input,
    output,
    idempotent: true,
    open_world: true,
    passive: false,
};

/// The most records one call returns.
const MAX_RECORDS: u32 = 2_000;

/// The records a call returns unless asked otherwise.
const DEFAULT_RECORDS: u32 = 500;

/// The most records a summary call reads.
const MAX_SUMMARY: u32 = 20_000;

/// The records asked for in one ReadRange.
const PAGE: u32 = 200;

/// A cursor position resuming by position rather than sequence number.
const BY_POSITION: u64 = 1 << 62;

fn input() -> Value {
    let time = |what: &str| {
        json!({
            "type": "string",
            "pattern": "^[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}(:[0-9]{2}(\\.[0-9]{1,2})?)?$",
            "description": format!("{what}, the device's local time: YYYY-MM-DDTHH:MM[:SS[.hh]]"),
        })
    };
    schema::input(
        json!({
            "device": schema::device(),
            "object": {
                "type": "string",
                "description": "The log as type:instance: trend-log:1, trend-log-multiple:2",
            },
            "from": time("Records logged after this time"),
            "to": time("Records logged up to this time"),
            "max_records": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RECORDS,
                "default": DEFAULT_RECORDS,
            },
            "summary": {
                "type": "boolean",
                "default": false,
                "description": "Read the whole window and return only the summary",
            },
            "cursor": schema::cursor(),
        }),
        &["device", "object"],
    )
}

fn output() -> Value {
    let column = json!({
        "type": "object",
        "properties": {
            "column": { "type": "integer" },
            "numeric": {
                "type": ["object", "null"],
                "properties": {
                    "count": { "type": "integer" },
                    "min": { "type": ["number", "null"] },
                    "max": { "type": ["number", "null"] },
                    "mean": { "type": ["number", "null"] },
                },
            },
            "states": { "type": ["object", "null"], "description": "Samples by state" },
            "failures": { "type": "integer" },
            "other": { "type": "integer" },
        },
    });
    let value = json!({
        "anyOf": [
            schema::typed(),
            {
                "type": "object",
                "properties": { "error": { "type": "object" } },
                "required": ["error"],
                "additionalProperties": false,
            },
        ],
    });
    schema::output(json!({
        "device": { "type": "integer" },
        "object": { "type": "string" },
        "records": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "time": { "type": "string" },
                    "seq": { "type": "integer" },
                    "kind": { "enum": ["values", "log-status", "time-change"] },
                    "flagged": { "type": "boolean" },
                    "values": { "type": "array", "items": value },
                    "status_flags": schema::typed(),
                    "log_status": schema::typed(),
                    "time_change_s": { "type": ["number", "null"] },
                },
                "required": ["time", "kind", "flagged"],
            },
        },
        "summary": {
            "type": "object",
            "description": "Of the records this call read",
            "properties": {
                "records": { "type": "integer" },
                "flagged": { "type": "integer" },
                "log_status": { "type": "integer" },
                "time_changes": { "type": "integer" },
                "first_time": { "type": ["string", "null"] },
                "last_time": { "type": ["string", "null"] },
                "columns": { "type": "array", "items": column },
            },
        },
        "pages": { "type": "integer", "description": "ReadRange answers read" },
        "more_items": {
            "type": "integer",
            "description": "Answers the device cut short to fit (MORE_ITEMS)",
        },
        "gaps": {
            "type": "array",
            "description": "Records the log no longer held when read",
            "items": { "type": "object" },
        },
        "done": {
            "type": "boolean",
            "description": "The read reached the newest record or the end of the window",
        },
        "stopped": { "type": "object" },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    device: DeviceArg,
    object: String,
    from: Option<String>,
    to: Option<String>,
    max_records: Option<u32>,
    #[serde(default)]
    summary: bool,
    cursor: Option<String>,
}

/// A local date and time argument.
fn parse_time(field: &'static str, text: &str) -> Result<(Date, Time), ToolError> {
    let bad = || ToolError::invalid(field, format!("'{text}' is not YYYY-MM-DDTHH:MM[:SS[.hh]]"));
    let (date, time) = text.split_once(['T', ' ']).ok_or_else(bad)?;
    let mut ymd = date.splitn(3, '-').map(str::parse::<u16>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) = (ymd.next(), ymd.next(), ymd.next())
    else {
        return Err(bad());
    };
    let (clock, hundredths) = match time.split_once('.') {
        Some((clock, fraction)) => {
            let digits: u8 = fraction.parse().map_err(|_| bad())?;
            (
                clock,
                if fraction.len() == 1 {
                    digits * 10
                } else {
                    digits
                },
            )
        }
        None => (time, 0),
    };
    let mut hms = clock.splitn(3, ':').map(str::parse::<u8>);
    let (Some(Ok(hour)), Some(Ok(minute))) = (hms.next(), hms.next()) else {
        return Err(bad());
    };
    let second = match hms.next() {
        Some(Ok(second)) => second,
        Some(Err(_)) => return Err(bad()),
        None => 0,
    };
    let days = days_in(year, month).ok_or_else(bad)?;
    if !(1900..=2154).contains(&year)
        || day == 0
        || u32::from(day) > days
        || hour > 23
        || minute > 59
        || second > 59
        || hundredths > 99
    {
        return Err(bad());
    }
    let date = Date {
        year: u8::try_from(year - 1900).map_err(|_| bad())?,
        month: u8::try_from(month).map_err(|_| bad())?,
        day: u8::try_from(day).map_err(|_| bad())?,
        day_of_week: weekday(year, month, day),
    };
    let time = Time {
        hour,
        minute,
        second,
        hundredths,
    };
    Ok((date, time))
}

/// Days in `month` of `year`, or `None` for no such month.
fn days_in(year: u16, month: u16) -> Option<u32> {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    })
}

/// The day of the week, 1 for Monday to 7 for Sunday (Sakamoto's method).
fn weekday(year: u16, month: u16, day: u16) -> u8 {
    const OFFSETS: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = u32::from(if month < 3 { year - 1 } else { year });
    let sunday_zero =
        (y + y / 4 - y / 100 + y / 400 + OFFSETS[usize::from(month - 1)] + u32::from(day)) % 7;
    // 0 is Sunday here; BACnet counts Monday as 1.
    u8::try_from(if sunday_zero == 0 { 7 } else { sunday_zero }).unwrap_or(7)
}

/// A log cursor as a cursor position, and back.
fn pack(cursor: LogCursor) -> Option<u64> {
    match cursor {
        LogCursor::Sequence(seq) if seq < BY_POSITION => Some(seq),
        LogCursor::Position(at) if at < BY_POSITION => Some(BY_POSITION | at),
        _ => None,
    }
}

fn unpack(position: u64) -> LogCursor {
    if position & BY_POSITION != 0 {
        LogCursor::Position(position & !BY_POSITION)
    } else {
        LogCursor::Sequence(position)
    }
}

/// The sequence number `n` records after `first` in a Trend Log's range,
/// 1 up to 2^32 - 1 and round again.
fn advance(first: u64, n: u64) -> u64 {
    if first == 0 {
        return n;
    }
    (first - 1 + n) % u64::from(u32::MAX) + 1
}

/// A page's records, oldest first.
fn entries(page: &LogPage) -> Vec<Entry> {
    match &page.records {
        LogRecords::TrendLog(records) => records.iter().map(Entry::trend).collect(),
        LogRecords::TrendLogMultiple(records) => records.iter().map(Entry::multiple).collect(),
        _ => Vec::new(),
    }
}

/// Where a call's reading stands.
#[derive(Default)]
struct Read {
    rows: Vec<Value>,
    size: usize,
    count: u32,
    summary: Summary,
    pages: u32,
    more_items: u32,
    gaps: Vec<Value>,
    done: bool,
    stopped: Option<Value>,
    resume: Option<LogCursor>,
}

pub(super) async fn run<T: TransportPort + 'static>(
    ctx: &Ctx<T>,
    call: &Call,
    args: Args,
    raw: &Map<String, Value>,
) -> Result<Value, ToolError> {
    let (kind, instance) =
        parse_object_specifier(&args.object).map_err(|e| ToolError::invalid("object", e))?;
    if kind != ObjectType::TREND_LOG && kind != ObjectType::TREND_LOG_MULTIPLE {
        return Err(ToolError::invalid(
            "object",
            "a trend-log or trend-log-multiple object",
        ));
    }
    let log = ObjectIdentifier::new(kind, instance)
        .map_err(|e| ToolError::invalid("object", e.to_string()))?;
    let from = args
        .from
        .as_deref()
        .map(|text| parse_time("from", text))
        .transpose()?;
    let to = args
        .to
        .as_deref()
        .map(|text| parse_time("to", text).map(|(d, t)| stamp_text(&d, &t)))
        .transpose()?;
    let limit = if args.summary {
        MAX_SUMMARY
    } else {
        let wanted = args.max_records.unwrap_or(DEFAULT_RECORDS);
        if !(1..=MAX_RECORDS).contains(&wanted) {
            return Err(ToolError::invalid(
                "max_records",
                format!("1 to {MAX_RECORDS}"),
            ));
        }
        wanted
    };
    let device = ctx.device(call, &args.device).await?;
    let engine = ctx.engine()?;
    let mut cursor = match args.cursor.as_deref() {
        Some(_) => unpack(start(ctx, NAME, args.cursor.as_deref(), raw)?),
        None => from.map_or(LogCursor::Oldest, |(date, time)| {
            LogCursor::Time(date, time)
        }),
    };
    let object = object_text(kind, instance);
    let source = format!("device {} {object} log-buffer", device.instance);
    let cap = ctx.policy.caps.text;
    let mut read = Read::default();
    loop {
        if read.count >= limit || read.size >= MAX_STRUCTURED {
            read.resume = Some(cursor);
            break;
        }
        if read.pages > 0 && call.out_of_time() {
            read.resume = Some(cursor);
            break;
        }
        let want = u16::try_from((limit - read.count).min(PAGE)).unwrap_or(1);
        let page = match engine.read_trend(device.instance, log, cursor, want).await {
            Ok(page) => page,
            Err(error) if read.pages == 0 => {
                return Err(ToolError::device(device.instance, &error));
            }
            Err(error) => {
                read.stopped = Some(ToolError::device(device.instance, &error).json());
                read.resume = Some(cursor);
                break;
            }
        };
        read.pages += 1;
        if page.result_flags.2 {
            read.more_items += 1;
        }
        if let Some(gap) = page.gap {
            read.gaps.push(json!({
                "expected": gap.expected,
                "first": gap.first,
                "skipped": gap.skipped,
            }));
        }
        let mut stop = None;
        for (i, entry) in entries(&page).iter().enumerate() {
            let n = u64::try_from(i).unwrap_or(u64::MAX);
            let seq = page.first_sequence_number.map(|first| advance(first, n));
            if to.as_ref().is_some_and(|to| entry.time > *to) {
                read.done = true;
                break;
            }
            if read.count >= limit || read.size >= MAX_STRUCTURED {
                stop = Some(match (seq, cursor) {
                    (Some(seq), _) => LogCursor::Sequence(seq),
                    (None, LogCursor::Position(at)) => LogCursor::Position(at + n),
                    // A page without numbers or positions: go on after it.
                    (None, _) => page.next,
                });
                break;
            }
            read.summary.add(entry);
            read.count += 1;
            if !args.summary {
                let row = entry.json(seq, &source, cap);
                read.size += row.to_string().len();
                read.rows.push(row);
            }
        }
        if read.done {
            break;
        }
        if let Some(stop) = stop {
            read.resume = Some(stop);
            break;
        }
        cursor = page.next;
        if page.done {
            read.done = true;
            break;
        }
    }
    let resume = read.resume.and_then(pack);
    let mut body = json!({
        "device": device.instance,
        "object": object,
        "records": read.rows,
        "summary": read.summary.json(),
        "pages": read.pages,
        "more_items": read.more_items,
        "gaps": read.gaps,
        "done": read.done,
        "truncated": resume.is_some(),
        "next_cursor": resume.map(|at| next(ctx, NAME, raw, at)),
        "limits": ctx.limits(Some(usize::try_from(limit).unwrap_or(usize::MAX))),
    });
    if let Some(stopped) = read.stopped {
        body["stopped"] = stopped;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_parse_with_their_weekday() {
        let (date, time) = parse_time("from", "2026-10-25T07:30").unwrap();
        assert_eq!((date.year, date.month, date.day), (126, 10, 25));
        // 25 October 2026 is a Sunday.
        assert_eq!(date.day_of_week, 7);
        assert_eq!((time.hour, time.minute, time.second), (7, 30, 0));
        let (date, time) = parse_time("to", "2024-02-29 23:59:59.5").unwrap();
        assert_eq!(date.day_of_week, 4, "a Thursday");
        assert_eq!(time.hundredths, 50);
        for bad in [
            "2026-02-30T00:00",
            "2026-13-01T00:00",
            "2026-10-25T24:00",
            "2026-10-25",
            "1899-01-01T00:00",
            "2026-10-25T07:00:00.123",
        ] {
            assert!(parse_time("from", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cursors_round_trip_and_sequences_wrap() {
        for cursor in [LogCursor::Sequence(17), LogCursor::Position(3)] {
            assert_eq!(unpack(pack(cursor).unwrap()), cursor);
        }
        assert_eq!(pack(LogCursor::Oldest), None);
        assert_eq!(advance(5, 3), 8);
        assert_eq!(advance(u64::from(u32::MAX), 1), 1);
        assert_eq!(advance(0, 2), 2);
    }
}
