pub mod types;
pub mod database;
pub mod database_rw;
pub mod config;
pub mod error;
pub mod stats;
pub mod transaction;
pub mod utils;
pub(crate) mod components;
pub(crate) mod engine;
pub mod facade;

// Backward compatibility type alias
pub use facade::SearchIndex;
pub type Database = SearchIndex;