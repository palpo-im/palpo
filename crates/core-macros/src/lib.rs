//! Procedural macros used by palpo crates.
//!
//! See the documentation for the individual macros for usage details.
#![allow(unreachable_pub)]
// https://github.com/rust-lang/rust-clippy/issues/9029
#![allow(clippy::derive_partial_eq_without_eq)]

use identifiers::expand_id_dst;
use palpo_identifiers_validation::{
    device_key_id, event_id, mxc_uri, room_alias_id, room_id, room_version_id, server_name, user_id,
};
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, ItemEnum, ItemStruct, parse_macro_input};

mod events;
mod identifiers;
mod serde;
mod util;

use self::{
    events::{
        event::expand_event,
        event_content::expand_event_content,
        event_enum::{EventEnumInput, expand_event_enum},
        event_enum_from_event::expand_event_enum_from_event,
    },
    identifiers::IdentifierInput,
    serde::{
        as_str_as_ref_str::expand_as_str_as_ref_str,
        debug_as_ref_str::expand_debug_as_ref_str,
        deserialize_from_cow_str::expand_deserialize_from_cow_str,
        display_as_ref_str::expand_display_as_ref_str,
        enum_as_ref_str::expand_enum_as_ref_str,
        enum_from_string::expand_enum_from_string,
        eq_as_ref_str::expand_partial_eq_as_ref_str,
        ord_as_ref_str::{expand_ord_as_ref_str, expand_partial_ord_as_ref_str},
        serialize_as_ref_str::expand_serialize_as_ref_str,
    },
    util::import_palpo_core,
};

/// Generates an enum to represent the various Matrix event types.
///
/// This macro also implements the necessary traits for the type to serialize and deserialize
/// itself.
///
/// # Examples
///
/// ```ignore
/// # // HACK: This is "ignore" because of cyclical dependency drama.
/// use palpo_macros::event_enum;
///
/// event_enum! {
///     enum ToDevice {
///         "m.any.event",
///         "m.other.event",
///     }
///
///     enum State {
///         "m.more.events",
///         "m.different.event",
///     }
/// }
/// ```
/// (The enum name has to be a valid identifier for `<EventKind as Parse>::parse`)
///// TODO: Change above (`<EventKind as Parse>::parse`) to [] after fully qualified syntax is
///// supported:  https://github.com/rust-lang/rust/issues/74563
#[proc_macro]
pub fn event_enum(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as EventEnumInput);
    expand_event_enum(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generates traits implementations and types for an event content.
///
/// # Trait implementations
///
/// This macro implements the following traits for the type on which it is applied:
///
/// * `{kind}EventContent`
/// * `StaticEventContent`
/// * `StaticStateEventContent`, for the `State` kind.
///
/// # Generated types
///
/// It also generates type aliases and modified clones depending on the kind of event. To generate
/// the base name of those types, the macro simply removes `Content` from the name of the type,
/// which means that to apply this macro to a type, its name must always end with `Content`. And for
/// compatibility with the [`event_enum!`] macro, the name should actually end with `EventContent`.
///
/// Some kinds can generate a modified clone of the event content type. For instance, for an event
/// content type named `FooEventContent`:
///
/// * `RedactedFooEventContent`: the redacted form of the event content, for the `MessageLike` and
///   `State` kinds. It also generates the `RedactContent` implementation which applies the
///   redaction algorithm according to the Matrix specification.
///
///   The generated type implements `Redacted{Kind}EventContent`, `StaticEventContent`, `Serialize`
///   and `Deserialize`.
///
///   The generation only works if the type is a struct with named fields. To keep a field after
///   redaction, the `#[palpo_event(skip_redaction)]` attribute can be applied to that field.
///
///   To skip the generation of this type and trait to implement a custom redaction, or because it
///   is not a struct with named fields, the `#[palpo_event(custom_redacted)]` attribute can be used
///   on the container. The `RedactedFooEventContent` type must still exist and implement the same
///   traits, even if it is only a type alias, and the `RedactContent` trait must still be
///   implemented for those kinds.
/// * `PossiblyRedactedFooEventContent`: the form of the event content that is used when we don't
///   know whether a `State` event is redacted or not. It means that on this type any field that is
///   redacted must be optional, or it must have the `#[serde(default)]` attribute for
///   deserialization.
///
///   The generated type implements `PossiblyRedactedStateEventContent`, `StaticEventContent`,
///   `Serialize` and `Deserialize`.
///
///   The generation uses the rules as the redacted type, using the `#[palpo_event(skip_redaction)]`
///   attribute.
///
///   To skip the generation of this type to use a custom type, the
///   `#[palpo_event(custom_possibly_redacted)]` attribute can be used on the container. The
///   `PossiblyRedactedFooEventContent` type must still exist for the `State` kind and implement the
///   same traits, even if it is only a type alias.
///
/// Event content types of the `MessageLike` kind that use the `Relation` type also need a clone of
/// the event content without the `relates_to` field for use within relations, where nested
/// relations are not meant to be serialized by homeservers. This macro can generate a
/// `FooEventContentWithoutRelation` type if the `#[palpo_event(without_relation)]` attribute is
/// applied on the container. It also generates `From<FooEventContent> for
/// FooEventContentWithoutRelation` and `FooEventContentWithoutRelation::with_relation()`.
///
/// By default, the generated types get a `#[non_exhaustive]` attribute. This behavior can be
/// controlled by setting the `palpo_unstable_exhaustive_types` compile-time `cfg` setting as
/// `--cfg=palpo_unstable_exhaustive_types` using `RUSTFLAGS` or `.cargo/config.toml` (under
/// `[build]` -> `rustflags = ["..."]`). When that setting is activated, the attribute is not
/// applied so the types are exhaustive.
///
/// # Type aliases
///
/// All kinds generate at least one type alias for the full event format. For the same example type
/// named `FooEventContent`, the first type alias generated is `type FooEvent =
/// {Kind}Event<FooEventContent>`.
///
/// The only exception for this is if the type has the `GlobalAccountData + RoomAccountData` kinds,
/// it generates two type aliases with prefixes:
///
/// * `type GlobalFooEvent = GlobalAccountDataEvent<FooEventContent>`
/// * `type RoomFooEvent = RoomAccountDataEvent<FooEventContent>`
///
/// Some kinds generate more type aliases:
///
/// * `type SyncFooEvent = Sync{Kind}Event<FooEventContent>`: an event received via the `/sync` API,
///   for the `MessageLike`, `State` and `EphemeralRoom` kinds
/// * `type OriginalFooEvent = Original{Kind}Event<FooEventContent>`, a non-redacted event, for the
///   `MessageLike` and `State` kinds
/// * `type OriginalSyncFooEvent = OriginalSync{Kind}Event<FooEventContent>`, a non-redacted event
///   received via the `/sync` API, for the `MessageLike` and `State` kinds
/// * `type RedactedFooEvent = Redacted{Kind}Event<RedactedFooEventContent>`, a redacted event, for
///   the `MessageLike` and `State` kinds
/// * `type OriginalSyncFooEvent = RedactedSync{Kind}Event<RedactedFooEventContent>`, a redacted
///   event received via the `/sync` API, for the `MessageLike` and `State` kinds
/// * `type InitialFooEvent = InitialStateEvent<FooEventContent>`, an event sent during room
///   creation, for the `State` kind
/// * `type StrippedFooEvent = StrippedStateEvent<PossiblyRedactedFooEventContent>`, an event that
///   is in a room state preview when receiving an invite, for the `State` kind
///
/// You can use `cargo doc` to find out more details, its `--document-private-items` flag also lets
/// you generate documentation for binaries or private parts of a library.
///
/// # Syntax
///
/// The basic syntax for using this macro is:
///
/// ```ignore
/// #[derive(Clone, Debug, Deserialize, Serialize, EventContent)]
/// #[palpo_event(type = "m.foo_bar", kind = MessageLike)]
/// pub struct FooBarEventContent {
///     data: String,
/// }
/// ```
///
/// ## Container attributes
///
/// The following settings can be used on the container, with the `#[palpo_event(_)]` attribute.
/// `type` and `kind` are always required.
///
/// ### `type = "m.event_type"`
///
/// The `type` of the event according to the Matrix specification, always required. This is usually
/// a string with an `m.` prefix.
///
/// Types with an account data kind can also use the `.*` suffix, if the end of the type changes
/// dynamically. It must be associated with a field that has the `#[palpo_event(type_fragment)]`
/// attribute that will store the end of the event type. Those types have the
/// `StaticEventContent::IsPrefix` type set to `True`.
///
/// ### `kind = Kind`
///
/// The kind of the event, always required. It must be one of these values, which matches the
/// [`event_enum!`] macro:
///
/// * `MessageLike` - A message-like event sent in the timeline
/// * `State` - A state event sent in the timeline
/// * `GlobalAccountData` - Global config event
/// * `RoomAccountData` - Per-room config event
/// * `ToDevice` - Event sent directly to a device
/// * `EphemeralRoom` - Event that is not persistent in the room
///
/// It is possible to implement both account data kinds for the same type by using the syntax `kind
/// = GlobalAccountData + RoomAccountData`.
///
/// ### `alias = "m.event_type"`
///
/// An alternate `type` for the event, used during deserialization. It is usually used for
/// deserializing an event type using both its stable and unstable prefix.
///
/// ### `state_key = StringType`
///
/// The type of the state key of the event, required and only supported if the kind is `State`. This
/// type should be a string type like `String`, `EmptyStateKey` or an identifier type generated with
/// the `IdDst` macro.
///
/// ### `unsigned_type = UnsignedType`
///
/// A custom type to use for the `Unsigned` type of the `StaticStateEventContent` implementation if
/// the kind is `State`. Only necessary if the `StateUnsigned` type is not appropriate for this
/// type.
///
/// ### `custom_redacted`
///
/// If the kind requires a `Redacted{}EventContent` type and a `RedactContent` implementation and it
/// is not possible to generate them with the macro, setting this attribute prevents the macro from
/// trying to generate them. The type and trait must be implemented manually.
///
/// ### `custom_possibly_redacted`
///
/// If the kind requires a `PossiblyRedacted{}EventContent` type and it is not possible to generate
/// it with the macro, setting this attribute prevents the macro from trying to generate it. The
/// type must be implemented manually.
///
/// ### `without_relation`
///
/// If this is set, the macro will try to generate an `{}EventContentWithoutRelation` which is a
/// clone of the current type with the `relates_to` field removed.
///
/// ## Field attributes
///
/// The following settings can be used on the fields of a struct, with the `#[palpo_event(_)]`
/// attribute.
///
/// ### `skip_redaction`
///
/// If a `Redacted{}EventContent` type is generated by the macro, this field will be kept after
/// redaction.
///
/// ### `type_fragment`
///
/// If the event content's kind is account data and its type ends with the `.*`, this field is
/// required and will store the end of the event's type.
///
/// # Example
///
/// An example can be found in the docs at the root of `palpo_events`.
#[proc_macro_derive(EventContent, attributes(palpo_event))]
pub fn derive_event_content(input: TokenStream) -> TokenStream {
    let palpo_core = import_palpo_core();
    let input = parse_macro_input!(input as DeriveInput);

    expand_event_content(&input, &palpo_core)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generates implementations needed to serialize and deserialize Matrix events.
#[proc_macro_derive(Event, attributes(palpo_event))]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_event(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generates `From` implementations for event enums.
#[proc_macro_derive(EventEnumFromEvent)]
pub fn derive_from_event_to_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_event_enum_from_event(input).into()
}

/// Generate methods and trait impl's for DST identifier type.
///
/// This macro generates an `Owned*` wrapper type for the identifier type. This wrapper type is
/// variable, by default it'll use [`Box`], but it can be changed at compile time
/// by setting `--cfg=palpo_identifiers_storage=...` using `RUSTFLAGS` or `.cargo/config.toml` (under
/// `[build]` -> `rustflags = ["..."]`). Currently the only supported value is `Arc`, that uses
/// [`Arc`](std::sync::Arc) as a wrapper type.
///
/// This macro implements:
///
/// * Conversions to and from string types, `AsRef<[u8]>` and `AsRef<str>`, as well as `as_str()`
///   and `as_bytes()` methods. The borrowed type can be converted from a borrowed string without
///   allocation.
/// * Conversions to and from borrowed and owned type.
/// * `Deref`, `AsRef` and `Borrow` to the borrowed type for the owned type.
/// * `PartialEq` implementations for testing equality with string types and owned and borrowed
///   types.
///
/// # Attributes
///
/// * `#[palpo_api(validate = PATH)]`: the path to a function to validate the string during parsing
///   and deserialization. By default, the types implement `From` string types, when this is set
///   they implement `TryFrom`.
///
/// # Examples
///
/// ```ignore
/// # // HACK: This is "ignore" because of cyclical dependency drama.
/// use palpo_core_macros::IdDst;
///
/// #[derive(PartialEq, Eq, PartialOrd, Ord, Hash, IdDst)]
/// #[palpo_id(validate = palpo_identifiers_validation::user_id::validate)]
/// pub struct UserId(str);
/// ```
#[proc_macro_derive(IdDst, attributes(palpo_id))]
pub fn derive_id_dst(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);
    expand_id_dst(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compile-time checked `DeviceKeyId` construction.
#[proc_macro]
pub fn device_key_id(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(
        device_key_id::validate(&id.value()).is_ok(),
        "Invalid device key id"
    );

    let output = quote! {
        <&#dollar_crate::DeviceKeyId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `EventId` construction.
#[proc_macro]
pub fn event_id(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(event_id::validate(&id.value()).is_ok(), "Invalid event id");

    let output = quote! {
        <&#dollar_crate::EventId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `RoomAliasId` construction.
#[proc_macro]
pub fn room_alias_id(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(
        room_alias_id::validate(&id.value()).is_ok(),
        "Invalid room_alias_id"
    );

    let output = quote! {
        <&#dollar_crate::RoomAliasId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `RoomId` construction.
#[proc_macro]
pub fn room_id(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(room_id::validate(&id.value()).is_ok(), "Invalid room_id");

    let output = quote! {
        <&#dollar_crate::RoomId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `RoomVersionId` construction.
#[proc_macro]
pub fn room_version_id(input: TokenStream) -> TokenStream {
    let IdentifierInput {
        dollar_crate: _,
        id,
    } = parse_macro_input!(input as IdentifierInput);
    assert!(
        room_version_id::validate(&id.value()).is_ok(),
        "Invalid room_version_id"
    );

    let output = quote! {
        <palpo_core::RoomVersionId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `ServerName` construction.
#[proc_macro]
pub fn server_name(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(
        server_name::validate(&id.value()).is_ok(),
        "Invalid server_name"
    );

    let output = quote! {
        <&#dollar_crate::ServerName as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Compile-time checked `MxcUri` construction.
#[proc_macro]
pub fn mxc_uri(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(mxc_uri::validate(&id.value()).is_ok(), "Invalid mxc://");

    let output = quote! {
        <&#dollar_crate::MxcUri as ::std::convert::From<&str>>::from(#id)
    };

    output.into()
}

/// Compile-time checked `UserId` construction.
#[proc_macro]
pub fn user_id(input: TokenStream) -> TokenStream {
    let IdentifierInput { dollar_crate, id } = parse_macro_input!(input as IdentifierInput);
    assert!(user_id::validate(&id.value()).is_ok(), "Invalid user_id");

    let output = quote! {
        <&#dollar_crate::UserId as ::std::convert::TryFrom<&str>>::try_from(#id).unwrap()
    };

    output.into()
}

/// Derive the `AsRef<str>` trait for an enum.
#[proc_macro_derive(AsRefStr, attributes(palpo_enum))]
pub fn derive_enum_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemEnum);
    expand_enum_as_ref_str(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `From<T: AsRef<str> + Into<Box<str>>>` trait for an enum.
#[proc_macro_derive(FromString, attributes(palpo_enum))]
pub fn derive_enum_from_string(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemEnum);
    expand_enum_from_string(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

// FIXME: The following macros aren't actually interested in type details beyond name (and possibly
//        generics in the future). They probably shouldn't use `DeriveInput`.

/// Derive the `as_str()` method using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(AsStrAsRefStr, attributes(palpo_enum))]
pub fn derive_as_str_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_as_str_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `fmt::Display` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(DisplayAsRefStr)]
pub fn derive_display_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_display_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `fmt::Debug` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(DebugAsRefStr)]
pub fn derive_debug_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_debug_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `Serialize` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(SerializeAsRefStr)]
pub fn derive_serialize_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_serialize_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `Deserialize` trait using the `From<Cow<str>>` implementation of the type.
#[proc_macro_derive(DeserializeFromCowStr)]
pub fn derive_deserialize_from_cow_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_deserialize_from_cow_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `PartialOrd` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(PartialOrdAsRefStr)]
pub fn derive_partial_ord_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_partial_ord_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `Ord` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(OrdAsRefStr)]
pub fn derive_ord_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_ord_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the `PartialEq` trait using the `AsRef<str>` implementation of the type.
#[proc_macro_derive(PartialEqAsRefStr)]
pub fn derive_partial_eq_as_ref_str(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_partial_eq_as_ref_str(&input.ident)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Shorthand for the derives `AsRefStr`, `FromString`, `DisplayAsRefStr`, `DebugAsRefStr`,
/// `SerializeAsRefStr` and `DeserializeFromCowStr`.
#[proc_macro_derive(StringEnum, attributes(palpo_enum))]
pub fn derive_string_enum(input: TokenStream) -> TokenStream {
    fn expand_all(input: ItemEnum) -> syn::Result<proc_macro2::TokenStream> {
        let as_ref_str_impl = expand_enum_as_ref_str(&input)?;
        let from_string_impl = expand_enum_from_string(&input)?;
        let as_str_impl = expand_as_str_as_ref_str(&input.ident)?;
        let display_impl = expand_display_as_ref_str(&input.ident)?;
        let debug_impl = expand_debug_as_ref_str(&input.ident)?;
        let serialize_impl = expand_serialize_as_ref_str(&input.ident)?;
        let deserialize_impl = expand_deserialize_from_cow_str(&input.ident)?;

        Ok(quote! {
            #as_ref_str_impl
            #from_string_impl
            #as_str_impl
            #display_impl
            #debug_impl
            #serialize_impl
            #deserialize_impl
        })
    }

    let input = parse_macro_input!(input as ItemEnum);
    expand_all(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
