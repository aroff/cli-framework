//! Proc-macro derive for cli-framework's typed command API.
//!
//! # Usage
//!
//! ```rust,ignore
//! use cli_framework::CommandSpec;
//!
//! #[derive(CommandSpec)]
//! #[command(about = "Run optimization")]
//! #[cfw(category = "quality")]
//! struct RunArgs {
//!     #[arg(long, required)]
//!     config: std::path::PathBuf,
//!
//!     #[arg(long)]
//!     verbose: bool,
//!
//!     #[arg(long)]
//!     out_dir: Option<std::path::PathBuf>,
//! }
//! ```
//!
//! This generates `impl IntoCommandSpec for RunArgs` (the `CommandSpec`) and
//! `impl FromArgValueMap for RunArgs` (the infallible extractor).

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Fields, GenericArgument, Ident, Lit, PathArguments, Type,
};

/// Derive `IntoCommandSpec` and `FromArgValueMap` for a struct.
///
/// Attribute vocabulary:
/// - `#[command(about = "...")]` — command summary
/// - `#[command(long_about = "...")]` — extended description
/// - `#[cfw(category = "...")]` — help group category
/// - `#[cfw(syntax = "...")]` — usage hint line
/// - `#[cfw(deprecated = "...")]` — deprecation message
/// - `#[cfw(note = "...")]` — notes section
/// - `#[cfw(example = "...")]` — example (repeatable)
/// - `#[arg(long)]` / `#[arg(long = "name")]` — flag long name
/// - `#[arg(short)]` / `#[arg(short = 'x')]` — short flag
/// - `#[arg(required)]` — override cardinality to Required
/// - `#[arg(help = "...")]` — arg help text
#[proc_macro_derive(CommandSpec, attributes(command, cfw, arg))]
pub fn derive_command_spec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_impl(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "CommandSpec can only be derived on structs with named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "CommandSpec can only be derived on structs",
            ))
        }
    };

    // Parse struct-level attributes
    let mut summary = String::new();
    let mut long_about: Option<String> = None;
    let mut category: Option<String> = None;
    let mut syntax: Option<String> = None;
    let mut deprecated: Option<String> = None;
    let mut notes: Option<String> = None;
    let mut examples: Vec<String> = Vec::new();

    for attr in &input.attrs {
        if attr.path().is_ident("command") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("about") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        summary = s.value();
                    }
                } else if meta.path.is_ident("long_about") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        long_about = Some(s.value());
                    }
                }
                Ok(())
            });
        } else if attr.path().is_ident("cfw") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("category") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        category = Some(s.value());
                    }
                } else if meta.path.is_ident("syntax") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        syntax = Some(s.value());
                    }
                } else if meta.path.is_ident("deprecated") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        deprecated = Some(s.value());
                    }
                } else if meta.path.is_ident("note") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        notes = Some(s.value());
                    }
                } else if meta.path.is_ident("example") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        examples.push(s.value());
                    }
                }
                Ok(())
            });
        }
    }

    // Generate ArgSpec for each field
    let mut arg_specs = Vec::new();
    let mut extractors = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        // Convert snake_case to kebab-case for the CLI flag name
        let flag_name = field_name_str.replace('_', "-");

        // Inspect field type to determine cardinality and value type
        let (is_option, is_vec, inner_ty) = unwrap_option_or_vec(&field.ty);

        // Determine ArgKind and ArgValueType from inner_ty
        let (arg_kind_tokens, value_type_tokens, is_bool) = infer_arg_kind_and_type(inner_ty);

        // Cardinality: bool flags are Optional, Option<T> is Optional, Vec<T> is Repeated,
        // everything else is Required (unless #[arg(required)] forces it)
        let base_cardinality = if is_bool || is_option {
            "Optional"
        } else if is_vec {
            "Repeated"
        } else {
            "Required"
        };

        // Parse field-level #[arg(...)] attributes
        let mut long_override: Option<String> = None;
        let mut short_char: Option<char> = None;
        let mut help_text = String::new();
        let mut force_required = false;

        for attr in &field.attrs {
            if attr.path().is_ident("arg") {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("long") {
                        if meta.input.peek(syn::Token![=]) {
                            let value: Lit = meta.value()?.parse()?;
                            if let Lit::Str(s) = value {
                                long_override = Some(s.value());
                            }
                        }
                        // bare `long` uses field name (already the default)
                    } else if meta.path.is_ident("short") {
                        if meta.input.peek(syn::Token![=]) {
                            let value: Lit = meta.value()?.parse()?;
                            if let Lit::Char(c) = value {
                                short_char = Some(c.value());
                            }
                        } else {
                            // bare `short` → first char of field name
                            short_char = field_name_str.chars().next();
                        }
                    } else if meta.path.is_ident("required") {
                        force_required = true;
                    } else if meta.path.is_ident("help") {
                        let value: Lit = meta.value()?.parse()?;
                        if let Lit::Str(s) = value {
                            help_text = s.value();
                        }
                    }
                    Ok(())
                });
            }
        }

        let effective_long = long_override.as_deref().unwrap_or(&flag_name);

        let cardinality_str = if force_required {
            "Required"
        } else {
            base_cardinality
        };
        let cardinality_ident = Ident::new(cardinality_str, Span::call_site());

        let short_tokens = match short_char {
            Some(c) => quote! { Some(#c) },
            None => quote! { None },
        };

        let arg_spec = quote! {
            ::cli_framework::spec::arg_spec::ArgSpec {
                name: #effective_long,
                kind: #arg_kind_tokens,
                short: #short_tokens,
                long: Some(#effective_long),
                value_type: #value_type_tokens,
                cardinality: ::cli_framework::spec::arg_spec::Cardinality::#cardinality_ident,
                default: None,
                conflicts_with: vec![],
                requires: vec![],
                help: #help_text,
                ..Default::default()
            }
        };
        arg_specs.push(arg_spec);

        // Generate the extraction expression for FromArgValueMap
        let key = effective_long;
        let extractor = if is_bool {
            quote! {
                #field_name: matches!(
                    map.get(#key),
                    Some(::cli_framework::spec::value::ArgValue::Bool(true))
                ),
            }
        } else if is_option {
            // Option<T> extraction
            let extract_inner = extract_inner_value(inner_ty, key);
            quote! {
                #field_name: map.get(#key).and_then(|v| #extract_inner),
            }
        } else if is_vec {
            quote! {
                #field_name: match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::List(items)) => {
                        items.iter().filter_map(|v| {
                            if let ::cli_framework::spec::value::ArgValue::Str(s) = v {
                                Some(s.parse().unwrap_or_default())
                            } else {
                                None
                            }
                        }).collect()
                    }
                    _ => vec![],
                },
            }
        } else {
            // Required field: panic on missing (framework bug — should have been validated)
            let extract_req = extract_required_value(inner_ty, key, &field_name_str);
            quote! { #field_name: #extract_req, }
        };
        extractors.push(extractor);
    }

    // Build &'static str for summary/long_about/category/syntax etc.
    // We use string constants via Box::leak to get 'static lifetime.
    // For literal strings in attributes, we can use them directly as &'static str.
    let summary_ts = quote! { #summary };
    let long_about_ts = match long_about {
        Some(ref s) => quote! { Some(#s) },
        None => quote! { None },
    };
    let category_ts = match category {
        Some(ref s) => quote! { Some(#s) },
        None => quote! { None },
    };
    let syntax_ts = match syntax {
        Some(ref s) => quote! { Some(#s) },
        None => quote! { None },
    };
    let deprecated_ts = match deprecated {
        Some(ref s) => quote! { Some(#s) },
        None => quote! { None },
    };
    let notes_ts = match notes {
        Some(ref s) => quote! { Some(#s) },
        None => quote! { None },
    };
    let examples_ts = if examples.is_empty() {
        quote! { vec![] }
    } else {
        quote! { vec![ #(#examples),* ] }
    };

    let expanded = quote! {
        impl ::cli_framework::command::IntoCommandSpec for #name {
            fn command_spec() -> ::cli_framework::spec::command_tree::CommandSpec {
                ::cli_framework::spec::command_tree::CommandSpec {
                    summary: #summary_ts,
                    long_about: #long_about_ts,
                    category: #category_ts,
                    syntax: #syntax_ts,
                    deprecated: #deprecated_ts,
                    notes: #notes_ts,
                    examples: #examples_ts,
                    args: vec![ #(#arg_specs),* ],
                    ..Default::default()
                }
            }
        }

        impl ::cli_framework::command::FromArgValueMap for #name {
            fn from_arg_value_map(
                map: &::std::collections::HashMap<::std::string::String, ::cli_framework::spec::value::ArgValue>
            ) -> Self {
                Self {
                    #(#extractors)*
                }
            }
        }
    };

    Ok(expanded)
}

/// Determine if a type is Option<T>, Vec<T>, or bare T.
/// Returns (is_option, is_vec, inner_type).
fn unwrap_option_or_vec(ty: &Type) -> (bool, bool, &Type) {
    if let Type::Path(p) = ty {
        if let Some(seg) = p.path.segments.last() {
            if seg.ident == "Option" {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return (true, false, inner);
                    }
                }
            }
            if seg.ident == "Vec" {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(inner)) = args.args.first() {
                        return (false, true, inner);
                    }
                }
            }
        }
    }
    (false, false, ty)
}

/// Given a field type (unwrapped from Option/Vec), return (ArgKind tokens, ArgValueType tokens, is_bool).
fn infer_arg_kind_and_type(
    ty: &Type,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream, bool) {
    let type_name = extract_type_name(ty);

    match type_name.as_deref() {
        Some("bool") => (
            quote! { ::cli_framework::spec::arg_spec::ArgKind::Flag },
            quote! { ::cli_framework::spec::arg_spec::ArgValueType::Bool },
            true,
        ),
        Some("i64") | Some("i32") | Some("i16") | Some("i8") | Some("u64") | Some("u32")
        | Some("u16") | Some("u8") | Some("usize") | Some("isize") => (
            quote! { ::cli_framework::spec::arg_spec::ArgKind::Option },
            quote! { ::cli_framework::spec::arg_spec::ArgValueType::Int },
            false,
        ),
        Some("f64") | Some("f32") => (
            quote! { ::cli_framework::spec::arg_spec::ArgKind::Option },
            quote! { ::cli_framework::spec::arg_spec::ArgValueType::Float },
            false,
        ),
        _ => (
            // String, PathBuf, OsString, and anything else → Option/String
            quote! { ::cli_framework::spec::arg_spec::ArgKind::Option },
            quote! { ::cli_framework::spec::arg_spec::ArgValueType::String },
            false,
        ),
    }
}

fn extract_type_name(ty: &Type) -> Option<String> {
    if let Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

/// Generate extraction expression for an inner required (non-Option) field value.
fn extract_required_value(ty: &Type, key: &str, field_name: &str) -> proc_macro2::TokenStream {
    let type_name = extract_type_name(ty);
    match type_name.as_deref() {
        Some("i64") | Some("i32") | Some("i16") | Some("i8") | Some("u64") | Some("u32")
        | Some("u16") | Some("u8") | Some("usize") | Some("isize") => {
            quote! {
                match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::Int(i)) => *i as _,
                    _ => panic!("framework bug: required int arg '{}' missing from validated map", #field_name),
                }
            }
        }
        Some("f64") | Some("f32") => {
            quote! {
                match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::Float(f)) => *f as _,
                    _ => panic!("framework bug: required float arg '{}' missing from validated map", #field_name),
                }
            }
        }
        Some("String") => {
            quote! {
                match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::Str(s)) | Some(::cli_framework::spec::value::ArgValue::Enum(s)) => s.clone(),
                    _ => panic!("framework bug: required string arg '{}' missing from validated map", #field_name),
                }
            }
        }
        Some("PathBuf") => {
            quote! {
                match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::Str(s)) | Some(::cli_framework::spec::value::ArgValue::Enum(s)) => ::std::path::PathBuf::from(s),
                    _ => panic!("framework bug: required path arg '{}' missing from validated map", #field_name),
                }
            }
        }
        _ => {
            quote! {
                match map.get(#key) {
                    Some(::cli_framework::spec::value::ArgValue::Str(s)) | Some(::cli_framework::spec::value::ArgValue::Enum(s)) => {
                        s.parse().unwrap_or_else(|_| panic!("framework bug: failed to parse required arg '{}'", #field_name))
                    }
                    _ => panic!("framework bug: required arg '{}' missing from validated map", #field_name),
                }
            }
        }
    }
}

/// Generate extraction expression for an Option<T> field.
fn extract_inner_value(ty: &Type, _key: &str) -> proc_macro2::TokenStream {
    let type_name = extract_type_name(ty);
    match type_name.as_deref() {
        Some("i64") | Some("i32") | Some("i16") | Some("i8") | Some("u64") | Some("u32")
        | Some("u16") | Some("u8") | Some("usize") | Some("isize") => {
            quote! {
                if let ::cli_framework::spec::value::ArgValue::Int(i) = v { Some(*i as _) } else { None }
            }
        }
        Some("f64") | Some("f32") => {
            quote! {
                if let ::cli_framework::spec::value::ArgValue::Float(f) = v { Some(*f as _) } else { None }
            }
        }
        Some("PathBuf") => {
            quote! {
                if let ::cli_framework::spec::value::ArgValue::Str(s) | ::cli_framework::spec::value::ArgValue::Enum(s) = v {
                    Some(::std::path::PathBuf::from(s))
                } else { None }
            }
        }
        _ => {
            quote! {
                if let ::cli_framework::spec::value::ArgValue::Str(s) | ::cli_framework::spec::value::ArgValue::Enum(s) = v {
                    Some(s.clone())
                } else { None }
            }
        }
    }
}
