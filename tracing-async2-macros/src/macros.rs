#![allow(clippy::collapsible_match)]

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{FnArg, ItemFn, Pat, parse_macro_input};

pub fn build_layer_function(_args: TokenStream, item: TokenStream) -> TokenStream {
    //
    let mut inner_fn = parse_macro_input!(item as ItemFn);
    let outer_vis = inner_fn.vis.clone();
    let mut outer_sig = inner_fn.sig.clone();
    let mut inner_mut_arg0 = false;

    // Check that the function accepts two parameters
    if inner_fn.sig.inputs.len() != 2 {
        return token_stream_with_error(
            quote! { #outer_sig},
            syn::Error::new_spanned(
                inner_fn.sig,
                "function should take two parameters of which the second must be an `OwnedEvent`",
            ),
        );
    }

    // Modify OUTER function signature in the following ways:
    // - The function should NOT be async
    // - The return type should be `CallbackLayer`
    // - The first parameter should be owned and have matching mutability
    // - The second parameter should be `buffer_size: usize`
    outer_sig.asyncness = None;
    outer_sig.output =
        syn::parse2(quote! { -> CallbackLayer }).expect("Could not parse ReturnType");

    if let Some(arg) = outer_sig.inputs.get_mut(0) {
        let arg0 = arg.clone();
        if let FnArg::Typed(t) = arg0 {
            if let syn::Type::Reference(tr) = *t.ty {
                inner_mut_arg0 = tr.mutability.is_some();
                if let FnArg::Typed(t) = arg {
                    t.ty = tr.elem;
                    if let Pat::Ident(i) = t.pat.as_mut() {
                        i.mutability = tr.mutability;
                    }
                }
            }
        }
    }

    if let Some(arg) = outer_sig.inputs.get_mut(1) {
        *arg = syn::parse2::<FnArg>(quote! {buffer_size: usize}).expect("Could not parse FnArg")
    }

    // Modify INNER funccion signature in the following ways:
    // - Remove the visibility keyword if any
    // - Rename the function to "save"
    // - The first input should be borrowed (if not already)
    inner_fn.vis = syn::Visibility::Inherited;
    inner_fn.sig.ident = syn::Ident::new("save", Span::call_site());

    if let Some(arg) = inner_fn.sig.inputs.get_mut(0) {
        if let FnArg::Typed(t) = arg {
            if !matches!(*t.ty, syn::Type::Reference(_)) {
                let ty = &t.ty;
                *t.ty = syn::parse2(quote! { &#ty })
                    .expect("Could not change FnArg to reference in inner signature");
            }
        }
    }

    // We need the name of the first argument for the call to "save"
    let arg0_name: String = inner_fn
        .sig
        .inputs
        .first()
        .and_then(|arg| {
            if let FnArg::Typed(t) = arg {
                if let Pat::Ident(pat) = t.pat.as_ref() {
                    return Some(pat.ident.to_string());
                }
            }
            None
        })
        .unwrap_or_else(|| "self".into());

    let mutable = inner_mut_arg0.then(|| "mut ").unwrap_or_default();
    let arg0: syn::Expr =
        syn::parse_str(&format!("&{}{}", &mutable, &arg0_name)).expect("Could not parse &arg0");

    // We also need the function name as a string for error output
    let outer_fn_name: String = outer_sig.ident.to_string();

    quote! {
        #outer_vis #outer_sig {

            use tokio::{self, sync::mpsc};
            use tracing_async2::channel_layer;

            let (tx, mut rx) = mpsc::channel::<OwnedEvent>(64);

            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    if let Err(e) = save(#arg0, event).await {
                        eprintln!("{} error: {}", #outer_fn_name, e);
                    }
                }
            });

            #inner_fn

            channel_layer(tx)
        }
    }
    .into()
}

fn token_stream_with_error(mut tokens: proc_macro2::TokenStream, error: syn::Error) -> TokenStream {
    tokens.extend(error.into_compile_error());
    tokens.into()
}
