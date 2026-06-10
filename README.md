# excelx-rs

`excelx` is a small Rust crate for converting struct collections to XLSX
worksheets and parsing them back with explicit header and column-order metadata.

The crate supports manual `ExcelRow` implementations, default values during
parse, selected-sheet reads, homogeneous multi-sheet read/write, and a derive
macro in the separate `excelx-derive` crate.

## MSRV

The minimum supported Rust version is `1.85.0`.

## Example

```rust
use excelx::{CellValue, ColumnDef, ExcelError, ExcelRow, RowView, from_xlsx, to_xlsx};

#[derive(Debug, PartialEq)]
struct Person {
    id: i64,
    name: String,
    active: bool,
}

impl ExcelRow for Person {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("id", "ID", 1),
            ColumnDef::new("name", "Name", 2),
            ColumnDef::new("active", "Active", 3),
        ]
    }

    fn to_row(&self) -> Vec<CellValue> {
        vec![
            self.id.into(),
            self.name.clone().into(),
            self.active.into(),
        ]
    }

    fn from_row(row: &RowView) -> Result<Self, ExcelError> {
        Ok(Self {
            id: row.required_i64("id")?,
            name: row.required_string("name")?,
            active: row.required_bool("active")?,
        })
    }
}

let people = vec![Person {
    id: 1,
    name: "Ada".to_owned(),
    active: true,
}];

let bytes = to_xlsx(&people)?;
let parsed = from_xlsx::<Person>(&bytes)?;
assert_eq!(parsed, people);
# Ok::<(), ExcelError>(())
```

## Defaults

Defaults are applied during parse when the header exists but the cell is empty.
Typed `RowView` accessors parse defaults for `String`, integer, float, and
boolean fields.

```rust
ColumnDef::with_default("status", "Status", 3, "new")
```

## Multi-sheet Write

```rust
use excelx::{SheetData, to_xlsx_multi};

let workbook = to_xlsx_multi(&[
    SheetData::new("Active", active_people),
    SheetData::new("Archive", archived_people),
])?;
# Ok::<(), ExcelError>(())
```

## Multi-sheet Read

`from_xlsx()` reads the first worksheet. Use `SheetRef` or `ReadOptions` to
read a specific worksheet, or `from_xlsx_multi()` when every worksheet has the
same row schema.

```rust
use excelx::{SheetRef, from_xlsx_multi, from_xlsx_sheet};

let archive = from_xlsx_sheet::<Person>(&bytes, SheetRef::Name("Archive"))?;
let second_sheet = from_xlsx_sheet::<Person>(&bytes, SheetRef::Index(1))?;
let all_sheets = from_xlsx_multi::<Person>(&bytes)?;
# let _ = (archive, second_sheet, all_sheets);
# Ok::<(), ExcelError>(())
```

## Safety Limits

When parsing user-uploaded or otherwise untrusted workbooks, use explicit read
limits so unexpectedly large files fail before they consume too much memory:

```rust
use excelx::{ReadLimits, SheetRef, from_xlsx_with_limits};

let rows = from_xlsx_with_limits::<Person>(
    &bytes,
    SheetRef::Index(0),
    ReadLimits::new().max_rows(10_000).max_columns(64),
)?;
# let _ = rows;
# Ok::<(), ExcelError>(())
```

For homogeneous multi-sheet imports, use `MultiSheetReadLimits` with
`from_xlsx_multi_with_limits()`.

Writes are also checked against the XLSX worksheet limits of 1,048,576 rows
including the header row and 16,384 columns.

## Derive Macro

Add `excelx-derive` next to `excelx`, then derive the trait with field
metadata:

```rust
#[derive(excelx_derive::ExcelRow)]
struct Person {
    #[excel(header = "ID", order = 1)]
    id: i64,
    #[excel(header = "Name", order = 2)]
    name: String,
    #[excel(header = "Active", order = 3, default = "true")]
    active: bool,
    #[excel(header = "Nickname", order = 4, default = "N/A")]
    nickname: Option<String>,
}
```

The initial macro release supports named structs with `String`,
`Option<String>`, supported integer types, `f32`/`f64`, `bool`, and optional
scalar fields.

## Limitations

`excelx` is intentionally small. Current limitations:

* `from_xlsx()` reads the first worksheet by default. Use `SheetRef` or
  `ReadOptions` to select a worksheet explicitly.
* Multi-sheet read/write is homogeneous. Every parsed or written sheet must use
  the same row type.
* Integer writes go through XLSX numeric cells, which are stored as floating
  point values by Excel. Very large integers can lose precision.
* Defaults apply during parse when a required header exists and the cell is
  empty. Defaults are not applied when a header is missing.
* Date/time cells, formulas, styles, streaming large files, and custom number
  formats are out of scope for this release.
* The derive crate supports named structs only.

## Compatibility Fixtures

The CI workflow includes a LibreOffice compatibility job that creates an `.xlsx`
file with `libreoffice --headless --convert-to xlsx` and parses it through the
public API.

The compatibility test can also read external `.xlsx` fixtures from
`EXCELX_COMPAT_FIXTURE_DIR`. Use this for files saved by Microsoft Excel or
other spreadsheet tools:

```sh
EXCELX_COMPAT_FIXTURE_DIR=crates/excelx/tests/fixtures/compat \
  cargo test -p excelx --test compatibility_workbooks
```

Expected fixture shape:

```text
ID,Name,Active,Score
1,Ada,true,98.5
2,Grace,false,88
```
