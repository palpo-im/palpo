use proc_macro::{Span, TokenStream};
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use syn::{Error, Fields, Ident, ItemEnum, Meta, Variant};

use crate::{Result, utils::camel_to_snake_string};

pub(super) fn command_dispatch(item: ItemEnum, _args: &[Meta]) -> Result<TokenStream> {
    let name = &item.ident;
    let arm: Vec<TokenStream2> = item
        .variants
        .iter()
        .map(dispatch_arm)
        .collect::<Result<Vec<_>>>()?;
    let switch = quote! {
        #[allow(clippy::large_stack_frames)] //TODO: fixme
        pub(super) async fn process(
            command: #name,
            context: &crate::admin::Context<'_>
        ) -> AppResult<()> {
            use #name::*;
            #[allow(non_snake_case)]
            match command {
                #( #arm )*
            }
        }
    };

    Ok([item.into_token_stream(), switch]
        .into_iter()
        .collect::<TokenStream2>()
        .into())
}

fn dispatch_arm(v: &Variant) -> Result<TokenStream2> {
    let name = &v.ident;
    let target = camel_to_snake_string(&format!("{name}"));
    let handler = Ident::new(&target, Span::call_site().into());
    let res = match &v.fields {
        Fields::Named(fields) => {
            let field = fields.named.iter().filter_map(|f| f.ident.as_ref());
            let arg = field.clone();
            quote! {
                #name { #( #field ),* } => {
                    Box::pin(#handler(context, #( #arg ),*)).await
                },
            }
        }
        Fields::Unnamed(fields) => {
            let Some(ref field) = fields.unnamed.first() else {
                return Err(Error::new(
                    Span::call_site().into(),
                    "One unnamed field required",
                ));
            };
            quote! {
                #name ( #field ) => {
                    Box::pin(#handler::process(#field, context)).await
                }
            }
        }
        Fields::Unit => {
            quote! {
                #name => {
                    Box::pin(#handler(context)).await
                },
            }
        }
    };

    Ok(res)
}
