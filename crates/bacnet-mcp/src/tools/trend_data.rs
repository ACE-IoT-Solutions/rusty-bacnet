//! Trend log records as data, and the summary `bacnet_read_trend` computes
//! over them.
//!
//! A record is a sampled value (a typed datum, the CLI's JSON shape), the
//! error that stopped a sample, a change of the log's own status, or a
//! clock change. Records carrying status flags with any flag set, and log
//! status records, are flagged, since either means the samples around them
//! may not be what they seem.

use std::collections::BTreeMap;

use bacnet_ops::format::{Datum, Floats, StateTexts, ValueFormat};
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    BACnetLogMultipleRecord, BACnetLogRecord, LogData, LogDatum, LogValue,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::{Date, PropertyValue, StatusFlags, Time};
use serde_json::{Value, json};

use crate::errors::remote_json;
use crate::value::enveloped;

/// A date and time as `2026-10-25 07:00:00.00`, unspecified fields as 0,
/// so the text sorts as the time does.
pub(super) fn stamp_text(date: &Date, time: &Time) -> String {
    let part = |value: u8| if value == 0xFF { 0 } else { value };
    let year = if date.year == 0xFF {
        0
    } else {
        u16::from(date.year) + 1900
    };
    format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:02}",
        part(date.month),
        part(date.day),
        part(time.hour),
        part(time.minute),
        part(time.second),
        part(time.hundredths)
    )
}

/// What one log entry carries, ready for the result and the summary.
pub(super) struct Entry {
    pub(super) time: String,
    pub(super) kind: Kind,
    pub(super) status_flags: Option<StatusFlags>,
}

/// The parts of an entry.
pub(super) enum Kind {
    /// One sampled value or failure per column: one for a Trend Log, one
    /// per member for a Trend Log Multiple.
    Values(Vec<Sample>),
    LogStatus(LogStatus),
    TimeChange(f32),
}

/// One column's sample.
pub(super) enum Sample {
    Value(PropertyValue),
    /// A signed sample, which a log keeps to 64 bits.
    Signed(i64),
    /// A value of another datatype, as its tagged octets.
    Any(Vec<u8>),
    Failure(ErrorClass, ErrorCode),
}

impl Entry {
    /// A Trend Log record.
    pub(super) fn trend(record: &BACnetLogRecord) -> Self {
        let kind = match &record.log_datum {
            LogDatum::LogStatus(status) => Kind::LogStatus(*status),
            LogDatum::TimeChange(seconds) => Kind::TimeChange(*seconds),
            LogDatum::BooleanValue(v) => value(PropertyValue::Boolean(*v)),
            LogDatum::RealValue(v) => value(PropertyValue::Real(*v)),
            LogDatum::EnumValue(v) => value(enumerated(*v)),
            LogDatum::UnsignedValue(v) => value(PropertyValue::Unsigned(*v)),
            LogDatum::SignedValue(v) => Kind::Values(vec![Sample::Signed(*v)]),
            LogDatum::BitstringValue { unused_bits, data } => value(PropertyValue::BitString {
                unused_bits: *unused_bits,
                data: data.clone(),
            }),
            LogDatum::NullValue => value(PropertyValue::Null),
            LogDatum::Failure {
                error_class,
                error_code,
            } => Kind::Values(vec![Sample::Failure(*error_class, *error_code)]),
            LogDatum::AnyValue(octets) => Kind::Values(vec![Sample::Any(octets.clone())]),
        };
        Self {
            time: stamp_text(&record.date, &record.time),
            kind,
            status_flags: record.status_flags,
        }
    }

    /// A Trend Log Multiple record.
    pub(super) fn multiple(record: &BACnetLogMultipleRecord) -> Self {
        let kind = match &record.log_data {
            LogData::LogStatus(status) => Kind::LogStatus(*status),
            LogData::TimeChange(seconds) => Kind::TimeChange(*seconds),
            LogData::Values(values) => Kind::Values(values.iter().map(sample).collect()),
        };
        Self {
            time: stamp_text(&record.date, &record.time),
            kind,
            status_flags: None,
        }
    }

    /// Whether the record is flagged: status flags with any flag set, or a
    /// change of the log's status.
    pub(super) fn flagged(&self) -> bool {
        matches!(self.kind, Kind::LogStatus(_))
            || self.status_flags.is_some_and(|flags| !flags.is_empty())
    }

    /// The record as JSON: `seq` when the log numbered it, `source` naming
    /// the log for any device text in a value, `cap` capping it.
    pub(super) fn json(&self, seq: Option<u64>, source: &str, cap: usize) -> Value {
        let mut row = json!({ "time": self.time, "flagged": self.flagged() });
        if let Some(seq) = seq {
            row["seq"] = json!(seq);
        }
        if let Some(flags) = self.status_flags {
            row["status_flags"] = enveloped(&status_flags(flags), source, cap);
        }
        match &self.kind {
            Kind::Values(samples) => {
                row["kind"] = json!("values");
                let values: Vec<Value> = samples
                    .iter()
                    .map(|sample| sample_json(sample, source, cap))
                    .collect();
                row["values"] = json!(values);
            }
            Kind::LogStatus(status) => {
                row["kind"] = json!("log-status");
                row["log_status"] = enveloped(&log_status(*status), source, cap);
            }
            Kind::TimeChange(seconds) => {
                row["kind"] = json!("time-change");
                row["time_change_s"] = finite(f64::from(*seconds));
            }
        }
        row
    }
}

fn value(value: PropertyValue) -> Kind {
    Kind::Values(vec![Sample::Value(value)])
}

/// An enumerated sample: ENUMERATED when it fits 32 bits, else the number.
fn enumerated(raw: u64) -> PropertyValue {
    match u32::try_from(raw) {
        Ok(raw) => PropertyValue::Enumerated(raw),
        Err(_) => PropertyValue::Unsigned(raw),
    }
}

fn sample(value: &LogValue) -> Sample {
    match value {
        LogValue::BooleanValue(v) => Sample::Value(PropertyValue::Boolean(*v)),
        LogValue::RealValue(v) => Sample::Value(PropertyValue::Real(*v)),
        LogValue::EnumValue(v) => Sample::Value(enumerated(*v)),
        LogValue::UnsignedValue(v) => Sample::Value(PropertyValue::Unsigned(*v)),
        LogValue::SignedValue(v) => Sample::Signed(*v),
        LogValue::BitstringValue { unused_bits, data } => Sample::Value(PropertyValue::BitString {
            unused_bits: *unused_bits,
            data: data.clone(),
        }),
        LogValue::NullValue => Sample::Value(PropertyValue::Null),
        LogValue::Failure {
            error_class,
            error_code,
        } => Sample::Failure(*error_class, *error_code),
        LogValue::AnyValue(octets) => Sample::Any(octets.clone()),
    }
}

/// The formatter samples are named with: no state texts, every digit.
fn with_format<R>(f: impl FnOnce(&ValueFormat<'_>) -> R) -> R {
    let texts = StateTexts::default();
    f(&ValueFormat::new(
        ObjectType::TREND_LOG,
        &texts,
        Floats::Exact,
    ))
}

fn datum(value: &PropertyValue) -> Datum {
    with_format(|format| format.element_data(PropertyIdentifier::LOG_BUFFER, value))
}

fn status_flags(flags: StatusFlags) -> Datum {
    let value = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    };
    with_format(|format| format.element_data(PropertyIdentifier::STATUS_FLAGS, &value))
}

/// A log status as named bits.
fn log_status(status: LogStatus) -> Datum {
    let names: Vec<String> = [
        (LogStatus::LOG_DISABLED, "log-disabled"),
        (LogStatus::BUFFER_PURGED, "buffer-purged"),
        (LogStatus::LOG_INTERRUPTED, "log-interrupted"),
    ]
    .iter()
    .filter(|(bit, _)| status.contains(*bit))
    .map(|(_, name)| (*name).to_string())
    .collect();
    let bits: String = [
        LogStatus::LOG_DISABLED,
        LogStatus::BUFFER_PURGED,
        LogStatus::LOG_INTERRUPTED,
    ]
    .iter()
    .map(|bit| if status.contains(*bit) { '1' } else { '0' })
    .collect();
    Datum::Bits {
        bits,
        names: Some(names),
    }
}

fn sample_json(sample: &Sample, source: &str, cap: usize) -> Value {
    match sample {
        Sample::Value(value) => enveloped(&datum(value), source, cap),
        Sample::Signed(value) => json!(value),
        Sample::Any(octets) => {
            let decoded = with_format(|format| format.data(PropertyIdentifier::LOG_BUFFER, octets));
            enveloped(&decoded, source, cap)
        }
        Sample::Failure(class, code) => {
            let remote = RemoteError::Error {
                class: *class,
                code: *code,
            };
            json!({ "error": remote_json(&remote) })
        }
    }
}

/// A number JSON can hold, or null.
fn finite(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        Value::Null
    }
}

/// One column's statistics.
#[derive(Debug, Default, Clone)]
struct Column {
    numeric: u64,
    min: f64,
    max: f64,
    sum: f64,
    /// Booleans and enumerations, by value.
    states: BTreeMap<String, u64>,
    failures: u64,
    other: u64,
}

impl Column {
    fn add(&mut self, sample: &Sample) {
        let number = match sample {
            Sample::Value(PropertyValue::Real(v)) if v.is_finite() => Some(f64::from(*v)),
            Sample::Value(PropertyValue::Double(v)) if v.is_finite() => Some(*v),
            #[allow(clippy::cast_precision_loss)]
            Sample::Value(PropertyValue::Unsigned(v)) => Some(*v as f64),
            #[allow(clippy::cast_precision_loss)]
            Sample::Signed(v) => Some(*v as f64),
            _ => None,
        };
        if let Some(number) = number {
            if self.numeric == 0 {
                self.min = number;
                self.max = number;
            } else {
                self.min = self.min.min(number);
                self.max = self.max.max(number);
            }
            self.sum += number;
            self.numeric += 1;
            return;
        }
        match sample {
            Sample::Value(PropertyValue::Boolean(v)) => {
                *self.states.entry(v.to_string()).or_default() += 1;
            }
            Sample::Value(PropertyValue::Enumerated(v)) => {
                *self.states.entry(v.to_string()).or_default() += 1;
            }
            Sample::Failure(..) => self.failures += 1,
            _ => self.other += 1,
        }
    }

    fn json(&self, index: usize) -> Value {
        let numeric = (self.numeric > 0).then(|| {
            #[allow(clippy::cast_precision_loss)]
            let mean = self.sum / self.numeric as f64;
            json!({
                "count": self.numeric,
                "min": finite(self.min),
                "max": finite(self.max),
                "mean": finite(mean),
            })
        });
        json!({
            "column": index,
            "numeric": numeric,
            "states": (!self.states.is_empty()).then(|| json!(self.states)),
            "failures": self.failures,
            "other": self.other,
        })
    }
}

/// The summary of the records a call read.
#[derive(Debug, Default)]
pub(super) struct Summary {
    records: u64,
    flagged: u64,
    log_status: u64,
    time_changes: u64,
    first: Option<String>,
    last: Option<String>,
    columns: Vec<Column>,
}

impl Summary {
    pub(super) fn add(&mut self, entry: &Entry) {
        self.records += 1;
        if entry.flagged() {
            self.flagged += 1;
        }
        if self.first.is_none() {
            self.first = Some(entry.time.clone());
        }
        self.last = Some(entry.time.clone());
        match &entry.kind {
            Kind::Values(samples) => {
                if self.columns.len() < samples.len() {
                    self.columns.resize(samples.len(), Column::default());
                }
                for (column, sample) in self.columns.iter_mut().zip(samples) {
                    column.add(sample);
                }
            }
            Kind::LogStatus(_) => self.log_status += 1,
            Kind::TimeChange(_) => self.time_changes += 1,
        }
    }

    pub(super) fn json(&self) -> Value {
        let columns: Vec<Value> = self
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| column.json(index))
            .collect();
        json!({
            "records": self.records,
            "flagged": self.flagged,
            "log_status": self.log_status,
            "time_changes": self.time_changes,
            "first_time": self.first,
            "last_time": self.last,
            "columns": columns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: u8) -> (Date, Time) {
        let date = Date {
            year: 126,
            month: 10,
            day: 25,
            day_of_week: 7,
        };
        let time = Time {
            hour: 7,
            minute,
            second: 0,
            hundredths: 0,
        };
        (date, time)
    }

    /// A Trend Log Multiple summarises each member as a column; log
    /// status records are flagged and counted, failures counted, and a
    /// failure's error kept as data.
    #[test]
    fn multiple_logs_summarise_by_column() {
        let record = |minute, log_data| {
            let (date, time) = at(minute);
            BACnetLogMultipleRecord {
                date,
                time,
                log_data,
            }
        };
        let records = [
            record(
                0,
                LogData::Values(vec![LogValue::RealValue(1.5), LogValue::EnumValue(1)]),
            ),
            record(
                1,
                LogData::Values(vec![
                    LogValue::SignedValue(-4),
                    LogValue::Failure {
                        error_class: ErrorClass::DEVICE,
                        error_code: ErrorCode::OPERATIONAL_PROBLEM,
                    },
                ]),
            ),
            record(2, LogData::LogStatus(LogStatus::LOG_INTERRUPTED)),
            record(3, LogData::TimeChange(2.0)),
        ];
        let mut summary = Summary::default();
        let entries: Vec<Entry> = records.iter().map(Entry::multiple).collect();
        for entry in &entries {
            summary.add(entry);
        }
        let json = summary.json();
        assert_eq!(json["records"], json!(4));
        assert_eq!(json["flagged"], json!(1));
        assert_eq!(json["log_status"], json!(1));
        assert_eq!(json["time_changes"], json!(1));
        assert_eq!(json["first_time"], json!("2026-10-25 07:00:00.00"));
        let first = &json["columns"][0]["numeric"];
        assert_eq!(first["min"], json!(-4.0));
        assert_eq!(first["max"], json!(1.5));
        assert_eq!(first["mean"], json!(-1.25));
        assert_eq!(json["columns"][1]["states"], json!({ "1": 1 }));
        assert_eq!(json["columns"][1]["failures"], json!(1));
        let row = entries[1].json(Some(2), "s", 256);
        assert_eq!(row["values"][0], json!(-4));
        assert_eq!(
            row["values"][1]["error"]["code"],
            json!("operational-problem")
        );
        let status = entries[2].json(None, "s", 256);
        assert_eq!(status["kind"], json!("log-status"));
        assert_eq!(status["flagged"], json!(true));
        assert_eq!(status["log_status"]["names"], json!(["log-interrupted"]));
    }
}
