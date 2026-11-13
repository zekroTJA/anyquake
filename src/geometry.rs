//! Platform independent placement math.
//!
//! All placement is computed relative to the *current* geometry of a screen
//! at the moment the window is shown (or the screen layout changes), so
//! resolution changes and hot-plugged monitors are always respected.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

/// Where on the screen the window is placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Covers the whole screen; width and height are ignored.
    Fullscreen,
}

impl Anchor {
    pub const ALL: [Anchor; 10] = [
        Anchor::Top,
        Anchor::Bottom,
        Anchor::Left,
        Anchor::Right,
        Anchor::Center,
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomLeft,
        Anchor::BottomRight,
        Anchor::Fullscreen,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Anchor::Top => "Top",
            Anchor::Bottom => "Bottom",
            Anchor::Left => "Left",
            Anchor::Right => "Right",
            Anchor::Center => "Center",
            Anchor::TopLeft => "Top left",
            Anchor::TopRight => "Top right",
            Anchor::BottomLeft => "Bottom left",
            Anchor::BottomRight => "Bottom right",
            Anchor::Fullscreen => "Fullscreen",
        }
    }

    /// Identifier used in the config file and in the KWin script.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn id(self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::Bottom => "bottom",
            Anchor::Left => "left",
            Anchor::Right => "right",
            Anchor::Center => "center",
            Anchor::TopLeft => "top-left",
            Anchor::TopRight => "top-right",
            Anchor::BottomLeft => "bottom-left",
            Anchor::BottomRight => "bottom-right",
            Anchor::Fullscreen => "fullscreen",
        }
    }
}

/// Computes the window rectangle inside `area` (a screen or its work area).
///
/// `width_pct` / `height_pct` are percentages (clamped to 1..=100) of the
/// area's size.
pub fn place(area: Rect, anchor: Anchor, width_pct: f32, height_pct: f32) -> Rect {
    if anchor == Anchor::Fullscreen {
        return area;
    }

    let pct = |v: f32| v.clamp(1.0, 100.0) / 100.0;
    let w = ((area.w as f32 * pct(width_pct)).round() as i32).clamp(1, area.w.max(1));
    let h = ((area.h as f32 * pct(height_pct)).round() as i32).clamp(1, area.h.max(1));

    let left = area.x;
    let right = area.x + area.w - w;
    let hcenter = area.x + (area.w - w) / 2;
    let top = area.y;
    let bottom = area.y + area.h - h;
    let vcenter = area.y + (area.h - h) / 2;

    let (x, y) = match anchor {
        Anchor::Top => (hcenter, top),
        Anchor::Bottom => (hcenter, bottom),
        Anchor::Left => (left, vcenter),
        Anchor::Right => (right, vcenter),
        Anchor::Center => (hcenter, vcenter),
        Anchor::TopLeft => (left, top),
        Anchor::TopRight => (right, top),
        Anchor::BottomLeft => (left, bottom),
        Anchor::BottomRight => (right, bottom),
        Anchor::Fullscreen => unreachable!(),
    };

    Rect { x, y, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(100, 50, 1000, 800);

    #[test]
    fn top_full_width() {
        assert_eq!(
            place(AREA, Anchor::Top, 100.0, 50.0),
            Rect::new(100, 50, 1000, 400)
        );
    }

    #[test]
    fn bottom_right_quarter() {
        assert_eq!(
            place(AREA, Anchor::BottomRight, 50.0, 50.0),
            Rect::new(600, 450, 500, 400)
        );
    }

    #[test]
    fn center() {
        assert_eq!(
            place(AREA, Anchor::Center, 50.0, 50.0),
            Rect::new(350, 250, 500, 400)
        );
    }

    #[test]
    fn left_and_right_are_vertically_centered() {
        assert_eq!(
            place(AREA, Anchor::Left, 30.0, 100.0),
            Rect::new(100, 50, 300, 800)
        );
        assert_eq!(
            place(AREA, Anchor::Right, 30.0, 50.0),
            Rect::new(800, 250, 300, 400)
        );
    }

    #[test]
    fn fullscreen_ignores_percentages() {
        assert_eq!(place(AREA, Anchor::Fullscreen, 10.0, 10.0), AREA);
    }

    #[test]
    fn percentages_are_clamped() {
        assert_eq!(
            place(AREA, Anchor::TopLeft, 250.0, 0.0),
            Rect::new(100, 50, 1000, 8)
        );
    }
}
