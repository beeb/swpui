#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!(concat!("../", std::env!("CARGO_PKG_README")))]
pub mod app;
pub mod config;
pub mod hash;
pub mod path;
pub mod prelude;
pub mod preview;
pub mod replace;
pub mod search;
pub mod spinner;
pub mod types;
pub mod ui;
pub mod utils;
