//! Implementations and types to parse the `IdDst` macro input.

use as_variant::as_variant;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::meta::ParseNestedMeta;
use syn::parse_quote;
use syn::punctuated::Punctuated;

use super::IdDst;
use crate::util::NameSpace;

impl IdDst {
    /// Parse the given `IdDst` macro input.
    pub(super) fn parse(input: syn::ItemStruct) -> syn::Result<Self> {
        let mut id_dst_attrs = IdDstAttrs::default();
        let mut is_repr_transparent = false;

        for attr in &input.attrs {
            if attr.path().is_ident("palpo_id") {
                attr.parse_nested_meta(|meta| id_dst_attrs.try_merge(meta, attr))?;
            } else if !is_repr_transparent && attr.path().is_ident("repr") {
                is_repr_transparent = attr
                    .parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)?
                    .iter()
                    .any(|meta| meta.path().is_ident("transparent"));
            }
        }

        let IdDstAttrs { validate } = id_dst_attrs;

        // The generated conversions require the identifier to have the layout of str.
        // Active cfg_attr attributes have already been expanded by the compiler.
        if !is_repr_transparent {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "IDs must be `#[repr(transparent)]` over `str`",
            ));
        }

        if validate.is_none() && !input.generics.params.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "IDs without validation and with generics are not supported",
            ));
        }

        if input.generics.where_clause.is_some() {
            // So we don't have to insert #where_clause everywhere when it is always None in
            // practice.
            return Err(syn::Error::new(
                Span::call_site(),
                "where clauses on IDs are not supported",
            ));
        }

        let str_field_index = as_variant!(
            &input.fields,
            syn::Fields::Unnamed(syn::FieldsUnnamed { unnamed, .. }) => unnamed
        )
        .and_then(|unnamed| {
            let last_field = unnamed.last()?;
            let is_str = matches!(
                &last_field.ty,
                syn::Type::Path(syn::TypePath { qself: None, path, .. }) if path.is_ident("str")
            );
            is_str.then(|| unnamed.len() - 1)
        })
        .ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "Only tuple structs with a `str` as the last field are supported",
            )
        })?
        .into();

        let generics = input.generics;
        let (impl_generics, type_generics, _where_clause) = generics.split_for_impl();
        let impl_generics = quote! { #impl_generics };

        let ident = input.ident;
        let borrowed_type = parse_quote! { #ident #type_generics };
        let owned_ident = format_ident!("Owned{ident}");
        let owned_type = parse_quote! { #owned_ident #type_generics };
        let box_type = parse_quote! { ::std::boxed::Box<#borrowed_type> };
        let arc_type = parse_quote! { ::std::sync::Arc<#borrowed_type> };

        Ok(Self {
            ident,
            borrowed_type,
            owned_ident,
            owned_type,
            box_type,
            arc_type,
            generics,
            impl_generics,
            validate,
            str_field_index,
            palpo_core: NameSpace::palpo_core(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::IdDst;

    #[test]
    fn rejects_identifiers_without_transparent_layout() {
        for input in ["struct Id(str);", "#[repr(C)] struct Id(str);"] {
            let error = IdDst::parse(syn::parse_str(input).unwrap()).err().unwrap();
            assert_eq!(
                error.to_string(),
                "IDs must be `#[repr(transparent)]` over `str`"
            );
        }
    }

    #[test]
    fn rejects_identifiers_without_trailing_str() {
        for input in [
            "#[repr(transparent)] struct Id(String);",
            "#[repr(transparent)] struct Id([u8]);",
            "#[repr(transparent)] struct Id();",
            "#[repr(transparent)] struct Id { value: str }",
        ] {
            let error = IdDst::parse(syn::parse_str(input).unwrap()).err().unwrap();
            assert_eq!(
                error.to_string(),
                "Only tuple structs with a `str` as the last field are supported",
            );
        }
    }

    #[test]
    fn accepts_transparent_string_identifiers() {
        for input in [
            "#[repr(transparent)] struct Id(str);",
            "#[repr(transparent,)] struct Id(str);",
            "#[repr(transparent)] #[palpo_id(validate = validate)] struct Id<T>(std::marker::PhantomData<T>, str);",
        ] {
            assert!(
                IdDst::parse(syn::parse_str(input).unwrap()).is_ok(),
                "{input}"
            );
        }
    }
}

/// The parsed attributes of the [`IdDst`].
#[derive(Default)]
struct IdDstAttrs {
    /// The path to the function to use to validate the identifier.
    validate: Option<syn::Path>,
}

impl IdDstAttrs {
    /// Set the path to the function to use to validate the identifier.
    ///
    /// Returns an error if it is already set.
    fn set_validate(&mut self, validate: syn::Path, attr: &syn::Attribute) -> syn::Result<()> {
        if self.validate.is_some() {
            return Err(syn::Error::new_spanned(
                attr,
                "cannot have multiple values for `validate` attribute",
            ));
        }

        self.validate = Some(validate);
        Ok(())
    }

    /// Try to parse the given meta item and merge it into this `IdDstAttrs`.
    ///
    /// Returns an error if an unknown `palpo_id` attribute is encountered, or if an attribute
    /// that accepts a single value appears several times.
    fn try_merge(&mut self, meta: ParseNestedMeta<'_>, attr: &syn::Attribute) -> syn::Result<()> {
        if meta.path.is_ident("validate") {
            return self.set_validate(meta.value()?.parse()?, attr);
        }

        Err(meta.error("unsupported `palpo_id` attribute"))
    }
}
