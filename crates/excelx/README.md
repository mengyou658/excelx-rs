# excelx

Type-safe XLSX read/write helpers for Rust structs.

This crate supports manual `ExcelRow` implementations, selected-sheet reads,
homogeneous multi-sheet read/write, parse defaults, and explicit safety limits
for user-uploaded workbooks.

For production imports, prefer the limited APIs:

```rust
use excelx::{ReadLimits, SheetRef, from_xlsx_with_limits};

let rows = from_xlsx_with_limits::<Person>(
    &bytes,
    SheetRef::Index(0),
    ReadLimits::new().max_rows(10_000).max_columns(64),
)?;
# let _ = rows;
# Ok::<(), excelx::ExcelError>(())
```

See the workspace README for full examples, limitations, and compatibility
notes.
