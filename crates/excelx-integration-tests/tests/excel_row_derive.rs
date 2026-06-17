use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use excelx::{CellValue, ColumnDef, ExcelError, ExcelRow, RowView, from_xlsx, to_xlsx};
use rust_decimal::Decimal;

/// Minimal single-threaded executor used to drive the macro-generated async
/// helper in tests without pulling in an external runtime.
fn block_on<F: Future>(fut: F) -> F::Output {
    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    let waker: Waker = Arc::new(NoopWake).into();
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        if let Poll::Ready(result) = fut.as_mut().poll(&mut cx) {
            return result;
        }
    }
}

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

#[derive(Debug, PartialEq, Clone)]
struct MemberUser {
    name: String,
    email: String,
}

#[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
struct WithNested {
    #[excel(header = "ID", order = 1)]
    id: i64,
    #[excel(header = "User Name", order = 2, value = "user.name")]
    user: Option<MemberUser>,
    #[excel(header = "User Email", order = 3, value = "fallback_email.email")]
    fallback_email: Option<MemberUser>,
    // `value_parse_fn` overrides `value`: the `value` attribute is ignored and
    // the cell is left empty. The user-defined struct `MemberUser` is treated
    // as `Default` (None) for `from_row` because the derive macro does not
    // recognize it as a known cell type. The displayable label is produced by
    // the generated `value_parse_fn_async_user` helper.
    #[excel(
        header = "Async Label",
        order = 4,
        value_parse_fn = "get_dict_label(\"user_nick\", async_user, self)"
    )]
    async_user: Option<MemberUser>,
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

/// Mirrors `create::common::excel::DictUtil::get_dict_label` from the user
/// request. The macro-expanded method forwards the `user` field name as the
/// `value` parameter and `self` as the entity reference.
async fn get_dict_label(
    dict_type: &str,
    value: &Option<MemberUser>,
    entity: &WithNested,
) -> String {
    let _ = (dict_type, entity);
    match value {
        Some(user) => format!("label:{}", user.name),
        None => String::from("label:-"),
    }
}

#[test]
fn derive_value_attribute_reads_deep_field() {
    let row = WithNested {
        id: 7,
        user: Some(MemberUser {
            name: "Ada".to_owned(),
            email: "ada@example.com".to_owned(),
        }),
        fallback_email: Some(MemberUser {
            name: "Grace".to_owned(),
            email: "grace@example.com".to_owned(),
        }),
        async_user: None,
    };

    let cells = row.to_row();
    assert_eq!(cells[0], CellValue::Int(7));
    assert_eq!(cells[1], CellValue::String("Ada".to_owned()));
    assert_eq!(cells[2], CellValue::String("grace@example.com".to_owned()));
    // async_user is None, so the deep field expression is never reached
    assert_eq!(cells[3], CellValue::Empty);
}

#[test]
fn derive_value_attribute_handles_none() {
    let row = WithNested {
        id: 9,
        user: None,
        fallback_email: None,
        async_user: None,
    };

    let cells = row.to_row();
    assert_eq!(cells[0], CellValue::Int(9));
    assert_eq!(cells[1], CellValue::Empty);
    assert_eq!(cells[2], CellValue::Empty);
    assert_eq!(cells[3], CellValue::Empty);
}

#[test]
fn derive_value_parse_fn_generates_async_helper() {
    let row = WithNested {
        id: 11,
        user: None,
        fallback_email: None,
        async_user: Some(MemberUser {
            name: "Linus".to_owned(),
            email: "linus@example.com".to_owned(),
        }),
    };

    let label = block_on(row.value_parse_fn_async_user(&row.async_user));
    assert_eq!(label, "label:Linus");

    let label_none = block_on(row.value_parse_fn_async_user(&None));
    assert_eq!(label_none, "label:-");
}

#[test]
fn derive_value_parse_fn_overrides_value_attribute() {
    // When `value_parse_fn` is defined the deep field `value` path is
    // ignored: the cell is `Empty` regardless of whether the wrapped
    // `Option<MemberUser>` is `Some` or `None`. The displayable label is
    // produced by the generated async helper.
    let row_some = WithNested {
        id: 21,
        user: None,
        fallback_email: None,
        async_user: Some(MemberUser {
            name: "Grace".to_owned(),
            email: "grace@example.com".to_owned(),
        }),
    };
    assert_eq!(row_some.to_row()[3], CellValue::Empty);

    let row_none = WithNested {
        id: 22,
        user: None,
        fallback_email: None,
        async_user: None,
    };
    assert_eq!(row_none.to_row()[3], CellValue::Empty);
}

#[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
struct WithKnownAsyncField {
    #[excel(header = "ID", order = 1)]
    id: i64,
    // `value_parse_fn` with a known type: the cell is `Empty` and the
    // async helper produces the displayable label.
    #[excel(
        header = "Status Label",
        order = 2,
        value_parse_fn = "status_label(status, self)"
    )]
    status: i32,
}

async fn status_label(status: &i32, entity: &WithKnownAsyncField) -> String {
    let _ = entity;
    format!("status-{status}")
}

#[test]
fn derive_value_parse_fn_with_known_type_leaves_cell_empty() {
    let row = WithKnownAsyncField { id: 1, status: 7 };
    let cells = row.to_row();
    assert_eq!(cells[0], CellValue::Int(1));
    assert_eq!(cells[1], CellValue::Empty);

    let label = block_on(row.value_parse_fn_status(&row.status));
    assert_eq!(label, "status-7");
}

#[derive(Debug, PartialEq, Clone, excelx_derive::ExcelRow)]
struct TypedRow {
    #[excel(header = "Price", order = 1)]
    price: Decimal,
    #[excel(header = "Discount", order = 2)]
    discount: Option<Decimal>,
    #[excel(header = "Published At", order = 3)]
    published_at: NaiveDateTime,
    #[excel(header = "Birthday", order = 4)]
    birthday: NaiveDate,
    #[excel(header = "Start", order = 5)]
    start: NaiveTime,
    #[excel(header = "Avatar", order = 6)]
    avatar: Vec<u8>,
    #[excel(header = "Tags", order = 7, default = "")]
    tags: Vec<String>,
}

#[test]
fn derive_round_trips_decimal_and_chrono_types() {
    let price: Decimal = "199.99".parse().unwrap();
    let discount: Decimal = "0.15".parse().unwrap();
    let published_at = NaiveDateTime::parse_from_str("2024-05-01 12:34:56", "%Y-%m-%d %H:%M:%S").unwrap();
    let birthday = NaiveDate::from_ymd_opt(1990, 1, 2).unwrap();
    let start = NaiveTime::from_hms_opt(9, 30, 0).unwrap();

    let row = TypedRow {
        price,
        discount: Some(discount),
        published_at,
        birthday,
        start,
        avatar: vec![1, 2, 3, 4],
        tags: vec!["alpha".to_owned(), "beta".to_owned()],
    };

    let bytes = to_xlsx(&[row.clone()]).expect("write workbook");
    let parsed = from_xlsx::<TypedRow>(&bytes).expect("parse workbook");

    assert_eq!(parsed, vec![row]);
}

#[test]
fn derive_columns_metadata_for_typed_row() {
    assert_eq!(
        TypedRow::columns(),
        vec![
            ColumnDef::new("price", "Price", 1),
            ColumnDef::new("discount", "Discount", 2),
            ColumnDef::new("published_at", "Published At", 3),
            ColumnDef::new("birthday", "Birthday", 4),
            ColumnDef::new("start", "Start", 5),
            ColumnDef::new("avatar", "Avatar", 6),
            ColumnDef::with_default("tags", "Tags", 7, ""),
        ]
    );
}

#[test]
fn derive_optional_decimal_is_none_for_empty_cell() {
    let price: Decimal = "10.00".parse().unwrap();
    let published_at = NaiveDateTime::parse_from_str("2024-05-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
    let birthday = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let start = NaiveTime::from_hms_opt(0, 0, 0).unwrap();

    let row = TypedRow {
        price,
        discount: None,
        published_at,
        birthday,
        start,
        avatar: vec![],
        tags: vec![],
    };

    let bytes = to_xlsx(&[row]).expect("write workbook");
    let parsed = from_xlsx::<TypedRow>(&bytes).expect("parse workbook");

    assert_eq!(parsed[0].discount, None);
    assert!(parsed[0].avatar.is_empty());
    assert!(parsed[0].tags.is_empty());
}

#[test]
fn derive_decimal_default_applies_for_empty_cell() {
    #[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
    struct DecimalDefaults {
        #[excel(header = "ID", order = 1)]
        id: i64,
        #[excel(header = "Rate", order = 2, default = "0.0825")]
        rate: Option<Decimal>,
    }

    let row = DecimalDefaults {
        id: 1,
        rate: None,
    };
    let bytes = to_xlsx(&[row]).expect("write workbook");
    let parsed = from_xlsx::<DecimalDefaults>(&bytes).expect("parse workbook");

    let expected: Decimal = "0.0825".parse().unwrap();
    assert_eq!(
        parsed,
        vec![DecimalDefaults {
            id: 1,
            rate: Some(expected),
        }]
    );
}

#[test]
fn derive_datetime_default_applies_for_empty_cell() {
    #[derive(Debug, PartialEq, excelx_derive::ExcelRow)]
    struct DateTimeDefaults {
        #[excel(header = "ID", order = 1)]
        id: i64,
        #[excel(header = "When", order = 2, default = "2024-01-01T00:00:00")]
        when: Option<NaiveDateTime>,
    }

    let row = DateTimeDefaults { id: 1, when: None };
    let bytes = to_xlsx(&[row]).expect("write workbook");
    let parsed = from_xlsx::<DateTimeDefaults>(&bytes).expect("parse workbook");

    let expected = NaiveDateTime::parse_from_str("2024-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S").unwrap();
    assert_eq!(
        parsed,
        vec![DateTimeDefaults {
            id: 1,
            when: Some(expected),
        }]
    );
}
