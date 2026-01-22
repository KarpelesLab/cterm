//! Image decoding for terminal graphics
//!
//! Decodes PNG, JPEG, and GIF images to RGBA pixel data suitable for display
//! in the terminal.

use image::GenericImageView;
use std::io::Cursor;
use thiserror::Error;

/// Error type for image decoding failures
#[derive(Debug, Error)]
pub enum ImageDecodeError {
    #[error("Failed to guess image format")]
    UnknownFormat,
    #[error("Image decode error: {0}")]
    DecodeError(#[from] image::ImageError),
    #[error("Image too large: {0}x{1} pixels")]
    TooLarge(u32, u32),
}

/// Maximum image dimensions to prevent memory issues
const MAX_IMAGE_DIMENSION: u32 = 4096;

/// Decoded image data
#[derive(Debug)]
pub struct DecodedImage {
    /// RGBA pixel data (4 bytes per pixel)
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
}

/// Decode an image from raw bytes (PNG, JPEG, or GIF)
///
/// Returns RGBA pixel data and dimensions. For GIF, only the first frame is decoded.
pub fn decode_image(data: &[u8]) -> Result<DecodedImage, ImageDecodeError> {
    let reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| ImageDecodeError::UnknownFormat)?;

    let format = reader.format();
    log::debug!("Decoding image format: {:?}", format);

    let img = reader.decode()?;

    let (width, height) = img.dimensions();

    // Sanity check dimensions
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(ImageDecodeError::TooLarge(width, height));
    }

    // Convert to RGBA8
    let rgba = img.to_rgba8();
    let data = rgba.into_raw();

    log::debug!("Decoded image: {}x{} ({} bytes)", width, height, data.len());

    Ok(DecodedImage {
        data,
        width: width as usize,
        height: height as usize,
    })
}

/// Guess if data looks like an image based on magic bytes
pub fn looks_like_image(data: &[u8]) -> bool {
    if data.len() < 3 {
        return false;
    }

    // JPEG magic: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }

    if data.len() < 6 {
        return false;
    }

    // GIF magic: GIF87a or GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return true;
    }

    if data.len() < 8 {
        return false;
    }

    // PNG magic: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_image() {
        // PNG
        assert!(looks_like_image(&[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A
        ]));

        // JPEG
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]));

        // GIF
        assert!(looks_like_image(b"GIF89a"));
        assert!(looks_like_image(b"GIF87a"));

        // Not an image
        assert!(!looks_like_image(b"Hello, World!"));
        assert!(!looks_like_image(&[]));
    }

    #[test]
    fn test_looks_like_image_minimal() {
        // PNG signature only
        assert!(looks_like_image(&[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a
        ]));
    }

    #[test]
    fn test_decode_minimal_png() {
        // World's smallest valid PNG (1x1 transparent)
        // From: data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4AWOwBQAAPwA+Eq7IEAAAAABJRU5ErkJggg==
        let png_data = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAAAAAA6fptVAAAACklEQVR4AWOwBQAAPwA+Eq7IEAAAAABJRU5ErkJggg=="
        ).unwrap();

        let result = decode_image(&png_data);
        assert!(result.is_ok(), "Failed to decode minimal PNG: {:?}", result.err());

        let img = result.unwrap();
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
        assert_eq!(img.data.len(), 4); // 1x1 RGBA
    }
}
