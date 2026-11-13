//! Regenerates anyquake.ico from the icon drawing in src/icon.rs:
//!
//!   cargo run --example make-ico
//!
//! Writes uncompressed 32-bit images, so neither an image library nor
//! ImageMagick is needed.

#[path = "../../src/icon.rs"]
mod icon;

use std::path::Path;

// The sizes Windows asks for at 100–200 % display scaling.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() -> std::io::Result<()> {
    let images: Vec<Vec<u8>> = SIZES.iter().map(|&size| bitmap(size)).collect();

    let mut ico = Vec::new();
    // ICONDIR: reserved, type 1 (icon), number of images.
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (&size, image) in SIZES.iter().zip(&images) {
        // ICONDIRENTRY; a size of 256 is stored as 0.
        ico.push(size as u8);
        ico.push(size as u8);
        ico.push(0); // palette colors
        ico.push(0); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in &images {
        ico.extend_from_slice(image);
    }

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/windows/anyquake.ico");
    std::fs::write(&out, ico)?;
    println!("wrote {}", out.display());
    Ok(())
}

/// One icon image: BITMAPINFOHEADER, BGRA pixels bottom-up, AND mask.
fn bitmap(size: u32) -> Vec<u8> {
    let rgba = icon::rgba(size);
    // The AND mask has one bit per pixel, rows padded to 4 bytes. It is
    // ignored for 32-bit images (the alpha channel is used) but must exist.
    let mask_row = size.div_ceil(32) * 4;
    let pixels_len = size * size * 4;

    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // header size
    out.extend_from_slice(&(size as i32).to_le_bytes()); // width
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // height: image + mask
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(pixels_len + mask_row * size).to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution, palette

    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            out.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}
