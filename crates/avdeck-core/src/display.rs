//! Where the device screen goes inside a tab, and how pointer positions map back to device
//! pixels. Pure geometry in egui points; `pixels_per_point` is the window's DPI scale.

/// Axis-aligned rectangle in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Display orientation of a frame, as `Image.format.rotation` (`SkinRotation`) reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    Portrait,
    /// Turned 90° counterclockwise: the natural top edge is on the left.
    Landscape,
    ReversePortrait,
    /// Turned 90° clockwise: the natural top edge is on the right.
    ReverseLandscape,
}

impl Rotation {
    /// `SkinRotation` value from the proto; unknown values count as portrait.
    pub fn from_proto(value: i32) -> Self {
        match value {
            1 => Self::Landscape,
            2 => Self::ReversePortrait,
            3 => Self::ReverseLandscape,
            _ => Self::Portrait,
        }
    }

    /// Quarter turns counterclockwise from portrait, 0..4.
    pub fn quarters(self) -> i32 {
        match self {
            Self::Portrait => 0,
            Self::Landscape => 1,
            Self::ReversePortrait => 2,
            Self::ReverseLandscape => 3,
        }
    }

    /// Direction, in natural device pixels, of moving down on the drawn (rotated) screen.
    pub fn screen_down(self) -> (i32, i32) {
        match self {
            Self::Portrait => (0, 1),
            Self::Landscape => (-1, 0),
            Self::ReversePortrait => (0, -1),
            Self::ReverseLandscape => (1, 0),
        }
    }

    pub fn sideways(self) -> bool {
        matches!(self, Self::Landscape | Self::ReverseLandscape)
    }
}

/// Largest rectangle with the frame's aspect ratio, centered in `area`. It never shows more than
/// one physical pixel per device pixel, and its edges sit on the physical pixel grid so the
/// texture is not blurred by a half-pixel offset.
pub fn fit(area: Rect, frame: (u32, u32), pixels_per_point: f32) -> Rect {
    let (frame_width, frame_height) = (frame.0.max(1) as f32, frame.1.max(1) as f32);
    let native = 1.0 / pixels_per_point;
    let scale = (area.width / frame_width)
        .min(area.height / frame_height)
        .min(native)
        .max(0.0);
    let snap = |value: f32| (value * pixels_per_point).round() / pixels_per_point;
    let width = snap(frame_width * scale);
    let height = snap(frame_height * scale);
    let x = snap(area.x + (area.width - width) / 2.0);
    let y = snap(area.y + (area.height - height) / 2.0);
    Rect::new(x, y, width, height)
}

/// Device pixel under `point`, in the display's natural (unrotated) orientation, where touches
/// are delivered. `screen` is where the frame is drawn and `natural` the panel size in its
/// natural orientation (`hw.lcd.width` x `hw.lcd.height`). `None` outside the screen.
pub fn to_device(
    point: (f32, f32),
    screen: Rect,
    natural: (u32, u32),
    rotation: Rotation,
) -> Option<(i32, i32)> {
    let u = (point.0 - screen.x) / screen.width;
    let v = (point.1 - screen.y) / screen.height;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    // Fractions across the natural display.
    let (x, y) = match rotation {
        Rotation::Portrait => (u, v),
        Rotation::Landscape => (1.0 - v, u),
        Rotation::ReversePortrait => (1.0 - u, 1.0 - v),
        Rotation::ReverseLandscape => (v, 1.0 - u),
    };
    let pixel = |fraction: f32, size: u32| {
        ((fraction * size as f32) as i32).clamp(0, size.saturating_sub(1) as i32)
    };
    Some((pixel(x, natural.0), pixel(y, natural.1)))
}

/// The window edge being dragged, as `WM_SIZING` reports it (`WMSZ_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    TopLeft,
    TopRight,
    Bottom,
    BottomLeft,
    BottomRight,
}

impl Edge {
    pub fn from_wmsz(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Left,
            2 => Self::Right,
            3 => Self::Top,
            4 => Self::TopLeft,
            5 => Self::TopRight,
            6 => Self::Bottom,
            7 => Self::BottomLeft,
            8 => Self::BottomRight,
            _ => return None,
        })
    }
}

/// Outer window rectangle `(left, top, right, bottom)` in physical pixels while the user drags
/// `edge`, corrected so the screen area keeps `aspect` (width / height) and the device fills it
/// with no bars. `extra` is everything around the screen area, horizontally and vertically:
/// borders, title bar, toolbar, tab bar. The edge opposite the drag stays put.
pub fn lock_aspect(
    edge: Edge,
    (left, top, right, bottom): (i32, i32, i32, i32),
    extra: (i32, i32),
    aspect: f32,
    min_height: i32,
) -> (i32, i32, i32, i32) {
    let content_width = (right - left - extra.0).max(1) as f32;
    let content_height = (bottom - top - extra.1).max(1) as f32;
    let width_for = |height: f32| (height * aspect).round() as i32 + extra.0;
    let height_for = |width: f32| (width / aspect).round() as i32 + extra.1;
    let width_driven = match edge {
        Edge::Left | Edge::Right => true,
        Edge::Top | Edge::Bottom => false,
        // Corners follow whichever side the cursor pulled further.
        _ => content_width / aspect >= content_height,
    };
    let (mut width, mut height) = if width_driven {
        (right - left, height_for(content_width))
    } else {
        (width_for(content_height), bottom - top)
    };
    // Never smaller than the minimum screen height, still at the aspect ratio.
    if height - extra.1 < min_height {
        height = min_height + extra.1;
        width = width_for(min_height as f32);
    }
    let moves_left = matches!(edge, Edge::Left | Edge::TopLeft | Edge::BottomLeft);
    let moves_top = matches!(edge, Edge::Top | Edge::TopLeft | Edge::TopRight);
    let (left, right) = if moves_left {
        (right - width, right)
    } else {
        (left, left + width)
    };
    let (top, bottom) = if moves_top {
        (bottom - height, bottom)
    } else {
        (top, top + height)
    };
    (left, top, right, bottom)
}

/// Smallest screen height, in points, for a device tab: tall enough for the toolbar beside it,
/// and wide enough (with the toolbar) that the window is at least `min_window_width` wide.
pub fn min_screen_height(
    toolbar_height: f32,
    toolbar_width: f32,
    min_window_width: f32,
    aspect: f32,
) -> f32 {
    let for_width = (min_window_width - toolbar_width).max(0.0) / aspect.max(0.01);
    toolbar_height.max(for_width).ceil()
}

/// Size in points of the screen area when a device first shows: its longer side is `fraction`
/// of the monitor height, whatever the orientation.
pub fn initial_screen_size(monitor_height: f32, fraction: f32, frame: (u32, u32)) -> (f32, f32) {
    let (width, height) = (frame.0.max(1) as f32, frame.1.max(1) as f32);
    let scale = monitor_height * fraction / width.max(height);
    (width * scale, height * scale)
}

/// Physical-pixel rectangle `(left, top, right, bottom)`, edges exclusive like Win32 `RECT`.
pub type Bounds = (i32, i32, i32, i32);

/// Top-left corner for a side window of `size` (visible pixels) docked next to `owner` (visible
/// bounds): to the right when it fits in the work area, else to the left, else on the side with
/// more room. It is top-aligned with the owner and kept inside `work`, overlapping the owner if
/// it has to.
pub fn dock_beside(owner: Bounds, size: (i32, i32), work: Bounds, gap: i32) -> (i32, i32) {
    let right_room = work.2 - owner.2 - gap;
    let left_room = owner.0 - gap - work.0;
    let x = if right_room >= size.0 || right_room >= left_room {
        owner.2 + gap
    } else {
        owner.0 - gap - size.0
    };
    let x = x.min(work.2 - size.0).max(work.0);
    let y = owner.1.min(work.3 - size.1).max(work.1);
    (x, y)
}

/// How far a window docked next to the owner moves when the owner goes from `before` to `after`:
/// it keeps its distance to the owner edge it sits next to, and to the owner's top.
pub fn follow(before: Bounds, after: Bounds, side: Bounds) -> (i32, i32) {
    let on_right = side.0 + side.2 >= before.0 + before.2;
    let dx = if on_right {
        after.2 - before.2
    } else {
        after.0 - before.0
    };
    (dx, after.1 - before.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIXEL_10_PRO: (u32, u32) = (1280, 2856);
    const PIXEL_TABLET: (u32, u32) = (1600, 2560);

    #[test]
    fn phone_fills_the_height_of_a_tab_on_the_150_percent_monitor() {
        // 2560x1600 at 150% is 1706.7x1066.7 points; a tab area below a 40 pt toolbar strip.
        let area = Rect::new(0.0, 40.0, 1000.0, 1000.0);
        let screen = fit(area, PIXEL_10_PRO, 1.5);
        assert_eq!(screen.height, 1000.0);
        // 1000 * 1280 / 2856 = 448.18 pt = 672.3 px, snapped to 672 px.
        assert_eq!(screen.width, 448.0);
        assert_eq!(screen.y, 40.0);
        assert_eq!(screen.x * 1.5, (screen.x * 1.5).round());
    }

    #[test]
    fn sideways_tablet_fills_the_width() {
        let area = Rect::new(0.0, 0.0, 1200.0, 900.0);
        let screen = fit(area, (PIXEL_TABLET.1, PIXEL_TABLET.0), 1.0);
        assert_eq!((screen.width, screen.height), (1200.0, 750.0));
        assert_eq!((screen.x, screen.y), (0.0, 75.0));
    }

    #[test]
    fn never_shows_more_than_one_physical_pixel_per_device_pixel() {
        let area = Rect::new(0.0, 0.0, 4000.0, 4000.0);
        // At 100% a 1280x2856 frame stays 1280x2856 points, centered.
        let screen = fit(area, PIXEL_10_PRO, 1.0);
        assert_eq!((screen.width, screen.height), (1280.0, 2856.0));
        assert_eq!((screen.x, screen.y), (1360.0, 572.0));
        // At 200% the same frame is 640x1428 points: still one device pixel per physical pixel.
        let screen = fit(area, PIXEL_10_PRO, 2.0);
        assert_eq!((screen.width, screen.height), (640.0, 1428.0));
    }

    #[test]
    fn empty_area_gives_an_empty_screen() {
        let screen = fit(Rect::new(10.0, 10.0, 0.0, 0.0), PIXEL_10_PRO, 1.5);
        assert_eq!((screen.width, screen.height), (0.0, 0.0));
    }

    /// Corners of the drawn frame, just inside, for a 100x200 natural display.
    fn corners(rotation: Rotation) -> [Option<(i32, i32)>; 4] {
        let screen = if rotation.sideways() {
            Rect::new(10.0, 20.0, 200.0, 100.0)
        } else {
            Rect::new(10.0, 20.0, 100.0, 200.0)
        };
        let (left, top) = (screen.x + 0.1, screen.y + 0.1);
        let (right, bottom) = (
            screen.x + screen.width - 0.1,
            screen.y + screen.height - 0.1,
        );
        [(left, top), (right, top), (right, bottom), (left, bottom)]
            .map(|point| to_device(point, screen, (100, 200), rotation))
    }

    #[test]
    fn maps_corners_for_every_rotation() {
        let (tl, tr, br, bl) = (Some((0, 0)), Some((99, 0)), Some((99, 199)), Some((0, 199)));
        // Drawn corners clockwise from top-left, as natural-orientation corners.
        assert_eq!(corners(Rotation::Portrait), [tl, tr, br, bl]);
        assert_eq!(corners(Rotation::Landscape), [tr, br, bl, tl]);
        assert_eq!(corners(Rotation::ReversePortrait), [br, bl, tl, tr]);
        assert_eq!(corners(Rotation::ReverseLandscape), [bl, tl, tr, br]);
    }

    #[test]
    fn scales_to_device_pixels_and_rejects_points_outside() {
        let screen = Rect::new(0.0, 40.0, 448.0, 1000.0);
        let center = to_device((224.0, 540.0), screen, PIXEL_10_PRO, Rotation::Portrait);
        assert_eq!(center, Some((640, 1428)));
        assert_eq!(
            to_device((-1.0, 540.0), screen, PIXEL_10_PRO, Rotation::Portrait),
            None
        );
        assert_eq!(
            to_device((224.0, 1040.0), screen, PIXEL_10_PRO, Rotation::Portrait),
            None
        );
    }

    #[test]
    fn screen_down_agrees_with_to_device() {
        for rotation in [
            Rotation::Portrait,
            Rotation::Landscape,
            Rotation::ReversePortrait,
            Rotation::ReverseLandscape,
        ] {
            let screen = if rotation.sideways() {
                Rect::new(0.0, 0.0, 200.0, 100.0)
            } else {
                Rect::new(0.0, 0.0, 100.0, 200.0)
            };
            let center = (screen.width / 2.0, screen.height / 2.0);
            let (x0, y0) = to_device(center, screen, (100, 200), rotation).unwrap();
            let below = (center.0, center.1 + 10.0);
            let (x1, y1) = to_device(below, screen, (100, 200), rotation).unwrap();
            let (dx, dy) = rotation.screen_down();
            assert_eq!(
                ((x1 - x0).signum(), (y1 - y0).signum()),
                (dx, dy),
                "{rotation:?}"
            );
        }
    }

    #[test]
    fn reads_proto_rotation_values() {
        assert_eq!(Rotation::from_proto(1), Rotation::Landscape);
        assert_eq!(Rotation::from_proto(3), Rotation::ReverseLandscape);
        assert_eq!(Rotation::from_proto(9), Rotation::Portrait);
    }

    #[test]
    fn dragging_the_right_edge_sets_the_height() {
        // Pixel 10 Pro at 150%: 16x39 px borders and title, 61 px toolbar, 48 px tab bar.
        let aspect = 1280.0 / 2856.0;
        let rect = lock_aspect(
            Edge::Right,
            (100, 50, 100 + 700, 50 + 1000),
            (16 + 61, 39 + 48),
            aspect,
            0,
        );
        let content = (rect.2 - rect.0 - 16 - 61, rect.3 - rect.1 - 39 - 48);
        assert_eq!(content.0, 623);
        assert_eq!(content.1, (623.0 / aspect).round() as i32);
        assert_eq!((rect.0, rect.1), (100, 50));
    }

    #[test]
    fn dragging_the_top_left_corner_keeps_the_bottom_right_corner() {
        let aspect = 0.5;
        // Pulled mostly upward: content 400 wide, 1000 tall -> height drives.
        let rect = lock_aspect(
            Edge::TopLeft,
            (0, 0, 400 + 10 + 50, 1000 + 30),
            (10 + 50, 30),
            aspect,
            0,
        );
        assert_eq!((rect.2, rect.3), (460, 1030));
        assert_eq!(rect.2 - rect.0, 500 + 10 + 50);
        assert_eq!(rect.3 - rect.1, 1030);
    }

    #[test]
    fn top_edge_sets_the_width_from_the_height() {
        let rect = lock_aspect(Edge::Top, (0, 100, 300, 700), (40, 0), 0.5, 0);
        assert_eq!(rect, (0, 100, 340, 700));
    }

    #[test]
    fn minimum_screens() {
        // Pixel 10 Pro: the window width decides (280 pt wide window, 44 pt toolbar).
        let phone = min_screen_height(415.0, 44.0, 280.0, 1280.0 / 2856.0);
        assert_eq!(phone, 527.0);
        // Pixel Tablet in landscape: the toolbar height decides.
        assert_eq!(min_screen_height(415.0, 44.0, 280.0, 1.6), 415.0);
    }

    #[test]
    fn stops_at_the_minimum_height_keeping_the_aspect_ratio() {
        // Dragging the bottom-right corner far up and left on a phone (aspect 0.5).
        let rect = lock_aspect(Edge::BottomRight, (100, 100, 150, 140), (40, 60), 0.5, 600);
        assert_eq!(rect, (100, 100, 100 + 300 + 40, 100 + 600 + 60));
        // Dragging the top edge down keeps the bottom where it was.
        let rect = lock_aspect(Edge::Top, (0, 900, 400, 1000), (0, 0), 0.5, 600);
        assert_eq!(rect, (0, 400, 300, 1000));
    }

    #[test]
    fn the_longer_side_starts_at_80_percent_of_the_monitor_height() {
        // 2560x1600 at 150% = 1066.7 points high.
        let (width, height) = initial_screen_size(1066.7, 0.8, PIXEL_10_PRO);
        assert!((height - 853.36).abs() < 0.01);
        assert!((width - 853.36 * 1280.0 / 2856.0).abs() < 0.01);
        // Turned sideways the same device is as long, now horizontally.
        let (width, height) = initial_screen_size(1066.7, 0.8, (2856, 1280));
        assert!((width - 853.36).abs() < 0.01);
        assert!((height - 853.36 * 1280.0 / 2856.0).abs() < 0.01);
    }

    // Primary monitor at 150%: work area below a 48 px taskbar on top.
    const WORK: Bounds = (0, 48, 2560, 1600);

    #[test]
    fn docks_to_the_right_when_there_is_room() {
        let owner = (100, 160, 731, 1440);
        assert_eq!(dock_beside(owner, (835, 646), WORK, 8), (739, 160));
    }

    #[test]
    fn docks_to_the_left_or_the_roomier_side_otherwise() {
        // Measured: Avdeck centered at 965..1596, Extended Controls 1252x969 fits neither side.
        let owner = (965, 160, 1596, 1440);
        let (x, y) = dock_beside(owner, (1252, 969), WORK, 8);
        assert_eq!(y, 160);
        // More room on the left (957 px) than on the right (956 px); clamped to the work area.
        assert_eq!(x, 0);
        let owner = (1700, 160, 2331, 1440);
        assert_eq!(dock_beside(owner, (835, 646), WORK, 8), (857, 160));
    }

    #[test]
    fn keeps_the_docked_window_inside_the_work_area_vertically() {
        let owner = (100, 1200, 731, 1590);
        assert_eq!(dock_beside(owner, (835, 646), WORK, 8).1, 1600 - 646);
    }

    #[test]
    fn follows_the_edge_it_is_docked_to() {
        let before = (100, 160, 731, 1440);
        let right_side = (739, 160, 1574, 806);
        let left_side = (0, 160, 92, 806);
        // Dragging the owner's left edge: a window on the right stays, one on the left follows.
        let after = (50, 160, 731, 1440);
        assert_eq!(follow(before, after, right_side), (0, 0));
        assert_eq!(follow(before, after, left_side), (-50, 0));
        // Moving the whole owner moves both.
        let moved = (130, 200, 761, 1480);
        assert_eq!(follow(before, moved, right_side), (30, 40));
    }
}
