use calamine::Data;

use crate::ExcelError;

/// A small, explicit value model for the cell types supported by `excelx`.
///
/// The variants cover the MySQL column types most commonly projected to
/// Excel: integers, floats, booleans, decimals, dates and times, binary blobs,
/// and string sets. Custom cell shapes should be flattened to one of these
/// variants (or to [`CellValue::String`]) before being written.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Empty,
    /// Fixed-precision decimal, typically used for MySQL `DECIMAL(M, D)`.
    /// Available with the `decimal` feature.
    #[cfg(feature = "decimal")]
    Decimal(rust_decimal::Decimal),
    /// Naive date-time, typically used for MySQL `DATETIME` / `TIMESTAMP`.
    /// Available with the `chrono` feature.
    #[cfg(feature = "chrono")]
    DateTime(chrono::NaiveDateTime),
    /// Naive date (no time component), typically used for MySQL `DATE`.
    /// Available with the `chrono` feature.
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    /// Naive time (no date component), typically used for MySQL `TIME`.
    /// Available with the `chrono` feature.
    #[cfg(feature = "chrono")]
    Time(chrono::NaiveTime),
    /// Raw bytes, typically used for MySQL `BLOB` / `BINARY`.
    Bytes(Vec<u8>),
    /// List of strings, typically used for MySQL `SET`.
    StringList(Vec<String>),
}

impl CellValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Int(_) => "integer",
            Self::Float(_) => "float",
            Self::Bool(_) => "boolean",
            Self::Empty => "empty",
            #[cfg(feature = "decimal")]
            Self::Decimal(_) => "decimal",
            #[cfg(feature = "chrono")]
            Self::DateTime(_) => "datetime",
            #[cfg(feature = "chrono")]
            Self::Date(_) => "date",
            #[cfg(feature = "chrono")]
            Self::Time(_) => "time",
            Self::Bytes(_) => "bytes",
            Self::StringList(_) => "string-list",
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}

impl From<&str> for CellValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for CellValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<i64> for CellValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<i32> for CellValue {
    fn from(value: i32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<u32> for CellValue {
    fn from(value: u32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<f64> for CellValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<f32> for CellValue {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

impl From<bool> for CellValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Decimal> for CellValue {
    fn from(value: rust_decimal::Decimal) -> Self {
        Self::Decimal(value)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for CellValue {
    fn from(value: chrono::NaiveDateTime) -> Self {
        Self::DateTime(value)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDate> for CellValue {
    fn from(value: chrono::NaiveDate) -> Self {
        Self::Date(value)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveTime> for CellValue {
    fn from(value: chrono::NaiveTime) -> Self {
        Self::Time(value)
    }
}

impl From<Vec<u8>> for CellValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<Vec<String>> for CellValue {
    fn from(value: Vec<String>) -> Self {
        Self::StringList(value)
    }
}

impl TryFrom<&Data> for CellValue {
    type Error = ExcelError;

    fn try_from(value: &Data) -> Result<Self, Self::Error> {
        match value {
            Data::String(value) => Ok(Self::String(value.clone())),
            Data::Int(value) => Ok(Self::Int(*value)),
            Data::Float(value) => Ok(Self::Float(*value)),
            Data::Bool(value) => Ok(Self::Bool(*value)),
            Data::Empty => Ok(Self::Empty),
            #[cfg(feature = "chrono")]
            Data::DateTime(dt) => excel_datetime_to_naive_datetime(dt)
                .map(Self::DateTime)
                .ok_or_else(|| {
                    ExcelError::Parse(format!(
                        "unsupported Excel date/time cell value: {}",
                        dt.as_f64()
                    ))
                }),
            #[cfg(feature = "chrono")]
            Data::DateTimeIso(value) => chrono::NaiveDateTime::parse_from_str(value, "%FT%T%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%FT%T"))
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%F %T"))
                .map(Self::DateTime)
                .map_err(|err| {
                    ExcelError::Parse(format!(
                        "unsupported ISO date/time cell value `{value}`: {err}"
                    ))
                }),
            #[cfg(not(feature = "chrono"))]
            Data::DateTime(dt) => Err(ExcelError::Parse(format!(
                "Excel date/time cell value {} requires the `chrono` feature on excelx",
                dt.as_f64()
            ))),
            #[cfg(not(feature = "chrono"))]
            Data::DateTimeIso(value) => Err(ExcelError::Parse(format!(
                "Excel ISO date/time cell value `{value}` requires the `chrono` feature on excelx"
            ))),
            Data::DurationIso(value) => Err(ExcelError::Parse(format!(
                "unsupported ISO duration cell value: {value}"
            ))),
            Data::Error(value) => Err(ExcelError::Parse(format!(
                "cell contains Excel error: {value}"
            ))),
        }
    }
}

/// Convert a calamine `ExcelDateTime` value to a `NaiveDateTime` in UTC.
///
/// Supports the standard 1900-based epoch and the legacy 1904-based epoch
/// (used by macOS-originated workbooks). Returns `None` if the resulting
/// `NaiveDateTime` is outside the representable range.
#[cfg(feature = "chrono")]
fn excel_datetime_to_naive_datetime(dt: &calamine::ExcelDateTime) -> Option<chrono::NaiveDateTime> {
    if dt.is_duration() {
        return None;
    }

    let (year, month, day, hour, minute, second, millisecond) = dt.to_ymd_hms_milli();
    let date = chrono::NaiveDate::from_ymd_opt(
        i32::from(year),
        u32::from(month),
        u32::from(day),
    )?;
    let time = chrono::NaiveTime::from_hms_milli_opt(
        u32::from(hour),
        u32::from(minute),
        u32::from(second),
        u32::from(millisecond),
    )?;
    date.and_hms_milli_opt(
        u32::from(hour),
        u32::from(minute),
        u32::from(second),
        u32::from(millisecond),
    )
    .or_else(|| Some(chrono::NaiveDateTime::new(date, time)))
}
