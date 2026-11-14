//! Renders the logos in `assets/` into every icon the app and its packages
//! use. Run from anywhere with `cargo icons` (see `.cargo/config.toml`).
//!
//! - `assets/anyquake-logo.svg` → `packaging/anyquake.svg` (Linux packages),
//!   `packaging/macos/AnyQuake.icns` (macOS app bundle),
//!   `packaging/windows/anyquake.ico` (Windows installer and shortcuts) and
//!   `src/icons/anyquake-<size>.rgba` (window and tray icons).
//! - `assets/anyquake-logo-white.svg` → `src/icons/anyquake-menubar.rgba`
//!   (macOS menu bar icon).
//!
//! The `.rgba` files are raw, non-premultiplied RGBA pixels, row by row, so
//! the app can embed them without an image decoder.

use resvg::{tiny_skia, usvg};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Sizes of the icons embedded in the app; `src/icon.rs` must match.
const APP_SIZES: [u32; 3] = [22, 32, 64];

/// tray-icon shows the macOS menu bar icon at the full menu bar height of
/// 22pt, so the canvas is 22pt at 2x for Retina displays and the logo is
/// shrunk inside it to the height of the system's menu bar glyphs.
const MENUBAR_SIZE: u32 = 44;
const MENUBAR_LOGO_HEIGHT: f32 = 30.0; // 15pt

/// (icns type, pixel size)
const ICNS_ENTRIES: [(&[u8; 4], u32); 11] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024), // 512@2x
    (b"ic11", 32),   // 16@2x
    (b"ic12", 64),   // 32@2x
    (b"ic13", 256),  // 128@2x
    (b"ic14", 512),  // 256@2x
];

/// The sizes Windows asks for at 100–200 % display scaling.
const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let colored_svg = root.join("assets/anyquake-logo.svg");
    let colored = load(&colored_svg)?;
    let white = load(&root.join("assets/anyquake-logo-white.svg"))?;

    copy(&colored_svg, &root.join("packaging/anyquake.svg"))?;

    let icons = root.join("src/icons");
    fs::create_dir_all(&icons)?;
    for size in APP_SIZES {
        write(
            &icons.join(format!("anyquake-{size}.rgba")),
            &rgba(&render(&colored, size)?),
        )?;
    }
    write(
        &icons.join("anyquake-menubar.rgba"),
        &rgba(&render_fitted(&white, MENUBAR_SIZE, MENUBAR_LOGO_HEIGHT)?),
    )?;

    write(
        &root.join("packaging/macos/AnyQuake.icns"),
        &icns(&colored)?,
    )?;
    write(
        &root.join("packaging/windows/anyquake.ico"),
        &ico(&colored)?,
    )?;
    Ok(())
}

fn load(path: &Path) -> Result<usvg::Tree> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(usvg::Tree::from_data(&data, &usvg::Options::default())?)
}

/// Renders the whole SVG canvas into a `size`×`size` pixmap.
fn render(tree: &usvg::Tree, size: u32) -> Result<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("invalid icon size")?;
    let svg = tree.size();
    let transform =
        tiny_skia::Transform::from_scale(size as f32 / svg.width(), size as f32 / svg.height());
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Renders only the drawn content of the SVG, ignoring the canvas margins,
/// `content_height` pixels tall and centered in a `size`×`size` pixmap.
fn render_fitted(tree: &usvg::Tree, size: u32, content_height: f32) -> Result<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("invalid icon size")?;
    let bounds = tree.root().abs_bounding_box().ok_or("SVG draws nothing")?;
    let scale = content_height / bounds.height();
    let center = size as f32 / 2.0;
    let transform = tiny_skia::Transform::from_translate(
        -(bounds.x() + bounds.width() / 2.0),
        -(bounds.y() + bounds.height() / 2.0),
    )
    .post_scale(scale, scale)
    .post_translate(center, center);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

fn rgba(pixmap: &tiny_skia::Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// An .icns file is a list of PNG images with a small header, so macOS's
/// iconutil isn't needed.
fn icns(tree: &usvg::Tree) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for (kind, size) in ICNS_ENTRIES {
        let png = render(tree, size)?.encode_png()?;
        body.extend_from_slice(kind);
        body.extend_from_slice(&(8 + png.len() as u32).to_be_bytes());
        body.extend_from_slice(&png);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(&(8 + body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// An .ico file with uncompressed 32-bit images, which every consumer
/// (Explorer, Inno Setup) understands.
fn ico(tree: &usvg::Tree) -> Result<Vec<u8>> {
    let images = ICO_SIZES
        .iter()
        .map(|&size| Ok(ico_bitmap(&rgba(&render(tree, size)?), size)))
        .collect::<Result<Vec<_>>>()?;

    let mut out = Vec::new();
    // ICONDIR: reserved, type 1 (icon), number of images.
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(ICO_SIZES.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * ICO_SIZES.len() as u32;
    for (&size, image) in ICO_SIZES.iter().zip(&images) {
        // ICONDIRENTRY; a size of 256 is stored as 0.
        out.push(size as u8);
        out.push(size as u8);
        out.push(0); // palette colors
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in &images {
        out.extend_from_slice(image);
    }
    Ok(out)
}

/// One .ico image: BITMAPINFOHEADER, BGRA pixels bottom-up, AND mask.
fn ico_bitmap(rgba: &[u8], size: u32) -> Vec<u8> {
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

fn write(path: &PathBuf, data: &[u8]) -> Result<()> {
    fs::write(path, data).map_err(|e| format!("{}: {e}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    fs::copy(from, to).map_err(|e| format!("{} → {}: {e}", from.display(), to.display()))?;
    println!("wrote {}", to.display());
    Ok(())
}
