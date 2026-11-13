//! The application icon, drawn in code so no image assets or decoders are
//! needed: a rounded slate square with an accent "drop-down" panel at the
//! top.

/// Returns `size`×`size` pixels as RGBA bytes.
pub fn rgba(size: u32) -> Vec<u8> {
    let s = size as f32;
    let radius = s * 0.22;
    let panel_bottom = s * 0.52;
    let margin = s * 0.14;
    let background = [0x26, 0x32, 0x38];
    let accent = [0x4f, 0xc3, 0xf7];
    let light = [0xec, 0xef, 0xf1];

    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let alpha = rounded_rect_coverage(px, py, s, radius);
            let mut color = background;
            let in_panel = py > margin * 0.6 && py < panel_bottom && px > margin && px < s - margin;
            if in_panel {
                color = accent;
            }
            // A prompt-like bar below the panel.
            let bar = py > panel_bottom + s * 0.12
                && py < panel_bottom + s * 0.2
                && px > margin
                && px < s * 0.55;
            if bar {
                color = light;
            }
            out.extend_from_slice(&[color[0], color[1], color[2], (alpha * 255.0) as u8]);
        }
    }
    out
}

/// Anti-aliased coverage of a rounded square of size `s` at pixel center.
fn rounded_rect_coverage(px: f32, py: f32, s: f32, r: f32) -> f32 {
    let cx = px.clamp(r, s - r);
    let cy = py.clamp(r, s - r);
    let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}
