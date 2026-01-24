//! cterm-ui: UI abstraction layer
//!
//! This crate defines traits and types for the UI layer, allowing
//! different UI backends (GTK4, Qt, etc.) to implement the terminal
//! interface.

pub mod events;
pub mod theme;
pub mod traits;
pub mod utils;

pub use events::*;
pub use theme::*;
pub use traits::*;
pub use utils::*;
