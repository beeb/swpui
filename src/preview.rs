//! Background preview worker, on-demand cache, and rich preview data types.
//!
//! Search results carry only byte offsets and captures. The worker reads files lazily
//! to derive context lines, line content, and match column positions, with an LRU cache
//! that bounds resident memory.

pub mod cache;
pub mod data;
pub mod worker;

pub use worker::{PreviewCommand, PreviewRequest, PreviewResult, PreviewWorker, WantedSet};
