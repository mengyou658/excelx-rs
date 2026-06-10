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

        let conversion = FieldConversion::for_type(&field.ty)?;
        to_row_values.push(conversion.to_cell_value(field_ident)?);
        from_row_fields.push(conversion.build_field_initializer(field_ident, &field_name)?);
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
    })
}

#[derive(Default)]
struct ExcelAttrs {
    header: Option<String>,
    order: Option<usize>,
    default: Option<String>,
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

fn is_supported_integer(ty: &Type) -> bool {
    ["i8", "i16", "i32", "i64", "u8", "u16", "u32"]
        .iter()
        .any(|expected| type_is(ty, expected))
}

fn is_supported_float(ty: &Type) -> bool {
    ["f32", "f64"].iter().any(|expected| type_is(ty, expected))
}
