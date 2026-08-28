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

// ============================================================================
// #[derive(ConfigManifest)] — spec 021 / ADR 0073
// ============================================================================
//
// Generates `impl IntoConfigManifest for StructName`, producing a
// `cli_framework::config::manifest::ConfigManifest` from the struct's field
// attributes. Mirrors `#[derive(CommandSpec)]` above in style: struct-level
// attribute for document-wide metadata, field-level attributes for
// per-field flags, `Lit`-based `parse_nested_meta` parsing throughout.
//
// The manifest is data, not a view of the Rust type (spec 021, "Manifest
// schema is data, not Rust types, at the consumption boundary") — this macro
// only *produces* a `ConfigManifest` value at runtime (via
// `IntoConfigManifest::config_manifest()`), exactly as `CommandSpec` above
// only produces a `CommandSpec` value. Nothing downstream is generic over
// the derived type.

/// Derive `IntoConfigManifest` for a struct.
///
/// Attribute vocabulary:
/// - `#[config_manifest(app = "...")]` — struct-level, required: the
///   application name stamped on the generated [`ConfigManifest`].
/// - `#[manifest(...)]` — field-level, all sub-keys optional:
///   - `key = "..."` — override the field's manifest key (default: the Rust
///     field name).
///   - `label = "..."`, `description = "..."`, `group = "..."`
///   - `scope = "machine" | "user" | "org"` (default: `machine`)
///   - `platforms = "desktop,mobile"` (comma-separated; empty/absent means
///     all platforms)
///   - `secret`, `local_only`, `protected`, `restart_required` — bare flags
///   - `manageable = false`, `enforceable = false` (both default `true`;
///     bare `manageable`/`enforceable` sets `true`, which is redundant with
///     the default but accepted)
///   - `kind = "duration" | "url" | "path" | "string" | "bool" | "integer" |
///     "float" | "enum"` — override the Rust-type-inferred
///     [`FieldKind`](../cli_framework/config/manifest/enum.FieldKind.html).
///     Needed for `duration` and `url` (no distinct Rust type exists for
///     either) and useful to force `enum` on a `String` field.
///   - `allowed = "a,b,c"` — for `kind = "enum"`, the enum's `values`; for
///     any numeric/string kind, populates `constraints.allowed_values`
///     instead.
///   - `min = <number>`, `max = <number>` — numeric range constraint.
///   - `section` — treat this field as a nested [`FieldKind::Section`]; the
///     field's Rust type must itself `#[derive(ConfigManifest)]`.
///
/// Rust-type inference (when `kind` is not overridden): `bool` -> `boolean`,
/// integer types -> `integer`, `f32`/`f64` -> `float`, `PathBuf` -> `path`,
/// `Vec<T>` -> `list` (item kind inferred from `T`), everything else ->
/// `string`.
#[proc_macro_derive(ConfigManifest, attributes(config_manifest, manifest))]
pub fn derive_config_manifest(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_config_manifest_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn derive_config_manifest_impl(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => &f.named,
            _ => {
                return Err(syn::Error::new(
                    Span::call_site(),
                    "ConfigManifest can only be derived on structs with named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new(
                Span::call_site(),
                "ConfigManifest can only be derived on structs",
            ))
        }
    };

    let mut app: Option<String> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("config_manifest") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("app") {
                    let value: Lit = meta.value()?.parse()?;
                    if let Lit::Str(s) = value {
                        app = Some(s.value());
                    }
                }
                Ok(())
            });
        }
    }
    let app = app.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "#[derive(ConfigManifest)] requires #[config_manifest(app = \"...\")]",
        )
    })?;

    let mut field_exprs = Vec::new();

    for field in fields {
        field_exprs.push(config_manifest_field_expr(field)?);
    }

    let expanded = quote! {
        impl ::cli_framework::config::manifest::IntoConfigManifest for #name {
            fn config_manifest() -> ::cli_framework::config::manifest::ConfigManifest {
                let __default = <#name as ::std::default::Default>::default();
                ::cli_framework::config::manifest::ConfigManifest {
                    manifest_schema_version: ::cli_framework::config::manifest::MANIFEST_SCHEMA_VERSION,
                    app: #app.to_string(),
                    fields: vec![ #(#field_exprs),* ],
                }
            }
        }
    };

    Ok(expanded)
}

/// Parsed `#[manifest(...)]` attributes for one field, before being turned
/// into a `FieldManifest { .. }` construction expression.
struct ManifestFieldAttrs {
    key: Option<String>,
    label: Option<String>,
    description: Option<String>,
    group: Option<String>,
    scope: String,
    platforms: Vec<String>,
    secret: bool,
    local_only: bool,
    protected: bool,
    manageable: bool,
    enforceable: bool,
    restart_required: bool,
    kind_override: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
    allowed: Vec<String>,
    section: bool,
}

impl Default for ManifestFieldAttrs {
    fn default() -> Self {
        Self {
            key: None,
            label: None,
            description: None,
            group: None,
            scope: "machine".to_string(),
            platforms: vec![],
            secret: false,
            local_only: false,
            protected: false,
            manageable: true,
            enforceable: true,
            restart_required: false,
            kind_override: None,
            min: None,
            max: None,
            allowed: vec![],
            section: false,
        }
    }
}

fn parse_manifest_field_attrs(field: &syn::Field) -> ManifestFieldAttrs {
    let mut attrs = ManifestFieldAttrs::default();

    for attr in &field.attrs {
        if !attr.path().is_ident("manifest") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            let bare_bool_flag =
                |meta: &syn::meta::ParseNestedMeta<'_>| -> syn::Result<Option<bool>> {
                    if meta.input.peek(syn::Token![=]) {
                        let value: Lit = meta.value()?.parse()?;
                        Ok(match value {
                            Lit::Bool(b) => Some(b.value),
                            _ => None,
                        })
                    } else {
                        Ok(Some(true))
                    }
                };

            if meta.path.is_ident("key") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.key = Some(s.value());
                }
            } else if meta.path.is_ident("label") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.label = Some(s.value());
                }
            } else if meta.path.is_ident("description") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.description = Some(s.value());
                }
            } else if meta.path.is_ident("group") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.group = Some(s.value());
                }
            } else if meta.path.is_ident("scope") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.scope = s.value();
                }
            } else if meta.path.is_ident("platforms") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.platforms = split_csv(&s.value());
                }
            } else if meta.path.is_ident("secret") {
                attrs.secret = true;
            } else if meta.path.is_ident("local_only") {
                attrs.local_only = true;
            } else if meta.path.is_ident("protected") {
                attrs.protected = true;
            } else if meta.path.is_ident("restart_required") {
                attrs.restart_required = true;
            } else if meta.path.is_ident("section") {
                attrs.section = true;
            } else if meta.path.is_ident("manageable") {
                if let Some(v) = bare_bool_flag(&meta)? {
                    attrs.manageable = v;
                }
            } else if meta.path.is_ident("enforceable") {
                if let Some(v) = bare_bool_flag(&meta)? {
                    attrs.enforceable = v;
                }
            } else if meta.path.is_ident("kind") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.kind_override = Some(s.value());
                }
            } else if meta.path.is_ident("min") {
                attrs.min = lit_to_f64(&meta.value()?.parse()?);
            } else if meta.path.is_ident("max") {
                attrs.max = lit_to_f64(&meta.value()?.parse()?);
            } else if meta.path.is_ident("allowed") {
                if let Lit::Str(s) = meta.value()?.parse()? {
                    attrs.allowed = split_csv(&s.value());
                }
            }
            Ok(())
        });
    }

    attrs
}

fn split_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

fn lit_to_f64(lit: &Lit) -> Option<f64> {
    match lit {
        Lit::Float(f) => f.base10_parse::<f64>().ok(),
        Lit::Int(i) => i.base10_parse::<f64>().ok(),
        _ => None,
    }
}

fn opt_str_tokens(v: &Option<String>) -> proc_macro2::TokenStream {
    match v {
        Some(s) => quote! { Some(#s.to_string()) },
        None => quote! { None },
    }
}

/// Build the `::cli_framework::config::manifest::FieldKind::...` tokens for
/// an override string from `#[manifest(kind = "...")]`. The `"enum"` case is
/// handled by the caller (it needs `allowed`, which this function doesn't
/// see) before this is reached.
fn manifest_kind_from_override(k: &str) -> syn::Result<proc_macro2::TokenStream> {
    Ok(match k {
        "duration" => quote! { ::cli_framework::config::manifest::FieldKind::Duration },
        "url" => quote! { ::cli_framework::config::manifest::FieldKind::Url },
        "path" => quote! { ::cli_framework::config::manifest::FieldKind::Path },
        "string" | "str" => quote! { ::cli_framework::config::manifest::FieldKind::Str },
        "bool" | "boolean" => quote! { ::cli_framework::config::manifest::FieldKind::Bool },
        "integer" | "int" => quote! { ::cli_framework::config::manifest::FieldKind::Int },
        "float" => quote! { ::cli_framework::config::manifest::FieldKind::Float },
        other => {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("unknown #[manifest(kind = \"{other}\")] override"),
            ))
        }
    })
}

/// Infer a `FieldKind` from a Rust type when no `kind` override is given.
/// Mirrors `infer_arg_kind_and_type`'s type-name matching above, but for the
/// manifest's (different) kind vocabulary.
fn infer_manifest_kind(ty: &Type) -> proc_macro2::TokenStream {
    match extract_type_name(ty).as_deref() {
        Some("bool") => quote! { ::cli_framework::config::manifest::FieldKind::Bool },
        Some("i64") | Some("i32") | Some("i16") | Some("i8") | Some("u64") | Some("u32")
        | Some("u16") | Some("u8") | Some("usize") | Some("isize") => {
            quote! { ::cli_framework::config::manifest::FieldKind::Int }
        }
        Some("f64") | Some("f32") => quote! { ::cli_framework::config::manifest::FieldKind::Float },
        Some("PathBuf") => quote! { ::cli_framework::config::manifest::FieldKind::Path },
        _ => quote! { ::cli_framework::config::manifest::FieldKind::Str },
    }
}

fn config_manifest_field_expr(field: &syn::Field) -> syn::Result<proc_macro2::TokenStream> {
    let field_name = field.ident.as_ref().unwrap();
    let field_name_str = field_name.to_string();
    let attrs = parse_manifest_field_attrs(field);

    let key = attrs.key.clone().unwrap_or(field_name_str);
    let (_is_option, is_vec, inner_ty) = unwrap_option_or_vec(&field.ty);
    let is_enum_kind = attrs.kind_override.as_deref() == Some("enum");

    let kind_tokens = if attrs.section {
        quote! {
            ::cli_framework::config::manifest::FieldKind::Section {
                fields: <#inner_ty as ::cli_framework::config::manifest::IntoConfigManifest>::config_manifest().fields,
            }
        }
    } else if is_enum_kind {
        let values = &attrs.allowed;
        quote! {
            ::cli_framework::config::manifest::FieldKind::Enum {
                values: vec![ #(#values.to_string()),* ],
            }
        }
    } else if let Some(k) = &attrs.kind_override {
        manifest_kind_from_override(k)?
    } else if is_vec {
        let item_kind = infer_manifest_kind(inner_ty);
        quote! { ::cli_framework::config::manifest::FieldKind::List { item: Box::new(#item_kind) } }
    } else {
        infer_manifest_kind(inner_ty)
    };

    let scope_tokens = match attrs.scope.as_str() {
        "user" => quote! { ::cli_framework::config::manifest::Scope::User },
        "org" => quote! { ::cli_framework::config::manifest::Scope::Org },
        _ => quote! { ::cli_framework::config::manifest::Scope::Machine },
    };

    let label_tokens = opt_str_tokens(&attrs.label);
    let description_tokens = opt_str_tokens(&attrs.description);
    let group_tokens = opt_str_tokens(&attrs.group);
    let platforms = &attrs.platforms;
    let platforms_tokens = quote! { vec![ #(#platforms.to_string()),* ] };

    // `allowed` doubles as the enum's `values` (handled above) and, for any
    // other kind, a `constraints.allowed_values` restriction — never both at
    // once for the same field.
    let allowed_for_constraints = if is_enum_kind {
        &[][..]
    } else {
        &attrs.allowed[..]
    };
    let has_constraints =
        attrs.min.is_some() || attrs.max.is_some() || !allowed_for_constraints.is_empty();
    let constraints_tokens = if has_constraints {
        let min_tokens = match attrs.min {
            Some(m) => quote! { Some(#m) },
            None => quote! { None },
        };
        let max_tokens = match attrs.max {
            Some(m) => quote! { Some(#m) },
            None => quote! { None },
        };
        let allowed_tokens = if allowed_for_constraints.is_empty() {
            quote! { None }
        } else {
            quote! { Some(vec![ #(::cli_framework::__private::serde_json::Value::String(#allowed_for_constraints.to_string())),* ]) }
        };
        quote! {
            Some(::cli_framework::config::manifest::FieldConstraints {
                min: #min_tokens,
                max: #max_tokens,
                allowed_values: #allowed_tokens,
            })
        }
    } else {
        quote! { None }
    };

    let secret = attrs.secret;
    let local_only = attrs.local_only;
    let protected = attrs.protected;
    let manageable = attrs.manageable;
    let enforceable = attrs.enforceable;
    let restart_required = attrs.restart_required;

    let default_tokens = if attrs.section {
        quote! { None }
    } else {
        quote! { ::cli_framework::__private::serde_json::to_value(&__default.#field_name).ok() }
    };

    Ok(quote! {
        ::cli_framework::config::manifest::FieldManifest {
            key: #key.to_string(),
            kind: #kind_tokens,
            default: #default_tokens,
            label: #label_tokens,
            description: #description_tokens,
            group: #group_tokens,
            scope: #scope_tokens,
            platforms: #platforms_tokens,
            secret: #secret,
            local_only: #local_only,
            protected: #protected,
            manageable: #manageable,
            enforceable: #enforceable,
            restart_required: #restart_required,
            constraints: #constraints_tokens,
        }
    })
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

// ============================================================================
// Unit tests for the ConfigManifest derive's helper functions and its inner
// `derive_config_manifest_impl` — a plain `syn`/`proc_macro2` function, so it
// is callable directly here without any actual macro expansion. This is
// deliberate: coverage tooling (`cargo llvm-cov`) instruments the test
// *binary* it runs, and a `#[proc_macro_derive]` entry point only ever
// executes inside `rustc`'s own process while compiling a *different*
// crate's derive-macro invocation — that execution is invisible to a
// coverage run of this crate (or of `cli-framework`'s own test binaries,
// which is where `tests/unit/config_manifest.rs` exercises the derive
// end-to-end and functionally proves every flag combination, but without
// attributing coverage back to these source lines). Testing the inner
// `syn`-typed functions directly, as done below, is the only way to get a
// real, attributable coverage number for this logic; this same limitation
// pre-dates this slice and applies equally to `derive_impl` (the
// `CommandSpec` derive above), which has never had unit tests of its own for
// the same reason.
#[cfg(test)]
mod config_manifest_tests {
    use super::*;

    fn expand(src: &str) -> syn::Result<String> {
        let input: DeriveInput = syn::parse_str(src).expect("valid struct source");
        derive_config_manifest_impl(input).map(|ts| ts.to_string())
    }

    #[test]
    fn missing_app_attribute_is_a_compile_error() {
        let err = expand("struct S { field: bool }").unwrap_err();
        assert!(err.to_string().contains("config_manifest(app"));
    }

    #[test]
    fn rejects_tuple_structs() {
        let err = expand("struct S(bool);").unwrap_err();
        assert!(err.to_string().contains("named fields"));
    }

    #[test]
    fn rejects_enums() {
        let err = expand("enum E { A, B }").unwrap_err();
        assert!(err.to_string().contains("can only be derived on structs"));
    }

    #[test]
    fn every_primitive_kind_is_inferred_from_rust_type() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                a: bool,
                b: i64,
                c: f64,
                d: String,
                e: std::path::PathBuf,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("FieldKind :: Bool"));
        assert!(out.contains("FieldKind :: Int"));
        assert!(out.contains("FieldKind :: Float"));
        assert!(out.contains("FieldKind :: Str"));
        assert!(out.contains("FieldKind :: Path"));
    }

    #[test]
    fn vec_field_infers_list_of_its_item_kind() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S { tags: Vec<String> }
            "#,
        )
        .unwrap();
        assert!(out.contains("FieldKind :: List"));
        assert!(
            out.contains("Box :: new (:: cli_framework :: config :: manifest :: FieldKind :: Str)")
        );
    }

    #[test]
    fn kind_override_duration_url_and_plain_aliases() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(kind = "duration")]
                a: u64,
                #[manifest(kind = "url")]
                b: String,
                #[manifest(kind = "path")]
                c: String,
                #[manifest(kind = "string")]
                d: String,
                #[manifest(kind = "bool")]
                e: String,
                #[manifest(kind = "integer")]
                f: String,
                #[manifest(kind = "float")]
                g: String,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("FieldKind :: Duration"));
        assert!(out.contains("FieldKind :: Url"));
    }

    #[test]
    fn unknown_kind_override_is_a_compile_error() {
        let err = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(kind = "nonsense")]
                a: String,
            }
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }

    #[test]
    fn enum_kind_uses_allowed_as_values_not_as_constraints() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(kind = "enum", allowed = "a,b,c")]
                level: String,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("FieldKind :: Enum"));
        assert!(out.contains("\"a\" . to_string ()"));
        // Constraints must stay `None` for an enum kind — `allowed` was
        // consumed as the enum's `values`, not as a separate restriction.
        assert!(out.contains("constraints : None"));
    }

    #[test]
    fn allowed_on_a_non_enum_kind_becomes_a_constraint() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(allowed = "x,y")]
                a: String,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("allowed_values"));
    }

    #[test]
    fn min_and_max_populate_constraints() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(min = 1, max = 10.5)]
                a: i64,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("min : Some (1f64)") || out.contains("min : Some (1"));
        assert!(out.contains("10.5"));
    }

    #[test]
    fn every_bare_and_valued_flag_is_threaded_through() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(
                    key = "renamed",
                    label = "Label",
                    description = "Desc",
                    group = "Group",
                    scope = "org",
                    platforms = "desktop,mobile",
                    secret,
                    local_only,
                    protected,
                    manageable = false,
                    enforceable = false,
                    restart_required
                )]
                a: bool,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("\"renamed\""));
        assert!(out.contains("\"Label\""));
        assert!(out.contains("\"Desc\""));
        assert!(out.contains("\"Group\""));
        assert!(out.contains("Scope :: Org"));
        assert!(out.contains("\"desktop\""));
        assert!(out.contains("\"mobile\""));
        assert!(out.contains("secret : true"));
        assert!(out.contains("local_only : true"));
        assert!(out.contains("protected : true"));
        assert!(out.contains("manageable : false"));
        assert!(out.contains("enforceable : false"));
        assert!(out.contains("restart_required : true"));
    }

    #[test]
    fn bare_manageable_and_enforceable_default_to_true() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(manageable, enforceable)]
                a: bool,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("manageable : true"));
        assert!(out.contains("enforceable : true"));
    }

    #[test]
    fn manageable_with_a_non_bool_literal_is_ignored_and_default_true_wins() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(manageable = "yes")]
                a: bool,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("manageable : true"));
    }

    #[test]
    fn unrelated_field_attributes_are_skipped_without_disturbing_manifest_parsing() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[doc = "unrelated"]
                #[manifest(secret)]
                a: bool,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("secret : true"));
    }

    #[test]
    fn user_and_machine_scope_strings_map_correctly() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(scope = "user")]
                a: bool,
                #[manifest(scope = "machine")]
                b: bool,
                c: bool,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("Scope :: User"));
        // "machine" and the default (no `scope` attribute at all) both
        // compile to the same `Scope::Machine` token sequence.
        let machine_count = out.matches("Scope :: Machine").count();
        assert_eq!(
            machine_count, 2,
            "both b and c must resolve to Scope::Machine"
        );
    }

    #[test]
    fn section_field_recurses_into_the_nested_types_manifest() {
        let out = expand(
            r#"
            #[config_manifest(app = "myapp")]
            struct S {
                #[manifest(section)]
                network: Network,
            }
            "#,
        )
        .unwrap();
        assert!(out.contains("FieldKind :: Section"));
        assert!(out.contains("< Network as :: cli_framework :: config :: manifest :: IntoConfigManifest > :: config_manifest () . fields"));
        // Sections never carry a `default` of their own.
        assert!(out.contains("default : None"));
    }

    #[test]
    fn split_csv_trims_and_drops_empty_segments() {
        assert_eq!(split_csv("a, b ,,c"), vec!["a", "b", "c"]);
        assert_eq!(split_csv(""), Vec::<String>::new());
        assert_eq!(split_csv("solo"), vec!["solo"]);
    }

    #[test]
    fn lit_to_f64_handles_int_float_and_rejects_other_literals() {
        let float_lit: Lit = syn::parse_str("3.5").unwrap();
        assert_eq!(lit_to_f64(&float_lit), Some(3.5));
        let int_lit: Lit = syn::parse_str("7").unwrap();
        assert_eq!(lit_to_f64(&int_lit), Some(7.0));
        let str_lit: Lit = syn::parse_str("\"nope\"").unwrap();
        assert_eq!(lit_to_f64(&str_lit), None);
    }

    #[test]
    fn opt_str_tokens_some_and_none() {
        assert_eq!(
            opt_str_tokens(&Some("x".to_string())).to_string(),
            "Some (\"x\" . to_string ())"
        );
        assert_eq!(opt_str_tokens(&None).to_string(), "None");
    }

    #[test]
    fn manifest_kind_from_override_covers_every_alias_and_rejects_unknown() {
        for (input, expected_variant) in [
            ("duration", "Duration"),
            ("url", "Url"),
            ("path", "Path"),
            ("string", "Str"),
            ("str", "Str"),
            ("bool", "Bool"),
            ("boolean", "Bool"),
            ("integer", "Int"),
            ("int", "Int"),
            ("float", "Float"),
        ] {
            let ts = manifest_kind_from_override(input).unwrap();
            assert!(
                ts.to_string().contains(expected_variant),
                "input {input} should map to variant {expected_variant}, got {ts}"
            );
        }
        assert!(manifest_kind_from_override("nonsense").is_err());
    }

    #[test]
    fn infer_manifest_kind_matches_every_numeric_alias_and_falls_back_to_str() {
        for (ty_src, expected) in [
            ("bool", "Bool"),
            ("i8", "Int"),
            ("i16", "Int"),
            ("i32", "Int"),
            ("i64", "Int"),
            ("u8", "Int"),
            ("u16", "Int"),
            ("u32", "Int"),
            ("u64", "Int"),
            ("usize", "Int"),
            ("isize", "Int"),
            ("f32", "Float"),
            ("f64", "Float"),
            ("PathBuf", "Path"),
            ("String", "Str"),
            ("SomeCustomType", "Str"),
        ] {
            let ty: Type = syn::parse_str(ty_src).unwrap();
            let ts = infer_manifest_kind(&ty);
            assert!(
                ts.to_string().contains(expected),
                "{ty_src} should infer {expected}, got {ts}"
            );
        }
    }
}
