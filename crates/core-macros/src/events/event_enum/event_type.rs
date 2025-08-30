use std::collections::BTreeMap;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use super::{EventEnumEntry, EventEnumInput, EventKind};

pub fn expand_event_type_enums(
    input: EventEnumInput,
    palpo_core: &TokenStream,
) -> syn::Result<TokenStream> {
    let mut entries_map: BTreeMap<EventKind, Vec<&Vec<EventEnumEntry>>> = BTreeMap::new();

    for event in &input.enums {
        if event.events.is_empty() {
            continue;
        }

        entries_map
            .entry(event.kind)
            .or_default()
            .push(&event.events);

        if event.kind.is_timeline() {
            entries_map
                .entry(EventKind::Timeline)
                .or_default()
                .push(&event.events);
        }
    }

    let mut res = TokenStream::new();

    for (kind, entries) in entries_map {
        res.extend(
            generate_enum(kind, &entries, palpo_core)
                .unwrap_or_else(syn::Error::into_compile_error),
        );
    }

    Ok(res)
}

fn generate_enum(
    kind: EventKind,
    entries: &[&Vec<EventEnumEntry>],
    palpo_core: &TokenStream,
) -> syn::Result<TokenStream> {
    let serde = quote! { #palpo_core::__private::serde };
    let enum_doc = format!("The type of `{kind}` this is.");
    let ident = format_ident!("{kind}Type");

    let mut deduped: Vec<&EventEnumEntry> = vec![];
    for item in entries.iter().copied().flatten() {
        if let Some(idx) = deduped
            .iter()
            .position(|e| e.types.ev_type == item.types.ev_type)
        {
            // If there is a variant without config attributes use that
            if deduped[idx].attrs != item.attrs && item.attrs.is_empty() {
                deduped[idx] = item;
            }
        } else {
            deduped.push(item);
        }
    }

    let event_types = deduped.iter().map(|e| &e.types.ev_type);

    let variants: Vec<_> = deduped
        .iter()
        .map(|e| {
            let start = e.to_variant().decl();
            let data = e
                .has_type_fragment()
                .then(|| quote! { (::std::string::String) });

            quote! {
                #start #data
            }
        })
        .collect();

    let to_cow_str_match_arms: Vec<_> = deduped
        .iter()
        .map(|e| {
            let v = e.to_variant();
            let start = v.match_arm(quote! { Self });
            let ev_type = &e.types.ev_type;

            if ev_type.is_prefix() {
                let fstr = ev_type.without_wildcard().to_owned() + "{}";
                quote! { #start(_s) => ::std::borrow::Cow::Owned(::std::format!(#fstr, _s)) }
            } else {
                quote! { #start => ::std::borrow::Cow::Borrowed(#ev_type) }
            }
        })
        .collect();

    let mut from_str_match_arms = TokenStream::new();
    for event in &deduped {
        let v = event.to_variant();
        let ctor = v.ctor(quote! { Self });
        let ev_types = event.types.iter();
        let attrs = &event.attrs;

        if event.has_type_fragment() {
            for ev_type in ev_types {
                let prefix = ev_type.without_wildcard();

                from_str_match_arms.extend(quote! {
                    #(#attrs)*
                    // Use if-let guard once available
                    _s if _s.starts_with(#prefix) => {
                        #ctor(::std::convert::From::from(_s.strip_prefix(#prefix).unwrap()))
                    }
                });
            }
        } else {
            from_str_match_arms.extend(quote! { #(#attrs)* #(#ev_types)|* => #ctor, });
        }
    }

    let from_ident_for_timeline = if kind.is_timeline() && !matches!(kind, EventKind::Timeline) {
        let match_arms = deduped.iter().map(|e| {
            let v = e.to_variant();
            let ident_var = v.match_arm(quote! { #ident });
            let timeline_var = v.ctor(quote! { Self });

            if e.has_type_fragment() {
                quote! { #ident_var (_s) => #timeline_var (_s) }
            } else {
                quote! { #ident_var => #timeline_var }
            }
        });

        Some(quote! {
            #[allow(deprecated)]
            impl ::std::convert::From<#ident> for TimelineEventType {
                fn from(s: #ident) -> Self {
                    match s {
                        #(#match_arms,)*
                        #ident ::_Custom(_s) => Self::_Custom(_s),
                    }
                }
            }
        })
    } else {
        None
    };

    Ok(quote! {
        #[doc = #enum_doc]
        ///
        /// This type can hold an arbitrary string. To build events with a custom type, convert it
        /// from a string with `::from()` / `.into()`. To check for events that are not available as a
        /// documented variant here, use its string representation, obtained through `.to_string()`.
        #[derive(salvo::oapi::ToSchema, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, diesel::deserialize::FromSqlRow, diesel::expression::AsExpression)]
        #[diesel(sql_type = diesel::sql_types::Text)]
        pub enum #ident {
            #(
                #[doc = #event_types]
                #variants,
            )*
            #[doc(hidden)]
            _Custom(crate::PrivOwnedStr),
        }

        impl #ident {
            fn to_cow_str(&self) -> ::std::borrow::Cow<'_, ::std::primitive::str> {
                match self {
                    #(#to_cow_str_match_arms,)*
                    Self::_Custom(crate::PrivOwnedStr(s)) => ::std::borrow::Cow::Borrowed(s),
                }
            }
        }

        // impl salvo::oapi::ToSchema for #ident {
        //     fn to_schema(components: &mut salvo::oapi::Components) -> salvo::oapi::RefOr<salvo::oapi::Schema> {
        //         String::to_schema(components)
        //     }
        // }

        impl ::std::fmt::Display for #ident {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                self.to_cow_str().fmt(f)
            }
        }

        impl ::std::fmt::Debug for #ident {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                <str as ::std::fmt::Debug>::fmt(&self.to_cow_str(), f)
            }
        }

        impl ::std::convert::From<&::std::primitive::str> for #ident {
            fn from(s: &::std::primitive::str) -> Self {
                match s {
                    #from_str_match_arms
                    _ => Self::_Custom(crate::PrivOwnedStr(::std::convert::From::from(s))),
                }
            }
        }

        impl ::std::convert::From<::std::string::String> for #ident {
            fn from(s: ::std::string::String) -> Self {
                ::std::convert::From::from(s.as_str())
            }
        }

        impl<'de> #serde::Deserialize<'de> for #ident {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: #serde::Deserializer<'de>
            {
                let s = palpo_core::serde::deserialize_cow_str(deserializer)?;
                Ok(::std::convert::From::from(&s[..]))
            }
        }

        impl #serde::Serialize for #ident {
            fn serialize<S>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error>
            where
                S: #serde::Serializer,
            {
                self.to_cow_str().serialize(serializer)
            }
        }

        impl diesel::deserialize::FromSql<diesel::sql_types::Text, diesel::pg::Pg> for #ident {
            fn from_sql(bytes: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
                let value = <String as diesel::deserialize::FromSql<diesel::sql_types::Text, diesel::pg::Pg>>::from_sql(bytes)?;
                Ok(Self::from(value))
            }
        }

        impl diesel::serialize::ToSql<diesel::sql_types::Text, diesel::pg::Pg> for #ident {
            fn to_sql(&self, out: &mut diesel::serialize::Output<'_, '_, diesel::pg::Pg>) -> diesel::serialize::Result {
                diesel::serialize::ToSql::<diesel::sql_types::Text, diesel::pg::Pg>::to_sql(self.to_cow_str().as_ref(), &mut out.reborrow())
            }
        }

        #from_ident_for_timeline
    })
}
