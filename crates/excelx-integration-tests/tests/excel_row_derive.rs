use excelx::{CellValue, ColumnDef, ExcelError, ExcelRow, RowView, from_xlsx, to_xlsx};

#[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
struct DerivedPerson {
    #[excel(header = "ID", order = 1)]
    id: i64,
    #[excel(header = "Name", order = 2)]
    name: String,
    #[excel(header = "Score", order = 3, default = "0.5")]
    score: f64,
    #[excel(header = "Active", order = 4)]
    active: bool,
    #[excel(header = "Nickname", order = 5, default = "N/A")]
    nickname: Option<String>,
}

#[test]
fn derive_round_trips_supported_fields() {
    let rows = vec![DerivedPerson {
        id: 1,
        name: "Ada".to_owned(),
        score: 98.5,
        active: true,
        nickname: Some("Countess".to_owned()),
    }];

    let bytes = to_xlsx(&rows).expect("write workbook");
    let parsed = from_xlsx::<DerivedPerson>(&bytes).expect("parse workbook");

    assert_eq!(parsed, rows);
}

#[test]
fn derive_preserves_column_metadata() {
    assert_eq!(
        DerivedPerson::columns(),
        vec![
            ColumnDef::new("id", "ID", 1),
            ColumnDef::new("name", "Name", 2),
            ColumnDef::with_default("score", "Score", 3, "0.5"),
            ColumnDef::new("active", "Active", 4),
            ColumnDef::with_default("nickname", "Nickname", 5, "N/A"),
        ]
    );
}

#[test]
fn derive_applies_defaults_for_empty_cells() {
    #[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
    struct Defaults {
        #[excel(header = "ID", order = 1)]
        id: i64,
        #[excel(header = "Score", order = 2, default = "1.25")]
        score: Option<f64>,
        #[excel(header = "Nickname", order = 3, default = "N/A")]
        nickname: Option<String>,
    }

    impl Defaults {
        fn empty_defaults(id: i64) -> Self {
            Self {
                id,
                score: None,
                nickname: None,
            }
        }
    }

    let bytes = to_xlsx(&[Defaults::empty_defaults(1)]).expect("write workbook");
    let parsed = from_xlsx::<Defaults>(&bytes).expect("parse workbook");

    assert_eq!(
        parsed,
        vec![Defaults {
            id: 1,
            score: Some(1.25),
            nickname: Some("N/A".to_owned()),
        }]
    );
}

#[test]
fn derive_integer_range_errors_use_visible_header() {
    #[derive(Debug)]
    struct Source;

    impl ExcelRow for Source {
        fn columns() -> Vec<ColumnDef> {
            vec![ColumnDef::new("id", "Small ID", 1)]
        }

        fn to_row(&self) -> Vec<CellValue> {
            vec![CellValue::Int(300)]
        }

        fn from_row(_: &RowView) -> Result<Self, ExcelError> {
            Ok(Self)
        }
    }

    #[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
    struct Target {
        #[excel(header = "Small ID", order = 1)]
        id: u8,
    }

    let bytes = to_xlsx(&[Source]).expect("write workbook");
    let error = from_xlsx::<Target>(&bytes).expect_err("range error");

    assert!(matches!(
        error,
        ExcelError::InvalidCellType {
            row: 2,
            column,
            expected,
            found,
        } if column == "Small ID" && expected == "u8" && found == "integer out of range"
    ));
}
