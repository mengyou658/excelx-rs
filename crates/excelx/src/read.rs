use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};

use calamine::{Data, Range, Reader, Xlsx};

use crate::{CellValue, ColumnDef, ExcelError, ExcelRow, RowView, validate_columns};

/// Maximum number of rows in an XLSX worksheet, including the header row.
pub const XLSX_MAX_ROWS: usize = 1_048_576;

/// Maximum number of columns in an XLSX worksheet.
pub const XLSX_MAX_COLUMNS: usize = 16_384;

/// Selects a worksheet by workbook order or visible worksheet name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SheetRef<'a> {
    Index(usize),
    Name(&'a str),
}

/// Options for parsing an XLSX workbook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadOptions<'a> {
    pub sheet: SheetRef<'a>,
}

impl<'a> Default for ReadOptions<'a> {
    fn default() -> Self {
        Self {
            sheet: SheetRef::Index(0),
        }
    }
}

/// Optional safety limits for parsing untrusted or user-uploaded workbooks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReadLimits {
    /// Maximum number of non-empty data rows to parse per worksheet.
    pub max_rows: Option<usize>,
    /// Maximum number of schema columns accepted for the row type.
    pub max_columns: Option<usize>,
}

impl ReadLimits {
    pub const fn new() -> Self {
        Self {
            max_rows: None,
            max_columns: None,
        }
    }

    pub const fn max_rows(mut self, max_rows: usize) -> Self {
        self.max_rows = Some(max_rows);
        self
    }

    pub const fn max_columns(mut self, max_columns: usize) -> Self {
        self.max_columns = Some(max_columns);
        self
    }
}

/// Optional safety limits for parsing homogeneous multi-sheet workbooks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MultiSheetReadLimits {
    /// Maximum number of worksheets to parse.
    pub max_sheets: Option<usize>,
    /// Maximum number of non-empty data rows to parse per worksheet.
    pub max_rows_per_sheet: Option<usize>,
    /// Maximum number of schema columns accepted for the row type.
    pub max_columns_per_sheet: Option<usize>,
}

impl MultiSheetReadLimits {
    pub const fn new() -> Self {
        Self {
            max_sheets: None,
            max_rows_per_sheet: None,
            max_columns_per_sheet: None,
        }
    }

    pub const fn max_sheets(mut self, max_sheets: usize) -> Self {
        self.max_sheets = Some(max_sheets);
        self
    }

    pub const fn max_rows_per_sheet(mut self, max_rows_per_sheet: usize) -> Self {
        self.max_rows_per_sheet = Some(max_rows_per_sheet);
        self
    }

    pub const fn max_columns_per_sheet(mut self, max_columns_per_sheet: usize) -> Self {
        self.max_columns_per_sheet = Some(max_columns_per_sheet);
        self
    }
}

/// Parsed rows for one worksheet in a homogeneous multi-sheet workbook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedSheet<T> {
    pub name: String,
    pub rows: Vec<T>,
}

impl<T> ParsedSheet<T> {
    pub fn new(name: impl Into<String>, rows: Vec<T>) -> Self {
        Self {
            name: name.into(),
            rows,
        }
    }
}

/// Parse the first worksheet in an XLSX byte slice into typed rows.
pub fn from_xlsx<T: ExcelRow>(bytes: &[u8]) -> Result<Vec<T>, ExcelError> {
    from_reader(Cursor::new(bytes))
}

/// Parse the first worksheet in an XLSX reader into typed rows.
pub fn from_reader<T: ExcelRow, R: Read + Seek>(reader: R) -> Result<Vec<T>, ExcelError> {
    from_reader_with_options(reader, ReadOptions::default())
}

/// Parse one selected worksheet in an XLSX byte slice into typed rows.
pub fn from_xlsx_sheet<T: ExcelRow>(
    bytes: &[u8],
    sheet: SheetRef<'_>,
) -> Result<Vec<T>, ExcelError> {
    from_xlsx_with_options(bytes, ReadOptions { sheet })
}

/// Parse one selected worksheet in an XLSX byte slice with explicit safety limits.
pub fn from_xlsx_with_limits<T: ExcelRow>(
    bytes: &[u8],
    sheet: SheetRef<'_>,
    limits: ReadLimits,
) -> Result<Vec<T>, ExcelError> {
    from_reader_with_limits(Cursor::new(bytes), sheet, limits)
}

/// Parse one selected worksheet in an XLSX byte slice with explicit options.
pub fn from_xlsx_with_options<T: ExcelRow>(
    bytes: &[u8],
    options: ReadOptions<'_>,
) -> Result<Vec<T>, ExcelError> {
    from_reader_with_options(Cursor::new(bytes), options)
}

/// Parse one selected worksheet in an XLSX reader with explicit options.
pub fn from_reader_with_options<T: ExcelRow, R: Read + Seek>(
    reader: R,
    options: ReadOptions<'_>,
) -> Result<Vec<T>, ExcelError> {
    from_reader_with_limits(reader, options.sheet, ReadLimits::default())
}

/// Parse one selected worksheet in an XLSX reader with explicit safety limits.
pub fn from_reader_with_limits<T: ExcelRow, R: Read + Seek>(
    reader: R,
    sheet: SheetRef<'_>,
    limits: ReadLimits,
) -> Result<Vec<T>, ExcelError> {
    let columns = T::columns();
    let sorted_columns = validate_columns(&columns)?;
    validate_column_limit("schema", sorted_columns.len(), limits.max_columns)?;
    let mut workbook: Xlsx<R> = Xlsx::new(reader)?;
    let range = worksheet_range(&mut workbook, sheet)?;

    parse_range(range, &columns, &sorted_columns, limits)
}

/// Parse all worksheets in an XLSX byte slice as the same row type.
pub fn from_xlsx_multi<T: ExcelRow>(bytes: &[u8]) -> Result<Vec<ParsedSheet<T>>, ExcelError> {
    from_reader_multi(Cursor::new(bytes))
}

/// Parse all worksheets in an XLSX byte slice with explicit safety limits.
pub fn from_xlsx_multi_with_limits<T: ExcelRow>(
    bytes: &[u8],
    limits: MultiSheetReadLimits,
) -> Result<Vec<ParsedSheet<T>>, ExcelError> {
    from_reader_multi_with_limits(Cursor::new(bytes), limits)
}

/// Parse all worksheets in an XLSX reader as the same row type.
pub fn from_reader_multi<T: ExcelRow, R: Read + Seek>(
    reader: R,
) -> Result<Vec<ParsedSheet<T>>, ExcelError> {
    from_reader_multi_with_limits(reader, MultiSheetReadLimits::default())
}

/// Parse all worksheets in an XLSX reader with explicit safety limits.
pub fn from_reader_multi_with_limits<T: ExcelRow, R: Read + Seek>(
    reader: R,
    limits: MultiSheetReadLimits,
) -> Result<Vec<ParsedSheet<T>>, ExcelError> {
    let columns = T::columns();
    let sorted_columns = validate_columns(&columns)?;
    validate_column_limit("schema", sorted_columns.len(), limits.max_columns_per_sheet)?;
    let mut workbook: Xlsx<R> = Xlsx::new(reader)?;
    let sheet_names = workbook.sheet_names();

    if sheet_names.is_empty() {
        return Err(ExcelError::Parse(
            "workbook does not contain any worksheets".to_owned(),
        ));
    }
    validate_sheet_limit(sheet_names.len(), limits.max_sheets)?;

    let mut parsed_sheets = Vec::with_capacity(sheet_names.len());
    for name in sheet_names {
        let range = workbook.worksheet_range(&name)?;
        let rows = parse_range(
            range,
            &columns,
            &sorted_columns,
            ReadLimits {
                max_rows: limits.max_rows_per_sheet,
                max_columns: limits.max_columns_per_sheet,
            },
        )?;
        parsed_sheets.push(ParsedSheet::new(name, rows));
    }

    Ok(parsed_sheets)
}

fn parse_range<T: ExcelRow>(
    range: Range<Data>,
    columns: &[ColumnDef],
    sorted_columns: &[ColumnDef],
    limits: ReadLimits,
) -> Result<Vec<T>, ExcelError> {
    let mut rows = range.rows();
    let header_row = rows
        .next()
        .ok_or_else(|| ExcelError::Parse("worksheet does not contain a header row".to_owned()))?;
    let header_map = build_header_map(header_row)?;
    validate_column_limit("worksheet", header_map.len(), limits.max_columns)?;
    ensure_required_headers(sorted_columns, &header_map)?;

    let mut parsed_rows = Vec::new();
    for (relative_index, row) in rows.enumerate() {
        if is_empty_row(row) {
            continue;
        }

        let excel_row_number = relative_index + 2;
        validate_row_limit(parsed_rows.len() + 1, limits.max_rows)?;
        let values = values_for_schema(row, columns, &header_map)?;
        let row_view = RowView::new(excel_row_number, columns, values);
        parsed_rows.push(T::from_row(&row_view)?);
    }

    Ok(parsed_rows)
}

fn worksheet_range<R: Read + Seek>(
    workbook: &mut Xlsx<R>,
    sheet: SheetRef<'_>,
) -> Result<Range<Data>, ExcelError> {
    match sheet {
        SheetRef::Index(index) => workbook
            .worksheet_range_at(index)
            .ok_or_else(|| missing_sheet_by_index(index))?
            .map_err(ExcelError::from),
        SheetRef::Name(name) => {
            if !workbook
                .sheet_names()
                .iter()
                .any(|sheet_name| sheet_name == name)
            {
                return Err(missing_sheet_by_name(name));
            }

            workbook.worksheet_range(name).map_err(ExcelError::from)
        }
    }
}

fn missing_sheet_by_index(index: usize) -> ExcelError {
    if index == 0 {
        ExcelError::Parse("workbook does not contain any worksheets".to_owned())
    } else {
        ExcelError::Parse(format!("worksheet index {index} does not exist"))
    }
}

fn missing_sheet_by_name(name: &str) -> ExcelError {
    ExcelError::Parse(format!("worksheet named `{name}` does not exist"))
}

fn validate_column_limit(
    label: &str,
    count: usize,
    limit: Option<usize>,
) -> Result<(), ExcelError> {
    if count > XLSX_MAX_COLUMNS {
        return Err(ExcelError::LimitExceeded(format!(
            "{label} defines {count} columns but XLSX supports at most {XLSX_MAX_COLUMNS}"
        )));
    }

    if let Some(limit) = limit {
        if count > limit {
            return Err(ExcelError::LimitExceeded(format!(
                "{label} defines {count} columns but configured max is {limit}"
            )));
        }
    }

    Ok(())
}

fn validate_row_limit(count: usize, limit: Option<usize>) -> Result<(), ExcelError> {
    let max_data_rows = XLSX_MAX_ROWS - 1;
    if count > max_data_rows {
        return Err(ExcelError::LimitExceeded(format!(
            "worksheet contains {count} data rows but XLSX supports at most {max_data_rows}"
        )));
    }

    if let Some(limit) = limit {
        if count > limit {
            return Err(ExcelError::LimitExceeded(format!(
                "worksheet contains {count} data rows but configured max is {limit}"
            )));
        }
    }

    Ok(())
}

fn validate_sheet_limit(count: usize, limit: Option<usize>) -> Result<(), ExcelError> {
    if let Some(limit) = limit {
        if count > limit {
            return Err(ExcelError::LimitExceeded(format!(
                "workbook contains {count} worksheets but configured max is {limit}"
            )));
        }
    }

    Ok(())
}

fn build_header_map(row: &[Data]) -> Result<HashMap<String, usize>, ExcelError> {
    let mut headers = HashMap::with_capacity(row.len());

    for (index, cell) in row.iter().enumerate() {
        if matches!(cell, Data::Empty) {
            continue;
        }

        let header = match cell {
            Data::String(value) => value.trim().to_owned(),
            Data::Int(value) => value.to_string(),
            Data::Float(value) => value.to_string(),
            Data::Bool(value) => value.to_string(),
            other => {
                return Err(ExcelError::Parse(format!(
                    "unsupported header cell type at column {}: {other}",
                    index + 1
                )));
            }
        };

        if header.is_empty() {
            continue;
        }

        if headers.insert(header.clone(), index).is_some() {
            return Err(ExcelError::DuplicateHeader(header));
        }
    }

    Ok(headers)
}

fn ensure_required_headers(
    columns: &[ColumnDef],
    header_map: &HashMap<String, usize>,
) -> Result<(), ExcelError> {
    for column in columns {
        if !header_map.contains_key(column.header) {
            return Err(ExcelError::MissingHeader(column.header.to_owned()));
        }
    }

    Ok(())
}

fn values_for_schema(
    row: &[Data],
    columns: &[ColumnDef],
    header_map: &HashMap<String, usize>,
) -> Result<Vec<CellValue>, ExcelError> {
    columns
        .iter()
        .map(|column| {
            let index = header_map
                .get(column.header)
                .ok_or_else(|| ExcelError::MissingHeader(column.header.to_owned()))?;

            row.get(*index)
                .map(CellValue::try_from)
                .unwrap_or(Ok(CellValue::Empty))
        })
        .collect()
}

fn is_empty_row(row: &[Data]) -> bool {
    row.iter().all(|cell| matches!(cell, Data::Empty))
}
