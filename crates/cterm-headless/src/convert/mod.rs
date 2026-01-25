//! Conversion utilities between cterm-core and proto types

pub mod color;
pub mod events;
pub mod key;
pub mod screen;

pub use color::{color_to_proto, proto_to_color};
pub use events::event_to_proto;
pub use key::{key_to_proto, modifiers_to_proto, proto_to_key, proto_to_modifiers};
pub use screen::{
    attrs_to_proto, cell_to_proto, proto_to_attrs, row_to_proto, screen_to_proto, screen_to_text,
};
