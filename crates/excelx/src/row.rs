use std::collections::HashMap;

use crate::{CellValue, ColumnDef, ExcelError};

/// Implement this trait to convert a struct to and from XLSX rows.
///
/// Invariants:
///
/// * `columns()` must return unique `field`, `header`, and `order` values.
/// * `to_row()` must return values in the same order as `columns()`.
/// * `from_row()` should use [`RowView`] accessors so errors include row and
///   column context.
pub trait ExcelRow: Sized {
    fn columns() -> Vec<ColumnDef>;
    fn to_row(&self) -> Vec<CellValue>;
    fn from_row(row: &RowView) -> Result<Self, ExcelError>;
}

/// A typed view over one parsed spreadsheet row.
#[derive(Clone, Debug)]
pub struct RowView {
    row_number: usize,
    values_by_field: HashMap<&'static str, CellValue>,
    headers_by_field: HashMap<&'static str, &'static str>,
    defaults_by_field: HashMap<&'static str, &'static str>,
    values_by_header: HashMap<&'static str, CellValue>,
}

enum FieldValue<'a> {
    Cell(&'a CellValue),
    Default(&'static str),
}

impl RowView {
    pub(crate) fn new(
        row_number: usize,
        columns: &[ColumnDef],
        values_by_schema_index: Vec<CellValue>,
    ) -> Self {
        let mut values_by_field = HashMap::with_capacity(columns.len());
        let mut headers_by_field = HashMap::with_capacity(columns.len());
        let mut defaults_by_field = HashMap::with_capacity(columns.len());
        let mut values_by_header = HashMap::with_capacity(columns.len());

        for (column, value) in columns.iter().zip(values_by_schema_index.into_iter()) {
            values_by_field.insert(column.field, value.clone());
            headers_by_field.insert(column.field, column.header);
            if let Some(default) = column.default {
                defaults_by_field.insert(column.field, default);
            }
            values_by_header.insert(column.header, value);
        }

        Self {
            row_number,
            values_by_field,
            headers_by_field,
            defaults_by_field,
            values_by_header,
        }
    }

    pub fn row_number(&self) -> usize {
        self.row_number
    }

    pub fn get(&self, field: &str) -> Option<&CellValue> {
        self.values_by_field.get(field)
    }

    pub fn get_by_header(&self, header: &str) -> Option<&CellValue> {
        self.values_by_header.get(header)
    }

    pub fn header_for_field(&self, field: &str) -> String {
        self.headers_by_field
            .get(field)
            .copied()
            .unwrap_or(field)
            .to_owned()
    }

    pub fn required_string(&self, field: &str) -> Result<String, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => Ok(value.to_owned()),
            FieldValue::Cell(CellValue::String(value)) => Ok(value.clone()),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "string", value)),
        }
    }

    pub fn optional_string(&self, field: &str) -> Result<Option<String>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => Ok(Some(value.to_owned())),
            FieldValue::Cell(CellValue::String(value)) => Ok(Some(value.clone())),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "string or empty", value)),
        }
    }

    pub fn required_i64(&self, field: &str) -> Result<i64, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_i64(field, value),
            FieldValue::Cell(CellValue::Int(value)) => Ok(*value),
            FieldValue::Cell(CellValue::Float(value)) if value.fract() == 0.0 => Ok(*value as i64),
            FieldValue::Cell(CellValue::String(value)) => value.trim().parse().map_err(|_| {
                self.invalid_type(field, "integer", &CellValue::String(value.clone()))
            }),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "integer", value)),
        }
    }

    pub fn optional_i64(&self, field: &str) -> Result<Option<i64>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_i64(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_i64(field).map(Some),
        }
    }

    pub fn required_f64(&self, field: &str) -> Result<f64, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_f64(field, value),
            FieldValue::Cell(CellValue::Int(value)) => Ok(*value as f64),
            FieldValue::Cell(CellValue::Float(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => value
                .trim()
                .parse()
                .map_err(|_| self.invalid_type(field, "number", &CellValue::String(value.clone()))),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "number", value)),
        }
    }

    pub fn optional_f64(&self, field: &str) -> Result<Option<f64>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_f64(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_f64(field).map(Some),
        }
    }

    pub fn required_bool(&self, field: &str) -> Result<bool, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_bool(field, value),
            FieldValue::Cell(CellValue::Bool(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => parse_bool(value).ok_or_else(|| {
                self.invalid_type(field, "boolean", &CellValue::String(value.clone()))
            }),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "boolean", value)),
        }
    }

    pub fn optional_bool(&self, field: &str) -> Result<Option<bool>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_bool(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_bool(field).map(Some),
        }
    }

    #[cfg(feature = "decimal")]
    pub fn required_decimal(&self, field: &str) -> Result<rust_decimal::Decimal, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_decimal(field, value),
            FieldValue::Cell(CellValue::Decimal(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => {
                value.trim().parse().map_err(|_| {
                    self.invalid_type(field, "decimal", &CellValue::String(value.clone()))
                })
            }
            FieldValue::Cell(value) => Err(self.invalid_type(field, "decimal", value)),
        }
    }

    #[cfg(feature = "decimal")]
    pub fn optional_decimal(
        &self,
        field: &str,
    ) -> Result<Option<rust_decimal::Decimal>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_decimal(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_decimal(field).map(Some),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn required_datetime(
        &self,
        field: &str,
    ) -> Result<chrono::NaiveDateTime, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_datetime(field, value),
            FieldValue::Cell(CellValue::DateTime(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => parse_naive_datetime(value)
                .ok_or_else(|| self.invalid_type(field, "datetime", &CellValue::String(value.clone()))),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "datetime", value)),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn optional_datetime(
        &self,
        field: &str,
    ) -> Result<Option<chrono::NaiveDateTime>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_datetime(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_datetime(field).map(Some),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn required_date(&self, field: &str) -> Result<chrono::NaiveDate, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_date(field, value),
            FieldValue::Cell(CellValue::Date(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => {
                chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").map_err(|_| {
                    self.invalid_type(field, "date", &CellValue::String(value.clone()))
                })
            }
            FieldValue::Cell(value) => Err(self.invalid_type(field, "date", value)),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn optional_date(
        &self,
        field: &str,
    ) -> Result<Option<chrono::NaiveDate>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_date(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_date(field).map(Some),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn required_time(&self, field: &str) -> Result<chrono::NaiveTime, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_time(field, value),
            FieldValue::Cell(CellValue::Time(value)) => Ok(*value),
            FieldValue::Cell(CellValue::String(value)) => parse_naive_time(value).ok_or_else(|| {
                self.invalid_type(field, "time", &CellValue::String(value.clone()))
            }),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "time", value)),
        }
    }

    #[cfg(feature = "chrono")]
    pub fn optional_time(
        &self,
        field: &str,
    ) -> Result<Option<chrono::NaiveTime>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Default(value) => self.parse_default_time(field, value).map(Some),
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_time(field).map(Some),
        }
    }

    pub fn required_bytes(&self, field: &str) -> Result<Vec<u8>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Cell(CellValue::Bytes(value)) => Ok(value.clone()),
            FieldValue::Cell(CellValue::String(value)) => parse_bytes(value).ok_or_else(|| {
                self.invalid_type(field, "bytes", &CellValue::String(value.clone()))
            }),
            FieldValue::Default(_) | FieldValue::Cell(CellValue::Empty) => Err(self.invalid_type(
                field,
                "bytes",
                &CellValue::Empty,
            )),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "bytes", value)),
        }
    }

    pub fn optional_bytes(&self, field: &str) -> Result<Option<Vec<u8>>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_bytes(field).map(Some),
        }
    }

    pub fn required_string_list(&self, field: &str) -> Result<Vec<String>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Cell(CellValue::StringList(value)) => Ok(value.clone()),
            FieldValue::Cell(CellValue::String(value)) => Ok(split_string_list(value)),
            FieldValue::Default(value) => Ok(split_string_list(value)),
            FieldValue::Cell(value) => Err(self.invalid_type(field, "string-list", value)),
        }
    }

    pub fn optional_string_list(
        &self,
        field: &str,
    ) -> Result<Option<Vec<String>>, ExcelError> {
        match self.value_or_default(field)? {
            FieldValue::Cell(CellValue::Empty) => Ok(None),
            _ => self.required_string_list(field).map(Some),
        }
    }

    fn value_or_default(&self, field: &str) -> Result<FieldValue<'_>, ExcelError> {
        match self.required_value(field)? {
            CellValue::Empty => Ok(self
                .defaults_by_field
                .get(field)
                .copied()
                .map(FieldValue::Default)
                .unwrap_or(FieldValue::Cell(&CellValue::Empty))),
            value => Ok(FieldValue::Cell(value)),
        }
    }

    fn required_value(&self, field: &str) -> Result<&CellValue, ExcelError> {
        self.values_by_field
            .get(field)
            .ok_or_else(|| ExcelError::Schema(format!("unknown field `{field}` in RowView")))
    }

    fn parse_default_i64(&self, field: &str, value: &str) -> Result<i64, ExcelError> {
        value
            .trim()
            .parse()
            .map_err(|_| self.invalid_default(field, "integer", value))
    }

    fn parse_default_f64(&self, field: &str, value: &str) -> Result<f64, ExcelError> {
        value
            .trim()
            .parse()
            .map_err(|_| self.invalid_default(field, "number", value))
    }

    fn parse_default_bool(&self, field: &str, value: &str) -> Result<bool, ExcelError> {
        parse_bool(value).ok_or_else(|| self.invalid_default(field, "boolean", value))
    }

    #[cfg(feature = "decimal")]
    fn parse_default_decimal(
        &self,
        field: &str,
        value: &str,
    ) -> Result<rust_decimal::Decimal, ExcelError> {
        value
            .trim()
            .parse()
            .map_err(|_| self.invalid_default(field, "decimal", value))
    }

    #[cfg(feature = "chrono")]
    fn parse_default_datetime(
        &self,
        field: &str,
        value: &str,
    ) -> Result<chrono::NaiveDateTime, ExcelError> {
        parse_naive_datetime(value)
            .ok_or_else(|| self.invalid_default(field, "datetime", value))
    }

    #[cfg(feature = "chrono")]
    fn parse_default_date(
        &self,
        field: &str,
        value: &str,
    ) -> Result<chrono::NaiveDate, ExcelError> {
        chrono::NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
            .map_err(|_| self.invalid_default(field, "date", value))
    }

    #[cfg(feature = "chrono")]
    fn parse_default_time(
        &self,
        field: &str,
        value: &str,
    ) -> Result<chrono::NaiveTime, ExcelError> {
        parse_naive_time(value).ok_or_else(|| self.invalid_default(field, "time", value))
    }

    fn invalid_type(&self, field: &str, expected: &str, found: &CellValue) -> ExcelError {
        ExcelError::InvalidCellType {
            row: self.row_number,
            column: self.header_for_field(field),
            expected: expected.to_owned(),
            found: found.type_name().to_owned(),
        }
    }

    fn invalid_default(&self, field: &str, expected: &str, value: &str) -> ExcelError {
        ExcelError::InvalidDefault {
            field: field.to_owned(),
            header: self.header_for_field(field),
            expected: expected.to_owned(),
            value: value.to_owned(),
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(feature = "chrono")]
fn parse_naive_datetime(value: &str) -> Option<chrono::NaiveDateTime> {
    let trimmed = value.trim();
    chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S%.f"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%F %T"))
        .ok()
}

#[cfg(feature = "chrono")]
fn parse_naive_time(value: &str) -> Option<chrono::NaiveTime> {
    let trimmed = value.trim();
    chrono::NaiveTime::parse_from_str(trimmed, "%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveTime::parse_from_str(trimmed, "%H:%M:%S"))
        .or_else(|_| chrono::NaiveTime::parse_from_str(trimmed, "%H:%M"))
        .ok()
}

/// Parse a byte list stringified by [`format_bytes`]. The string must start
/// with `[` and end with `]`. Whitespace around bytes is allowed.
fn parse_bytes(value: &str) -> Option<Vec<u8>> {
    let trimmed = value.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|part| part.trim().parse::<u8>().ok())
        .collect()
}

/// Split a comma-separated string list. Empty entries are dropped, and the
/// members are trimmed.
fn split_string_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_owned())
        .collect()
}
