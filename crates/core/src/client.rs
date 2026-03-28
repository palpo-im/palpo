pub mod account;
pub mod appservice;
pub mod backup;
pub mod dehydrated_device;
pub mod device;
pub mod directory;
pub mod discovery;
pub mod event;
pub mod filter;
pub mod http_header;
pub mod key;
pub mod media;
pub mod membership;
pub mod message;
pub mod presence;
pub mod profile;
pub mod push;
pub mod redact;
pub mod register;
pub mod relation;
pub mod room;
#[cfg(feature = "unstable-msc4143")]
pub mod rtc;
pub mod search;
pub mod admin;
#[doc(hidden)]
#[deprecated = "Renamed to `admin`."]
pub use admin as server;
pub mod session;
pub mod space;
pub mod state;
pub mod sync_events;
pub mod tag;
pub mod typing;
pub mod uiaa;
pub mod user;
pub mod user_directory;
pub mod voip;
