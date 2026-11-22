#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod sys;
pub mod msvc;
pub mod base;
pub mod encoding;
pub mod errors;
pub mod offline;
pub mod link;
pub mod link_dev;
pub mod xbtp2_sys;
pub mod registry;
pub mod conn;
pub mod link_conn;
pub mod session;
pub mod adapter;
pub mod session_impl;
pub mod api;
pub mod handshake;
pub mod dispatcher;
pub mod dispatcher_com;
pub mod service_info;
pub mod start;
pub mod xbtp2_json;
pub mod xbtp2_ext;
#[cfg(feature = "core")]
pub mod core_api;

pub use sys::*;

#[cfg(test)]
mod export_table_tests;
#[cfg(test)]
mod dispatcher_tests;
#[cfg(test)]
mod start_tests;
