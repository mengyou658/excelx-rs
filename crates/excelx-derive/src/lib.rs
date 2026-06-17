use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Expr, ExprLit, Fields, GenericArgument, Lit, PathArguments, Type,
    parse_macro_input,
};

#[proc_macro_derive(ExcelRow, attributes(excel))]
pub fn derive_excel_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_excel_row(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_excel_row(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &input.ident;
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    input,
                    "ExcelRow can only be derived for structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "ExcelRow can only be derived for structs",
            ));
        }
    };

    let mut column_defs = Vec::with_capacity(fields.len());
    let mut to_row_values = Vec::with_capacity(fields.len());
    let mut from_row_fields = Vec::with_capacity(fields.len());
    let mut impl_methods = Vec::new();

    for field in fields {
        let field_ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "field must be named"))?;
        let field_name = field_ident.to_string();
        let attrs = ExcelAttrs::parse(field)?;
        let header = attrs
            .header
            .ok_or_else(|| syn::Error::new_spanned(field, "missing #[excel(header = \"...\")]"))?;
        let order = attrs
            .order
            .ok_or_else(|| syn::Error::new_spanned(field, "missing #[excel(order = ...)]"))?;

        let default_tokens = attrs
            .default
            .as_ref()
            .map(|value| quote! { Some(#value) })
            .unwrap_or_else(|| quote! { None });

        column_defs.push(quote! {
            ::excelx::ColumnDef {
                field: #field_name,
                header: #header,
                order: #order,
                default: #default_tokens,
            }
        });

        // `value_parse_fn` takes precedence over `value`: when the user has
        // defined a custom async helper for this field, the deep-field access
        // path is ignored and the cell is left empty. The actual displayed
        // value is produced by the generated async method.
        let has_value_parse_fn = attrs.value_parse_fn.is_some();
        let has_value = attrs.value.is_some() && !has_value_parse_fn;
        let conversion = if has_value {
            None
        } else if has_value_parse_fn {
            // The field will be left at `Default` for `from_row` and the cell
            // value is `Empty`, so the conversion-based `from_row` path is
            // skipped entirely.
            None
        } else {
            Some(FieldConversion::for_type(&field.ty)?)
        };

        let to_row_value = if let (Some(path), true) = (&attrs.value, has_value) {
            build_value_cell_value(field_ident, &field_name, &field.ty, path)?
        } else if has_value_parse_fn {
            quote! { ::excelx::CellValue::Empty }
        } else {
            conversion
                .as_ref()
                .expect("conversion is present when value and value_parse_fn are absent")
                .to_cell_value(field_ident)?
        };
        to_row_values.push(to_row_value);

        let from_row_init = if let Some(conv) = conversion {
            conv.build_field_initializer(field_ident, &field_name)?
        } else {
            quote! { #field_ident: ::std::default::Default::default() }
        };
        from_row_fields.push(from_row_init);

        if let Some(expr) = &attrs.value_parse_fn {
            impl_methods.push(build_value_parse_fn_method(field_ident, &field.ty, expr)?);
        }
    }

    Ok(quote! {
        impl ::excelx::ExcelRow for #ident {
            fn columns() -> ::std::vec::Vec<::excelx::ColumnDef> {
                ::std::vec![#(#column_defs),*]
            }

            fn to_row(&self) -> ::std::vec::Vec<::excelx::CellValue> {
                ::std::vec![#(#to_row_values),*]
            }

            fn from_row(row: &::excelx::RowView) -> ::std::result::Result<Self, ::excelx::ExcelError> {
                ::std::result::Result::Ok(Self {
                    #(#from_row_fields),*
                })
            }
        }

        impl #ident {
            #(#impl_methods)*
        }
    })
}

#[derive(Default)]
struct ExcelAttrs {
    header: Option<String>,
    order: Option<usize>,
    default: Option<String>,
    /// Deep field access path, e.g. `user.name` where `user` is the field name
    /// and `name` is a field on the inner type. Used in `to_row()` to derive
    /// the cell value from a nested field.
    value: Option<Vec<String>>,
    /// A function call expression used to generate an `async fn` helper method
    /// on the struct. The function call's argument list may reference the
    /// field name (replaced with the helper's `value` parameter) and `self`
    /// (which refers to the entity instance inside the generated method).
    value_parse_fn: Option<syn::Expr>,
}

impl ExcelAttrs {
    fn parse(field: &syn::Field) -> syn::Result<Self> {
        let mut attrs = Self::default();

        for attr in &field.attrs {
            if !attr.path().is_ident("excel") {
                continue;
            }

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("header") {
                    let value = meta.value()?;
                    attrs.header = Some(value.parse::<syn::LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("order") {
                    let value = meta.value()?;
                    attrs.order = Some(parse_usize_lit(value.parse::<Expr>()?)?);
                    Ok(())
                } else if meta.path.is_ident("default") {
                    let value = meta.value()?;
                    attrs.default = Some(value.parse::<syn::LitStr>()?.value());
                    Ok(())
                } else if meta.path.is_ident("value") {
                    let value = meta.value()?;
                    let path: String = value.parse::<syn::LitStr>()?.value();
                    if path.is_empty() {
                        return Err(meta.error("value path cannot be empty"));
                    }
                    attrs.value = Some(path.split('.').map(|s| s.to_owned()).collect());
                    Ok(())
                } else if meta.path.is_ident("value_parse_fn") {
                    let value = meta.value()?;
                    let s: String = value.parse::<syn::LitStr>()?.value();
                    let expr: syn::Expr = syn::parse_str(&s)?;
                    if !matches!(expr, syn::Expr::Call(_)) {
                        return Err(meta.error("value_parse_fn must be a function call expression"));
                    }
                    attrs.value_parse_fn = Some(expr);
                    Ok(())
                } else {
                    Err(meta.error("unsupported excel attribute"))
                }
            })?;
        }

        Ok(attrs)
    }
}

fn parse_usize_lit(expr: Expr) -> syn::Result<usize> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value.base10_parse(),
        other => Err(syn::Error::new_spanned(
            other,
            "order must be an unsigned integer literal",
        )),
    }
}

enum FieldConversion<'a> {
    String,
    Integer(&'a Type),
    Float(&'a Type),
    Bool,
    #[cfg(feature = "decimal")]
    Decimal,
    #[cfg(feature = "chrono")]
    DateTime,
    #[cfg(feature = "chrono")]
    Date,
    #[cfg(feature = "chrono")]
    Time,
    Bytes,
    StringList,
    Option(Box<FieldConversion<'a>>),
}

impl<'a> FieldConversion<'a> {
    fn for_type(ty: &'a Type) -> syn::Result<Self> {
        if let Some(inner) = option_inner_type(ty) {
            if option_inner_type(inner).is_some() {
                return Err(syn::Error::new_spanned(
                    ty,
                    "nested Option fields are not supported",
                ));
            }

            return Ok(Self::Option(Box::new(Self::for_type(inner)?)));
        }

        if type_is(ty, "String") {
            return Ok(Self::String);
        }

        if type_is(ty, "bool") {
            return Ok(Self::Bool);
        }

        if is_supported_integer(ty) {
            return Ok(Self::Integer(ty));
        }

        if is_supported_float(ty) {
            return Ok(Self::Float(ty));
        }

        #[cfg(feature = "decimal")]
        if type_path_is(ty, &["rust_decimal", "Decimal"]) {
            return Ok(Self::Decimal);
        }

        if type_path_is(ty, &["rust_decimal", "Decimal"]) {
            return Err(syn::Error::new_spanned(
                ty,
                "rust_decimal::Decimal fields require the `decimal` feature on excelx-derive and excelx",
            ));
        }

        #[cfg(feature = "chrono")]
        if type_path_is(ty, &["chrono", "NaiveDateTime"]) {
            return Ok(Self::DateTime);
        }

        if type_path_is(ty, &["chrono", "NaiveDateTime"]) {
            return Err(syn::Error::new_spanned(
                ty,
                "chrono::NaiveDateTime fields require the `chrono` feature on excelx-derive and excelx",
            ));
        }

        #[cfg(feature = "chrono")]
        if type_path_is(ty, &["chrono", "NaiveDate"]) {
            return Ok(Self::Date);
        }

        if type_path_is(ty, &["chrono", "NaiveDate"]) {
            return Err(syn::Error::new_spanned(
                ty,
                "chrono::NaiveDate fields require the `chrono` feature on excelx-derive and excelx",
            ));
        }

        #[cfg(feature = "chrono")]
        if type_path_is(ty, &["chrono", "NaiveTime"]) {
            return Ok(Self::Time);
        }

        if type_path_is(ty, &["chrono", "NaiveTime"]) {
            return Err(syn::Error::new_spanned(
                ty,
                "chrono::NaiveTime fields require the `chrono` feature on excelx-derive and excelx",
            ));
        }

        if is_vec_of(ty, "u8") {
            return Ok(Self::Bytes);
        }

        if is_vec_of(ty, "String") {
            return Ok(Self::StringList);
        }

        Err(syn::Error::new_spanned(
            ty,
            "unsupported ExcelRow field type",
        ))
    }

    fn to_cell_value(&self, field_ident: &syn::Ident) -> syn::Result<proc_macro2::TokenStream> {
        match self {
            Self::String => Ok(quote! { ::excelx::CellValue::String(self.#field_ident.clone()) }),
            Self::Integer(_) => Ok(quote! { ::excelx::CellValue::Int(self.#field_ident.into()) }),
            Self::Float(ty) if type_is(ty, "f32") => Ok(
                quote! { ::excelx::CellValue::Float(::std::convert::Into::<f64>::into(self.#field_ident)) },
            ),
            Self::Float(_) => Ok(quote! { ::excelx::CellValue::Float(self.#field_ident) }),
            Self::Bool => Ok(quote! { ::excelx::CellValue::Bool(self.#field_ident) }),
            #[cfg(feature = "decimal")]
            Self::Decimal => Ok(quote! { ::excelx::CellValue::Decimal(self.#field_ident) }),
            #[cfg(feature = "chrono")]
            Self::DateTime => Ok(quote! { ::excelx::CellValue::DateTime(self.#field_ident) }),
            #[cfg(feature = "chrono")]
            Self::Date => Ok(quote! { ::excelx::CellValue::Date(self.#field_ident) }),
            #[cfg(feature = "chrono")]
            Self::Time => Ok(quote! { ::excelx::CellValue::Time(self.#field_ident) }),
            Self::Bytes => Ok(quote! { ::excelx::CellValue::Bytes(self.#field_ident.clone()) }),
            Self::StringList => Ok(quote! { ::excelx::CellValue::StringList(self.#field_ident.clone()) }),
            Self::Option(inner) => {
                let value_ident = format_ident!("value");
                let inner_tokens = inner.to_cell_value_for_value(&value_ident)?;
                Ok(quote! {
                    match &self.#field_ident {
                        ::std::option::Option::Some(#value_ident) => #inner_tokens,
                        ::std::option::Option::None => ::excelx::CellValue::Empty,
                    }
                })
            }
        }
    }

    fn to_cell_value_for_value(
        &self,
        value_ident: &syn::Ident,
    ) -> syn::Result<proc_macro2::TokenStream> {
        match self {
            Self::String => Ok(quote! { ::excelx::CellValue::String(#value_ident.clone()) }),
            Self::Integer(_) => Ok(quote! { ::excelx::CellValue::Int((*#value_ident).into()) }),
            Self::Float(ty) if type_is(ty, "f32") => Ok(
                quote! { ::excelx::CellValue::Float(::std::convert::Into::<f64>::into(*#value_ident)) },
            ),
            Self::Float(_) => Ok(quote! { ::excelx::CellValue::Float(*#value_ident) }),
            Self::Bool => Ok(quote! { ::excelx::CellValue::Bool(*#value_ident) }),
            #[cfg(feature = "decimal")]
            Self::Decimal => Ok(quote! { ::excelx::CellValue::Decimal(*#value_ident) }),
            #[cfg(feature = "chrono")]
            Self::DateTime => Ok(quote! { ::excelx::CellValue::DateTime(*#value_ident) }),
            #[cfg(feature = "chrono")]
            Self::Date => Ok(quote! { ::excelx::CellValue::Date(*#value_ident) }),
            #[cfg(feature = "chrono")]
            Self::Time => Ok(quote! { ::excelx::CellValue::Time(*#value_ident) }),
            Self::Bytes => Ok(quote! { ::excelx::CellValue::Bytes(#value_ident.clone()) }),
            Self::StringList => Ok(quote! { ::excelx::CellValue::StringList(#value_ident.clone()) }),
            Self::Option(_) => Err(syn::Error::new_spanned(
                value_ident,
                "nested Option fields are not supported",
            )),
        }
    }

    fn build_field_initializer(
        &self,
        field_ident: &syn::Ident,
        field_name: &str,
    ) -> syn::Result<proc_macro2::TokenStream> {
        let value_expr = self.required_accessor_expr(field_name)?;
        Ok(quote! { #field_ident: #value_expr })
    }

    fn required_accessor_expr(&self, field_name: &str) -> syn::Result<proc_macro2::TokenStream> {
        match self {
            Self::String => Ok(quote! { row.required_string(#field_name)? }),
            Self::Integer(ty) if type_is(ty, "i64") => {
                Ok(quote! { row.required_i64(#field_name)? })
            }
            Self::Integer(ty) => Ok(quote! {
                row.required_i64(#field_name)?.try_into().map_err(|_| {
                    ::excelx::ExcelError::InvalidCellType {
                        row: row.row_number(),
                        column: row.header_for_field(#field_name),
                        expected: ::std::stringify!(#ty).to_owned(),
                        found: "integer out of range".to_owned(),
                    }
                })?
            }),
            Self::Float(ty) if type_is(ty, "f32") => {
                Ok(quote! { row.required_f64(#field_name)? as f32 })
            }
            Self::Float(_) => Ok(quote! { row.required_f64(#field_name)? }),
            Self::Bool => Ok(quote! { row.required_bool(#field_name)? }),
            #[cfg(feature = "decimal")]
            Self::Decimal => Ok(quote! { row.required_decimal(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::DateTime => Ok(quote! { row.required_datetime(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::Date => Ok(quote! { row.required_date(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::Time => Ok(quote! { row.required_time(#field_name)? }),
            Self::Bytes => Ok(quote! { row.required_bytes(#field_name)? }),
            Self::StringList => Ok(quote! { row.required_string_list(#field_name)? }),
            Self::Option(inner) => inner.optional_accessor_expr(field_name),
        }
    }

    fn optional_accessor_expr(&self, field_name: &str) -> syn::Result<proc_macro2::TokenStream> {
        match self {
            Self::String => Ok(quote! { row.optional_string(#field_name)? }),
            Self::Integer(ty) if type_is(ty, "i64") => {
                Ok(quote! { row.optional_i64(#field_name)? })
            }
            Self::Integer(ty) => Ok(quote! {
                match row.optional_i64(#field_name)? {
                    ::std::option::Option::Some(value) => {
                        ::std::option::Option::Some(value.try_into().map_err(|_| {
                            ::excelx::ExcelError::InvalidCellType {
                                row: row.row_number(),
                                column: row.header_for_field(#field_name),
                                expected: ::std::stringify!(#ty).to_owned(),
                                found: "integer out of range".to_owned(),
                            }
                        })?)
                    }
                    ::std::option::Option::None => ::std::option::Option::None,
                }
            }),
            Self::Float(ty) if type_is(ty, "f32") => {
                Ok(quote! { row.optional_f64(#field_name)?.map(|value| value as f32) })
            }
            Self::Float(_) => Ok(quote! { row.optional_f64(#field_name)? }),
            Self::Bool => Ok(quote! { row.optional_bool(#field_name)? }),
            #[cfg(feature = "decimal")]
            Self::Decimal => Ok(quote! { row.optional_decimal(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::DateTime => Ok(quote! { row.optional_datetime(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::Date => Ok(quote! { row.optional_date(#field_name)? }),
            #[cfg(feature = "chrono")]
            Self::Time => Ok(quote! { row.optional_time(#field_name)? }),
            Self::Bytes => Ok(quote! { row.optional_bytes(#field_name)? }),
            Self::StringList => Ok(quote! { row.optional_string_list(#field_name)? }),
            Self::Option(_) => Err(syn::Error::new_spanned(
                field_name,
                "nested Option fields are not supported",
            )),
        }
    }
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };

    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }

    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };

    match args.args.first()? {
        GenericArgument::Type(inner) => Some(inner),
        _ => None,
    }
}

fn type_is(ty: &Type, expected: &str) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    type_path
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == expected)
}

/// Match a type path against `segments`.
///
/// Accepts two forms:
///
/// * An exact path match, e.g. `rust_decimal::Decimal` matches
///   `&["rust_decimal", "Decimal"]`.
/// * A bare ident that matches the last segment, e.g. `Decimal` (after
///   `use rust_decimal::Decimal;`) also matches
///   `&["rust_decimal", "Decimal"]`. This is the common case in user code
///   that imports the type with `use`.
fn type_path_is(ty: &Type, segments: &[&str]) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    let path_segments = &type_path.path.segments;

    if path_segments.len() == segments.len() {
        return path_segments
            .iter()
            .zip(segments.iter())
            .all(|(actual, expected)| actual.ident == *expected);
    }

    if path_segments.len() == 1 {
        if let (Some(actual), Some(expected)) = (path_segments.first(), segments.last()) {
            return actual.ident == *expected;
        }
    }

    false
}

fn is_supported_integer(ty: &Type) -> bool {
    ["i8", "i16", "i32", "i64", "u8", "u16", "u32"]
        .iter()
        .any(|expected| type_is(ty, expected))
}

fn is_supported_float(ty: &Type) -> bool {
    ["f32", "f64"].iter().any(|expected| type_is(ty, expected))
}

/// Match `Vec<T>` where the inner type's last ident equals `inner_ident`.
/// For example, `is_vec_of(ty, "u8")` returns `true` for `Vec<u8>`,
/// `std::vec::Vec<u8>`, and `Vec<::std::primitive::u8>` (any single-segment
/// inner type with ident `u8`).
fn is_vec_of(ty: &Type, inner_ident: &str) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };

    let Some(segment) = type_path.path.segments.last() else {
        return false;
    };

    if segment.ident != "Vec" {
        return false;
    }

    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return false;
    };

    args.args.iter().any(|arg| match arg {
        GenericArgument::Type(Type::Path(inner)) => inner
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == inner_ident),
        _ => false,
    })
}

/// Build a `CellValue` expression that reads a deep field path on the field.
///
/// The first segment of `path` must be the field's own name. If the field's
/// type is `Option<T>`, the chain after the field name is evaluated against
/// the unwrapped inner value; a `None` produces `CellValue::Empty`. Otherwise
/// the chain is evaluated directly on the field value. The final value is
/// converted to `CellValue::String` via `Display`, so all leaf types must
/// implement `std::fmt::Display`.
fn build_value_cell_value(
    field_ident: &syn::Ident,
    field_name: &str,
    field_ty: &Type,
    path: &[String],
) -> syn::Result<proc_macro2::TokenStream> {
    if path.is_empty() {
        return Err(syn::Error::new_spanned(
            field_ident,
            "value path cannot be empty",
        ));
    }

    if path[0] != field_name {
        return Err(syn::Error::new_spanned(
            field_ident,
            format!(
                "value path must start with the field name `{}`",
                field_name
            ),
        ));
    }

    let is_option = option_inner_type(field_ty).is_some();
    let rest = &path[1..];

    if is_option {
        let inner_expr = build_chain_on_value_ident(rest);
        Ok(quote! {
            match &self.#field_ident {
                ::std::option::Option::Some(__excelx_value) => {
                    ::excelx::CellValue::String((#inner_expr).to_string())
                }
                ::std::option::Option::None => ::excelx::CellValue::Empty,
            }
        })
    } else {
        let access = build_chain_on_field(rest, field_ident);
        Ok(quote! {
            ::excelx::CellValue::String((#access).to_string())
        })
    }
}

fn build_chain_on_value_ident(segments: &[String]) -> proc_macro2::TokenStream {
    let value_ident = format_ident!("__excelx_value");
    let mut tokens = quote! { #value_ident };
    for segment in segments {
        let ident = format_ident!("{}", segment);
        tokens.extend(quote! { .#ident });
    }
    tokens
}

fn build_chain_on_field(segments: &[String], field_ident: &syn::Ident) -> proc_macro2::TokenStream {
    let mut tokens = quote! { self.#field_ident };
    for segment in segments {
        let ident = format_ident!("{}", segment);
        tokens.extend(quote! { .#ident });
    }
    tokens
}

/// Build an `async fn` helper named `value_parse_fn_<field_name>` on the
/// struct. The method takes `(&self, value: &FieldType)` and returns the
/// result of the user-supplied function call as a `String`.
///
/// The function call expression is parsed once at proc-macro time. Within the
/// argument list, any bare identifier matching the field name is replaced
/// with the helper's `value` parameter, and `self` is left as-is (it already
/// refers to the entity inside the generated method).
fn build_value_parse_fn_method(
    field_ident: &syn::Ident,
    field_ty: &Type,
    expr: &syn::Expr,
) -> syn::Result<proc_macro2::TokenStream> {
    let method_name = format_ident!("value_parse_fn_{}", field_ident);

    let call = match expr {
        syn::Expr::Call(call) => call,
        _ => {
            return Err(syn::Error::new_spanned(
                expr,
                "value_parse_fn must be a function call expression",
            ))
        }
    };

    let field_name = field_ident.to_string();
    let mut new_args: Vec<syn::Expr> = Vec::with_capacity(call.args.len());
    for arg in &call.args {
        new_args.push(replace_field_ident(arg, &field_name));
    }

    let func_expr = &call.func;

    Ok(quote! {
        #[allow(dead_code)]
        pub async fn #method_name(&self, value: &#field_ty) -> ::std::string::String {
            (#func_expr)(#(#new_args),*).await.to_string()
        }
    })
}

fn replace_field_ident(expr: &syn::Expr, field_name: &str) -> syn::Expr {
    if let syn::Expr::Path(path) = expr {
        if path.path.is_ident(field_name) {
            return syn::parse_quote! { value };
        }
    }
    expr.clone()
}
