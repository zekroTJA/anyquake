//! The application icon, pre-rendered from `assets/*.svg` into raw RGBA
//! pixels by `cargo icons` (tools/icons), so no image decoder is needed.

#[derive(Clone, Copy)]
pub enum Variant {
    /// The colored logo.
    Colored,
    /// The white logo, smaller and with a transparent margin, for the macOS
    /// menu bar (see tools/icons).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MenuBar,
}

/// Returns the `size`×`size` icon as RGBA bytes. Only the sizes rendered by
/// tools/icons are available.
pub fn rgba(size: u32, variant: Variant) -> Vec<u8> {
    let data: &[u8] = match (variant, size) {
        (Variant::Colored, 22) => include_bytes!("icons/anyquake-22.rgba"),
        (Variant::Colored, 32) => include_bytes!("icons/anyquake-32.rgba"),
        (Variant::Colored, 64) => include_bytes!("icons/anyquake-64.rgba"),
        (Variant::MenuBar, 44) => include_bytes!("icons/anyquake-menubar.rgba"),
        _ => unreachable!("no {size}px icon rendered, see tools/icons"),
    };
    data.to_vec()
}
