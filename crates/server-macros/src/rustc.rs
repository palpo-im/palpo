use std::process::Command;
use std::str;

use proc_macro::TokenStream;
use quote::quote;

use crate::utils::get_crate_name;

pub(super) fn flags_capture(args: TokenStream) -> TokenStream {
    let Some(crate_name) = get_crate_name() else {
        return args;
    };

    let flag = std::env::args().collect::<Vec<_>>();
    let flag_len = flag.len();
    let ret = quote! {
        pub static RUSTC_FLAGS: [&str; #flag_len] = [#( #flag ),*];

        #[::ctor::ctor]
        fn _set_rustc_flags() {
            crate::info::rustc::FLAGS.lock().expect("locked").insert(#crate_name, &RUSTC_FLAGS);
        }

        // static strings have to be yanked on module unload
        #[::ctor::dtor]
        fn _unset_rustc_flags() {
            crate::info::rustc::FLAGS.lock().expect("locked").remove(#crate_name);
        }
    };

    ret.into()
}

pub(super) fn version(args: TokenStream) -> TokenStream {
    let Some(_) = get_crate_name() else {
        return args;
    };

    let rustc_path = std::env::args().next();
    let version = rustc_path
        .and_then(|rustc_path| Command::new(rustc_path).args(["-V"]).output().ok())
        .and_then(|output| {
            str::from_utf8(&output.stdout)
                .map(str::trim)
                .map(String::from)
                .ok()
        })
        .unwrap_or_default();

    let ret = quote! {
        static RUSTC_VERSION: &'static str = #version;
    };

    ret.into()
}
