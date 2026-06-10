use std::collections::HashSet;

use rust_xlsxwriter::{Workbook, Worksheet};

use crate::{
    CellValue, ExcelError, ExcelRow, SheetData, XLSX_MAX_COLUMNS, XLSX_MAX_ROWS, validate_columns,
};

/// Convert a slice of row values into a single-sheet XLSX workbook.
pub fn to_xlsx<T: ExcelRow>(data: &[T]) -> Result<Vec<u8>, ExcelError> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    write_rows_to_worksheet(worksheet, data)?;

    workbook.save_to_buffer().map_err(ExcelError::from)
}

/// Convert multiple homogeneous sheets into one XLSX workbook.
pub fn to_xlsx_multi<T: ExcelRow>(sheets: &[SheetData<T>]) -> Result<Vec<u8>, ExcelError> {
    if sheets.is_empty() {
        return Err(ExcelError::Schema(
            "multi-sheet workbook requires at least one sheet".to_owned(),
        ));
    }

    validate_sheet_names(sheets)?;

    let mut workbook = Workbook::new();
    for sheet in sheets {
        let worksheet = workbook.add_worksheet().set_name(&sheet.name)?;
        write_rows_to_worksheet(worksheet, &sheet.rows)?;
    }

    workbook.save_to_buffer().map_err(ExcelError::from)
}

fn write_rows_to_worksheet<T: ExcelRow>(
    worksheet: &mut Worksheet,
    data: &[T],
) -> Result<(), ExcelError> {
    let columns = T::columns();
    let sorted_columns = validate_columns(&columns)?;
    validate_worksheet_size(data.len(), sorted_columns.len())?;

    for (write_col, column) in sorted_columns.iter().enumerate() {
        worksheet.write_string(0, as_col(write_col)?, column.header)?;
    }

    for (row_index, item) in data.iter().enumerate() {
        let values = item.to_row();
        if values.len() != columns.len() {
            return Err(ExcelError::Schema(format!(
                "to_row returned {} values but schema defines {} columns",
                values.len(),
                columns.len()
            )));
        }

        let excel_row = as_row(row_index + 1)?;
        for (write_col, column) in sorted_columns.iter().enumerate() {
            let schema_index = columns
                .iter()
                .position(|candidate| candidate.field == column.field)
                .ok_or_else(|| {
                    ExcelError::Schema(format!("schema column `{}` disappeared", column.field))
                })?;
            write_cell(
                worksheet,
                excel_row,
                as_col(write_col)?,
                &values[schema_index],
            )?;
        }
    }

    Ok(())
}

fn validate_worksheet_size(row_count: usize, column_count: usize) -> Result<(), ExcelError> {
    let max_data_rows = XLSX_MAX_ROWS - 1;
    if row_count > max_data_rows {
        return Err(ExcelError::LimitExceeded(format!(
            "worksheet has {row_count} data rows but XLSX supports at most {max_data_rows}"
        )));
    }

    if column_count > XLSX_MAX_COLUMNS {
        return Err(ExcelError::LimitExceeded(format!(
            "worksheet has {column_count} columns but XLSX supports at most {XLSX_MAX_COLUMNS}"
        )));
    }

    Ok(())
}

fn validate_sheet_names<T>(sheets: &[SheetData<T>]) -> Result<(), ExcelError> {
    let mut names = HashSet::with_capacity(sheets.len());

    for sheet in sheets {
        let normalized = sheet.name.to_lowercase();
        if !names.insert(normalized) {
            return Err(ExcelError::Schema(format!(
                "duplicate sheet name: {}",
                sheet.name
            )));
        }
    }

    Ok(())
}

fn write_cell(
    worksheet: &mut Worksheet,
    row: u32,
    col: u16,
    value: &CellValue,
) -> Result<(), ExcelError> {
    match value {
        CellValue::String(value) => {
            worksheet.write_string(row, col, value)?;
        }
        CellValue::Int(value) => {
            worksheet.write_number(row, col, *value as f64)?;
        }
        CellValue::Float(value) => {
            worksheet.write_number(row, col, *value)?;
        }
        CellValue::Bool(value) => {
            worksheet.write_boolean(row, col, *value)?;
        }
        CellValue::Empty => {}
    }

    Ok(())
}

fn as_row(value: usize) -> Result<u32, ExcelError> {
    if value >= XLSX_MAX_ROWS {
        return Err(ExcelError::Write(format!(
            "row index {value} exceeds XLSX limits"
        )));
    }

    u32::try_from(value)
        .map_err(|_| ExcelError::Write(format!("row index {value} exceeds XLSX limits")))
}

fn as_col(value: usize) -> Result<u16, ExcelError> {
    if value >= XLSX_MAX_COLUMNS {
        return Err(ExcelError::Write(format!(
            "column index {value} exceeds XLSX limits"
        )));
    }

    u16::try_from(value)
        .map_err(|_| ExcelError::Write(format!("column index {value} exceeds XLSX limits")))
}
