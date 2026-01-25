//! Color conversion between cterm-core and proto

use crate::proto;

/// Convert cterm_core::Color to proto::Color
pub fn color_to_proto(color: &cterm_core::Color) -> proto::Color {
    use proto::color::ColorType;

    let color_type = match color {
        cterm_core::Color::Default => Some(ColorType::Default(true)),
        cterm_core::Color::Ansi(ansi) => Some(ColorType::Ansi(*ansi as u32)),
        cterm_core::Color::Indexed(idx) => Some(ColorType::Indexed(*idx as u32)),
        cterm_core::Color::Rgb(rgb) => Some(ColorType::Rgb(proto::Rgb {
            r: rgb.r as u32,
            g: rgb.g as u32,
            b: rgb.b as u32,
        })),
    };

    proto::Color { color_type }
}

/// Convert proto::Color to cterm_core::Color
pub fn proto_to_color(color: &proto::Color) -> cterm_core::Color {
    use proto::color::ColorType;

    match &color.color_type {
        Some(ColorType::Default(_)) => cterm_core::Color::Default,
        Some(ColorType::Ansi(idx)) => {
            if let Some(ansi) = cterm_core::AnsiColor::from_index(*idx as u8) {
                cterm_core::Color::Ansi(ansi)
            } else {
                cterm_core::Color::Default
            }
        }
        Some(ColorType::Indexed(idx)) => cterm_core::Color::Indexed(*idx as u8),
        Some(ColorType::Rgb(rgb)) => {
            cterm_core::Color::Rgb(cterm_core::Rgb::new(rgb.r as u8, rgb.g as u8, rgb.b as u8))
        }
        None => cterm_core::Color::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_color_roundtrip() {
        let color = cterm_core::Color::Default;
        let proto = color_to_proto(&color);
        let back = proto_to_color(&proto);
        assert_eq!(color, back);
    }

    #[test]
    fn test_ansi_color_roundtrip() {
        let color = cterm_core::Color::Ansi(cterm_core::AnsiColor::Red);
        let proto = color_to_proto(&color);
        let back = proto_to_color(&proto);
        assert_eq!(color, back);
    }

    #[test]
    fn test_rgb_color_roundtrip() {
        let color = cterm_core::Color::rgb(128, 64, 255);
        let proto = color_to_proto(&color);
        let back = proto_to_color(&proto);
        assert_eq!(color, back);
    }
}
