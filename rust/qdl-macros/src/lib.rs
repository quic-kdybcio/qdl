// SPDX-License-Identifier: BSD-3-Clause
/*
 * Copyright (c) Qualcomm Technologies, Inc. and/or its subsidiaries.
 */
//! Attribute macros for the Rust side of qdl.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, parse_quote, Error, ItemFn, Visibility};

/// Export a function to C, in the spirit of the kernel's EXPORT_SYMBOL.
///
/// The function must be `pub` and is emitted with the C ABI under its own
/// name. The C prototype is generated into qdl_rs.h by build.rs, so C code
/// only needs to include that header.
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return error(Span::call_site(), "#[export] takes no arguments");
    }

    let mut func = parse_macro_input!(item as ItemFn);

    if !matches!(func.vis, Visibility::Public(_)) {
        return error(func.sig.ident.span(), "#[export] functions must be pub");
    }
    if !func.sig.generics.params.is_empty() {
        return error(
            func.sig.ident.span(),
            "#[export] functions cannot be generic",
        );
    }
    if let Some(abi) = &func.sig.abi {
        if abi.name.as_ref().is_some_and(|name| name.value() != "C") {
            return error(func.sig.ident.span(), "#[export] functions use the C ABI");
        }
    }

    func.sig.abi = Some(parse_quote!(extern "C"));
    func.attrs.push(parse_quote!(#[unsafe(no_mangle)]));

    quote!(#func).into()
}

fn error(span: Span, msg: &str) -> TokenStream {
    Error::new(span, msg).to_compile_error().into()
}
