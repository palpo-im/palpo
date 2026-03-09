//! Palpo Admin UI
//! 
//! Modern web administration interface for Palpo Matrix server.
//! Built with Dioxus and compiled to WebAssembly.

pub mod app;
pub mod components;
pub mod pages;
pub mod services;
pub mod utils;
pub mod hooks;
pub mod models;
pub mod middleware;

pub use app::App;