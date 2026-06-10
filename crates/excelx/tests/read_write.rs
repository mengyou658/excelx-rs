use std::io::Cursor;

use calamine::{Reader, Xlsx};
use excelx::{
    CellValue, ColumnDef, ExcelError, ExcelRow, ReadLimits, RowView, XLSX_MAX_COLUMNS,
    XLSX_MAX_ROWS, from_xlsx, from_xlsx_with_limits, to_xlsx, validate_columns,
};

#[derive(Debug, PartialEq)]
struct Person {
    id: i64,
    name: String,
    active: bool,
    score: f64,
}

impl ExcelRow for Person {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("name", "Name", 2),
            ColumnDef::new("id", "ID", 1),
            ColumnDef::new("active", "Active", 4),
            ColumnDef::new("score", "Score", 3),
        ]
    }

    fn to_row(&self) -> Vec<CellValue> {
        vec![
            self.name.clone().into(),
            self.id.into(),
            self.active.into(),
            self.score.into(),
        ]
    }

    fn from_row(row: &RowView) -> Result<Self, ExcelError> {
        Ok(Self {
            id: row.required_i64("id")?,
            name: row.required_string("name")?,
            active: row.required_bool("active")?,
            score: row.required_f64("score")?,
        })
    }
}

#[test]
fn round_trips_single_sheet() {
    let people = vec![
        Person {
            id: 1,
            name: "Ada".to_owned(),
            active: true,
            score: 98.5,
        },
        Person {
            id: 2,
            name: "Grace".to_owned(),
            active: false,
            score: 88.0,
        },
    ];

    let bytes = to_xlsx(&people).expect("write workbook");
    let parsed = from_xlsx::<Person>(&bytes).expect("parse workbook");

    assert_eq!(parsed, people);
}

#[test]
fn writes_headers_in_column_order() {
    let bytes = to_xlsx(&[Person {
        id: 1,
        name: "Ada".to_owned(),
        active: true,
        score: 98.5,
    }])
    .expect("write workbook");

    let mut workbook: Xlsx<_> = Xlsx::new(Cursor::new(bytes)).expect("open workbook");
    let range = workbook
        .worksheet_range_at(0)
        .expect("first worksheet")
        .expect("read worksheet");
    let headers: Vec<String> = range
        .rows()
        .next()
        .expect("header row")
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(headers, ["ID", "Name", "Score", "Active"]);
}

#[test]
fn parses_reordered_headers() {
    #[derive(Debug, PartialEq)]
    struct Reordered(Person);

    impl ExcelRow for Reordered {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef::new("name", "Name", 1),
                ColumnDef::new("id", "ID", 2),
                ColumnDef::new("score", "Score", 3),
                ColumnDef::new("active", "Active", 4),
            ]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![
                self.0.name.clone().into(),
                self.0.id.into(),
                self.0.score.into(),
                self.0.active.into(),
            ]
        }

        fn from_row(row: &RowView) -> Result<Self, ExcelError> {
            Ok(Self(Person {
                id: row.required_i64("id")?,
                name: row.required_string("name")?,
                active: row.required_bool("active")?,
                score: row.required_f64("score")?,
            }))
        }
    }

    let source = vec![Reordered(Person {
        id: 7,
        name: "Katherine".to_owned(),
        active: true,
        score: 91.25,
    })];
    let bytes = to_xlsx(&source).expect("write workbook");

    let parsed = from_xlsx::<Person>(&bytes).expect("parse with different order");

    assert_eq!(parsed, vec![source.into_iter().next().unwrap().0]);
}

#[test]
fn missing_header_is_deterministic_error() {
    #[derive(Debug)]
    struct MissingHeaderPerson;

    impl ExcelRow for MissingHeaderPerson {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef::new("id", "Identifier", 1),
                ColumnDef::new("name", "Name", 2),
            ]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![CellValue::Int(1), CellValue::String("Ada".to_owned())]
        }

        fn from_row(_: &RowView) -> Result<Self, ExcelError> {
            Ok(Self)
        }
    }

    let bytes = to_xlsx(&[Person {
        id: 1,
        name: "Ada".to_owned(),
        active: true,
        score: 98.5,
    }])
    .expect("write workbook");

    let error = from_xlsx::<MissingHeaderPerson>(&bytes).expect_err("missing header");

    assert!(matches!(error, ExcelError::MissingHeader(header) if header == "Identifier"));
}

#[test]
fn row_view_reports_type_errors_with_context() {
    #[derive(Debug)]
    struct BadType;

    impl ExcelRow for BadType {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef::new("id", "ID", 1),
                ColumnDef::new("name", "Name", 2),
                ColumnDef::new("score", "Score", 3),
                ColumnDef::new("active", "Active", 4),
            ]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![
                CellValue::String("not an int".to_owned()),
                CellValue::String("Ada".to_owned()),
                CellValue::Float(98.5),
                CellValue::Bool(true),
            ]
        }

        fn from_row(row: &RowView) -> Result<Self, ExcelError> {
            row.required_i64("id")?;
            Ok(Self)
        }
    }

    let bytes = to_xlsx(&[BadType]).expect("write workbook");
    let error = from_xlsx::<BadType>(&bytes).expect_err("type error");

    assert!(matches!(
        error,
        ExcelError::InvalidCellType {
            row: 2,
            column,
            expected,
            found,
        } if column == "ID" && expected == "integer" && found == "string"
    ));
}

#[test]
fn ignores_empty_rows() {
    #[derive(Debug, PartialEq)]
    struct OptionalPerson {
        id: Option<i64>,
        name: Option<String>,
    }

    impl ExcelRow for OptionalPerson {
        fn columns() -> Vec<ColumnDef> {
            vec![
                ColumnDef::new("id", "ID", 1),
                ColumnDef::new("name", "Name", 2),
            ]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![
                self.id.map(CellValue::Int).unwrap_or(CellValue::Empty),
                self.name
                    .clone()
                    .map(CellValue::String)
                    .unwrap_or(CellValue::Empty),
            ]
        }

        fn from_row(row: &RowView) -> Result<Self, ExcelError> {
            Ok(Self {
                id: row.optional_i64("id")?,
                name: row.optional_string("name")?,
            })
        }
    }

    let bytes = to_xlsx(&[
        OptionalPerson {
            id: Some(1),
            name: Some("Ada".to_owned()),
        },
        OptionalPerson {
            id: None,
            name: None,
        },
    ])
    .expect("write workbook");

    let parsed = from_xlsx::<OptionalPerson>(&bytes).expect("parse workbook");

    assert_eq!(
        parsed,
        vec![OptionalPerson {
            id: Some(1),
            name: Some("Ada".to_owned())
        }]
    );
}

#[test]
fn validates_duplicate_order_and_headers() {
    let duplicate_order = vec![
        ColumnDef::new("id", "ID", 1),
        ColumnDef::new("name", "Name", 1),
    ];
    let duplicate_header = vec![
        ColumnDef::new("id", "ID", 1),
        ColumnDef::new("other_id", "ID", 2),
    ];

    assert!(matches!(
        validate_columns(&duplicate_order),
        Err(ExcelError::DuplicateColumnOrder(1))
    ));
    assert!(matches!(
        validate_columns(&duplicate_header),
        Err(ExcelError::DuplicateHeader(header)) if header == "ID"
    ));
}

#[test]
fn malformed_workbook_returns_parse_error() {
    let error = from_xlsx::<Person>(b"not an xlsx file").expect_err("malformed workbook");

    assert!(matches!(error, ExcelError::Parse(_)));
}

#[test]
fn empty_workbook_bytes_return_parse_error() {
    let error = from_xlsx::<Person>(&[]).expect_err("empty workbook bytes");

    assert!(matches!(error, ExcelError::Parse(_)));
}

#[test]
fn read_limits_reject_too_many_data_rows() {
    let people = vec![
        Person {
            id: 1,
            name: "Ada".to_owned(),
            active: true,
            score: 98.5,
        },
        Person {
            id: 2,
            name: "Grace".to_owned(),
            active: false,
            score: 88.0,
        },
    ];
    let bytes = to_xlsx(&people).expect("write workbook");

    let error = from_xlsx_with_limits::<Person>(
        &bytes,
        excelx::SheetRef::Index(0),
        ReadLimits::new().max_rows(1),
    )
    .expect_err("row limit");

    assert!(
        matches!(error, ExcelError::LimitExceeded(message) if message.contains("configured max is 1"))
    );
}

#[test]
fn read_limits_reject_too_many_schema_columns() {
    let bytes = to_xlsx(&[Person {
        id: 1,
        name: "Ada".to_owned(),
        active: true,
        score: 98.5,
    }])
    .expect("write workbook");

    let error = from_xlsx_with_limits::<Person>(
        &bytes,
        excelx::SheetRef::Index(0),
        ReadLimits::new().max_columns(3),
    )
    .expect_err("column limit");

    assert!(
        matches!(error, ExcelError::LimitExceeded(message) if message.contains("configured max is 3"))
    );
}

#[test]
fn write_rejects_more_rows_than_xlsx_supports() {
    #[derive(Clone, Copy)]
    struct Tiny;

    impl ExcelRow for Tiny {
        fn columns() -> Vec<ColumnDef> {
            vec![ColumnDef::new("id", "ID", 1)]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![CellValue::Int(1)]
        }

        fn from_row(_: &RowView) -> Result<Self, ExcelError> {
            Ok(Self)
        }
    }

    let rows = vec![Tiny; XLSX_MAX_ROWS];
    let error = to_xlsx(&rows).expect_err("row limit");

    assert!(matches!(error, ExcelError::LimitExceeded(message) if message.contains("data rows")));
}

#[test]
fn write_rejects_more_columns_than_xlsx_supports() {
    struct Wide;

    impl ExcelRow for Wide {
        fn columns() -> Vec<ColumnDef> {
            (0..=XLSX_MAX_COLUMNS)
                .map(|index| {
                    let field: &'static str = Box::leak(format!("field_{index}").into_boxed_str());
                    let header: &'static str =
                        Box::leak(format!("Header {index}").into_boxed_str());
                    ColumnDef::new(field, header, index)
                })
                .collect()
        }

        fn to_row(&self) -> Vec<CellValue> {
            Vec::new()
        }

        fn from_row(_: &RowView) -> Result<Self, ExcelError> {
            Ok(Self)
        }
    }

    let error = to_xlsx(&[Wide]).expect_err("column limit");

    assert!(matches!(error, ExcelError::LimitExceeded(message) if message.contains("columns")));
}
