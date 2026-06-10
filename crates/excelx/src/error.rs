use thiserror::Error;

/// Errors returned by schema validation, XLSX writing, and XLSX parsing.
#[derive(Debug, Error)]
pub enum ExcelError {
    #[error("schema error: {0}")]
    Schema(String),

    #[error("missing required header: {0}")]
    MissingHeader(String),

    #[error("duplicate column order: {0}")]
    DuplicateColumnOrder(usize),

    #[error("duplicate header: {0}")]
    DuplicateHeader(String),

    #[error("invalid cell type at row {row}, column {column}: expected {expected}, found {found}")]
    InvalidCellType {
        row: usize,
        column: String,
        expected: String,
        found: String,
    },

    #[error("invalid default for field {field} ({header}): expected {expected}, value {value:?}")]
    InvalidDefault {
        field: String,
        header: String,
        expected: String,
        value: String,
    },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("write error: {0}")]
    Write(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<calamine::XlsxError> for ExcelError {
    fn from(value: calamine::XlsxError) -> Self {
        Self::Parse(value.to_string())
    }
}

impl From<rust_xlsxwriter::XlsxError> for ExcelError {
    fn from(value: rust_xlsxwriter::XlsxError) -> Self {
        Self::Write(value.to_string())
    }
}
