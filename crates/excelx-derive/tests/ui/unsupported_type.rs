use excelx_derive::ExcelRow;

#[derive(ExcelRow)]
struct UnsupportedType {
    #[excel(header = "Tags", order = 1)]
    tags: std::collections::HashMap<String, String>,
}

fn main() {}
