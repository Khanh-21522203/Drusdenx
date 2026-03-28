// Kept for backward compatibility — delegates to the new facade layer.
// All logic lives in `src/core/engine.rs` and `src/core/components.rs`.
// Import paths like `crate::core::database::Database` continue to work.
pub use crate::core::facade::SearchIndex as Database;
