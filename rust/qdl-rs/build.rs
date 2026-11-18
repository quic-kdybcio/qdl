// SPDX-License-Identifier: BSD-3-Clause
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
//! Generates the C header for the #[export] functions and #[repr(C)] types.
//!
//! cbindgen parses source text and does not expand attribute macros, so it
//! cannot see #[export] itself. Instead the exported signatures, together
//! with the types and constants they use, are copied into a stub file where
//! #[export] becomes the plain #[unsafe(no_mangle)] extern "C" form that
//! cbindgen understands. The stub is never compiled.
//!
//! The header is written to $QDL_RS_HEADER when it is set (meson does this)
//! and to OUT_DIR otherwise, so `cargo build` on its own still works.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use quote::ToTokens;
use syn::{parse_quote, Item, Visibility};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-env-changed=QDL_RS_HEADER");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let stub = out_dir.join("qdl_rs_stub.rs");
    fs::write(&stub, generate_stub(&manifest_dir.join("src"))).expect("failed to write stub");

    let header = env::var_os("QDL_RS_HEADER")
        .map(PathBuf::from)
        .unwrap_or_else(|| out_dir.join("qdl_rs.h"));
    let config = cbindgen::Config::from_file(manifest_dir.join("cbindgen.toml"))
        .expect("failed to read cbindgen.toml");

    cbindgen::Builder::new()
        .with_config(config)
        .with_src(&stub)
        .generate()
        .expect("failed to generate qdl_rs.h")
        .write_to_file(&header);
}

/// Collects every source file in @src_dir and returns a stub crate that
/// keeps only what cbindgen needs: imports, public constants, #[repr(C)]
/// types, and #[export] functions with empty bodies.
fn generate_stub(src_dir: &Path) -> String {
    let mut files: Vec<PathBuf> = fs::read_dir(src_dir)
        .expect("failed to read src directory")
        .map(|entry| entry.expect("failed to read src entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();

    let mut stub = String::new();
    for file in files {
        let source = fs::read_to_string(&file).expect("failed to read source file");
        let ast = syn::parse_file(&source)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));

        for item in ast.items {
            if let Some(text) = stub_item(item) {
                stub.push_str(&text);
                stub.push('\n');
            }
        }
    }

    stub
}

fn stub_item(item: Item) -> Option<String> {
    match item {
        Item::Use(_) => Some(item.to_token_stream().to_string()),
        Item::Const(ref c) if matches!(c.vis, Visibility::Public(_)) => {
            Some(item.to_token_stream().to_string())
        }
        Item::Struct(ref s)
            if has_attr(&s.attrs, "repr") && matches!(s.vis, Visibility::Public(_)) =>
        {
            Some(item.to_token_stream().to_string())
        }
        Item::Fn(mut func) if has_attr(&func.attrs, "export") => {
            func.attrs.retain(|attr| !attr.path().is_ident("export"));
            func.attrs.push(parse_quote!(#[unsafe(no_mangle)]));
            func.sig.abi = Some(parse_quote!(extern "C"));
            func.block = parse_quote!({});
            Some(func.to_token_stream().to_string())
        }
        _ => None,
    }
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}
