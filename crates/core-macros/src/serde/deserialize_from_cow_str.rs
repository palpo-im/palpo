use proc_macro2::{Ident, TokenStream};
use quote::quote;

use crate::util::import_palpo_core;

pub fn expand_deserialize_from_cow_str(ident: &Ident) -> syn::Result<TokenStream> {
    let palpo_core = import_palpo_core();

    Ok(quote! {
        #[automatically_derived]
        impl<'de> #palpo_core::__private::serde::de::Deserialize<'de> for #ident {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: #palpo_core::__private::serde::de::Deserializer<'de>,
            {
                type CowStr<'a> = ::std::borrow::Cow<'a, ::std::primitive::str>;

                let cow = #palpo_core::serde::deserialize_cow_str(deserializer)?;
                Ok(::std::convert::From::<CowStr<'_>>::from(cow))
            }
        }
    })
}
