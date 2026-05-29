// Copyright (C) 2026 xhdlphzr
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Extract public function/method signatures, doc comments, and derives.
//!
//! This tool scans all `.rs` files in `src/` of the `editor` crate,
//! collects public functions and methods (including those in `impl` blocks),
//! and writes their signatures, `#[derive(...)]` attributes, and doc comments
//! to `../src/api-prompt.txt` (relative to the `editor` crate root).
//!
//! # Usage
//! Run from the `editor` crate directory:
//! ```bash
//! cargo run --bin generate_prompt
//! ```

use std::fs;
use std::path::PathBuf;
use syn::{Attribute, ImplItem, ImplItemFn, Item, ItemFn, Visibility};
use walkdir::WalkDir;

/// Check whether the visibility is public.
///
/// # Arguments
/// * `vis` - The visibility modifier.
///
/// # Returns
/// `true` if the item is `pub`, otherwise `false`.
fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

/// Extract doc comments (`///`) from the attributes.
///
/// # Arguments
/// * `attrs` - Slice of attributes.
///
/// # Returns
/// A single string containing all doc comments joined by spaces.
fn extract_doc(attrs: &[Attribute]) -> String {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(lit) = &meta.value {
                    if let syn::Lit::Str(lit_str) = &lit.lit {
                        docs.push(lit_str.value());
                    }
                }
            }
        }
    }
    docs.join(" ")
}

/// Extract the `#[derive(...)]` attribute as a string.
///
/// # Arguments
/// * `attrs` - Slice of attributes.
///
/// # Returns
/// The full `#[derive(...)]` string if present, otherwise an empty string.
fn extract_derives(attrs: &[Attribute]) -> String {
    let mut derives = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("derive") {
            if let Ok(meta) = attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::token::Comma>::parse_terminated,
            ) {
                for path in meta {
                    derives.push(quote::quote! { #path }.to_string());
                }
            }
        }
    }
    if derives.is_empty() {
        String::new()
    } else {
        format!("#[derive({})]", derives.join(", "))
    }
}

/// Extract the signature of a free function.
///
/// # Arguments
/// * `f` - The function item.
///
/// # Returns
/// The signature as a string (e.g., `pub fn new() -> Self`).
fn extract_fn_sig(f: &ItemFn) -> String {
    let sig = &f.sig;
    let generics = &sig.generics;
    let inputs = &sig.inputs;
    let ret = &sig.output;
    let ident = &sig.ident;
    let gen_str = quote::quote! { #generics }.to_string();
    let inputs_str = quote::quote! { #inputs }.to_string();
    let ret_str = quote::quote! { #ret }.to_string();
    format!("pub fn {}{}({}){}", ident, gen_str, inputs_str, ret_str)
}

/// Extract the signature of a method inside an `impl` block.
///
/// # Arguments
/// * `m` - The method item.
///
/// # Returns
/// The signature as a string (e.g., `pub fn insert(&mut self, text: &str)`).
fn extract_method_sig(m: &ImplItemFn) -> String {
    let sig = &m.sig;
    let ident = &sig.ident;
    let generics = &sig.generics;
    let inputs = &sig.inputs;
    let ret = &sig.output;
    let gen_str = quote::quote! { #generics }.to_string();
    let inputs_str = quote::quote! { #inputs }.to_string();
    let ret_str = quote::quote! { #ret }.to_string();
    format!("pub fn {}{}({}){}", ident, gen_str, inputs_str, ret_str)
}

/// Main entry point. Scans all source files and writes the extracted API to the output file.
fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = crate_dir.join("src");
    let out_file = crate_dir
        .parent()
        .unwrap()
        .join("src")
        .join("api-prompt.txt");
    fs::create_dir_all(out_file.parent().unwrap()).unwrap();

    let mut output = Vec::new();

    for entry in WalkDir::new(src_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let content = fs::read_to_string(path).unwrap();
        let syntax = syn::parse_file(&content).unwrap();

        for item in syntax.items {
            match item {
                Item::Fn(f) if is_pub(&f.vis) => {
                    output.push(format!("SIG: {}", extract_fn_sig(&f)));
                    let derives = extract_derives(&f.attrs);
                    if !derives.is_empty() {
                        output.push(format!("DERIVES: {}", derives));
                    }
                    let doc = extract_doc(&f.attrs);
                    if !doc.is_empty() {
                        output.push(format!("DOC: {}", doc));
                    }
                    output.push(String::new());
                }
                Item::Impl(imp) if imp.trait_.is_none() => {
                    for inner in imp.items {
                        if let ImplItem::Fn(m) = inner {
                            if is_pub(&m.vis) {
                                output.push(format!("SIG: {}", extract_method_sig(&m)));
                                let derives = extract_derives(&m.attrs);
                                if !derives.is_empty() {
                                    output.push(format!("DERIVES: {}", derives));
                                }
                                let doc = extract_doc(&m.attrs);
                                if !doc.is_empty() {
                                    output.push(format!("DOC: {}", doc));
                                }
                                output.push(String::new());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let text = output.join("\n");
    let final_text = format!(
        "// Copyright (C) 2026 xhdlphzr\n// SPDX-License-Identifier: AGPL-3.0-or-later\n\n{}",
        text
    );
    fs::write(&out_file, final_text).unwrap();
    println!("API extracted to {}", out_file.display());
}
