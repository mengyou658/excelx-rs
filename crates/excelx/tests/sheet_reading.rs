use std::io::Cursor;

use excelx::{
    CellValue, ColumnDef, ExcelError, ExcelRow, MultiSheetReadLimits, ParsedSheet, ReadOptions,
    RowView, SheetData, SheetRef, from_reader_with_options, from_xlsx_multi,
    from_xlsx_multi_with_limits, from_xlsx_sheet, from_xlsx_with_options, to_xlsx_multi,
};

#[derive(Debug, PartialEq)]
struct Person {
    id: i64,
    name: String,
}

impl Person {
    fn new(id: i64, name: &str) -> Self {
        Self {
            id,
            name: name.to_owned(),
        }
    }
}

impl ExcelRow for Person {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("id", "ID", 1),
            ColumnDef::new("name", "Name", 2),
        ]
    }

    fn to_row(&self) -> Vec<CellValue> {
        vec![self.id.into(), self.name.clone().into()]
    }

    fn from_row(row: &RowView) -> Result<Self, ExcelError> {
        Ok(Self {
            id: row.required_i64("id")?,
            name: row.required_string("name")?,
        })
    }
}

fn multi_sheet_workbook() -> Vec<u8> {
    to_xlsx_multi(&[
        SheetData::new("Active", vec![Person::new(1, "Ada")]),
        SheetData::new("Archive", vec![Person::new(2, "Grace")]),
        SheetData::new("Review", vec![Person::new(3, "Katherine")]),
    ])
    .expect("write workbook")
}

#[test]
fn reads_selected_sheet_by_index() {
    let bytes = multi_sheet_workbook();
    let parsed = from_xlsx_sheet::<Person>(&bytes, SheetRef::Index(1)).expect("parse sheet");

    assert_eq!(parsed, vec![Person::new(2, "Grace")]);
}

#[test]
fn reads_selected_sheet_by_name() {
    let bytes = multi_sheet_workbook();
    let parsed = from_xlsx_sheet::<Person>(&bytes, SheetRef::Name("Review")).expect("parse sheet");

    assert_eq!(parsed, vec![Person::new(3, "Katherine")]);
}

#[test]
fn reads_selected_sheet_with_options() {
    let bytes = multi_sheet_workbook();
    let options = ReadOptions {
        sheet: SheetRef::Name("Archive"),
    };
    let parsed = from_xlsx_with_options::<Person>(&bytes, options).expect("parse sheet");

    assert_eq!(parsed, vec![Person::new(2, "Grace")]);
}

#[test]
fn reads_selected_sheet_from_reader_with_options() {
    let bytes = multi_sheet_workbook();
    let options = ReadOptions {
        sheet: SheetRef::Index(2),
    };
    let parsed =
        from_reader_with_options::<Person, _>(Cursor::new(bytes), options).expect("parse reader");

    assert_eq!(parsed, vec![Person::new(3, "Katherine")]);
}

#[test]
fn reads_all_sheets_as_same_row_type() {
    let bytes = multi_sheet_workbook();
    let parsed = from_xlsx_multi::<Person>(&bytes).expect("parse workbook");

    assert_eq!(
        parsed,
        vec![
            ParsedSheet::new("Active", vec![Person::new(1, "Ada")]),
            ParsedSheet::new("Archive", vec![Person::new(2, "Grace")]),
            ParsedSheet::new("Review", vec![Person::new(3, "Katherine")]),
        ]
    );
}

#[test]
fn multi_sheet_limits_reject_too_many_sheets() {
    let bytes = multi_sheet_workbook();
    let error =
        from_xlsx_multi_with_limits::<Person>(&bytes, MultiSheetReadLimits::new().max_sheets(2))
            .expect_err("sheet limit");

    assert!(
        matches!(error, ExcelError::LimitExceeded(message) if message.contains("configured max is 2"))
    );
}

#[test]
fn multi_sheet_limits_reject_too_many_rows_per_sheet() {
    let bytes = multi_sheet_workbook();
    let error = from_xlsx_multi_with_limits::<Person>(
        &bytes,
        MultiSheetReadLimits::new().max_rows_per_sheet(0),
    )
    .expect_err("row limit");

    assert!(
        matches!(error, ExcelError::LimitExceeded(message) if message.contains("configured max is 0"))
    );
}

#[test]
fn missing_sheet_name_returns_parse_error() {
    let bytes = multi_sheet_workbook();
    let error =
        from_xlsx_sheet::<Person>(&bytes, SheetRef::Name("Missing")).expect_err("missing sheet");

    assert!(
        matches!(error, ExcelError::Parse(message) if message.contains("worksheet named `Missing`"))
    );
}

#[test]
fn missing_sheet_index_returns_parse_error() {
    let bytes = multi_sheet_workbook();
    let error = from_xlsx_sheet::<Person>(&bytes, SheetRef::Index(99)).expect_err("missing sheet");

    assert!(matches!(error, ExcelError::Parse(message) if message.contains("worksheet index 99")));
}
