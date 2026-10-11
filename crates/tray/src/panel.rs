//! The quick-add panel: where it sits and what it loads. Pure, so the
//! geometry can be tested; `main.rs` owns the window and the webview.

use taskstaka_core::Context;

/// A monitor's work area in logical points (origin top-left, as tao reports it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where the panel goes, in logical points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub const WIDTH: f64 = 680.0;
pub const HEIGHT: f64 = 72.0;
/// Kept clear of the screen edges on small displays.
pub const MARGIN: f64 = 16.0;
/// Spotlight sits about a fifth of the way down; this is where the eye lands.
const TOP_FRACTION: f64 = 0.22;

/// Centred horizontally, a fifth of the way down the given screen.
pub fn geometry(s: Screen) -> Geometry {
    let width = WIDTH.min(s.width - 2.0 * MARGIN);
    Geometry {
        x: s.x + (s.width - width) / 2.0,
        y: s.y + s.height * TOP_FRACTION,
        width,
        height: HEIGHT,
    }
}

/// The page the panel loads: the bare quick page in panel mode.
pub fn url(base: &str, ctx: Option<Context>) -> String {
    match ctx {
        Some(c) => format!("{base}/quick?panel=1&context={c}"),
        None => format!("{base}/quick?panel=1"),
    }
}

/// What the page tells the panel over IPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipc {
    /// Esc, or the user is done: hide the panel.
    Close,
    /// A task was added; the page shows "added" and will send Close itself.
    Added,
}

pub fn ipc(msg: &str) -> Option<Ipc> {
    match msg.trim() {
        "close" => Some(Ipc::Close),
        "added" => Some(Ipc::Added),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_sits_centred_a_quarter_down_like_spotlight() {
        // A 1728x1117 logical screen (14" MacBook Pro), origin at 0,0.
        let g = geometry(Screen {
            x: 0.0,
            y: 0.0,
            width: 1728.0,
            height: 1117.0,
        });
        assert_eq!((g.width, g.height), (680.0, 72.0));
        assert_eq!(g.x, (1728.0 - 680.0) / 2.0);
        assert!((g.y - 1117.0 * 0.22).abs() < 0.01, "{}", g.y);
    }

    #[test]
    fn panel_follows_the_screen_origin_on_a_second_display() {
        let g = geometry(Screen {
            x: -1920.0,
            y: 200.0,
            width: 1920.0,
            height: 1080.0,
        });
        assert_eq!(g.x, -1920.0 + (1920.0 - 680.0) / 2.0);
        assert_eq!(g.y, 200.0 + 1080.0 * 0.22);
    }

    #[test]
    fn panel_never_exceeds_a_small_screen() {
        let g = geometry(Screen {
            x: 0.0,
            y: 0.0,
            width: 600.0,
            height: 400.0,
        });
        assert!(g.width <= 600.0 - 2.0 * MARGIN);
        assert_eq!(g.x, MARGIN);
    }

    #[test]
    fn panel_url_marks_panel_mode_and_keeps_the_context() {
        assert_eq!(
            url("http://127.0.0.1:7407", None),
            "http://127.0.0.1:7407/quick?panel=1"
        );
        assert_eq!(
            url(
                "http://127.0.0.1:7407",
                Some(taskstaka_core::Context::Personal)
            ),
            "http://127.0.0.1:7407/quick?panel=1&context=personal"
        );
    }

    #[test]
    fn ipc_messages_are_the_two_the_page_sends() {
        assert_eq!(ipc("close"), Some(Ipc::Close));
        assert_eq!(ipc("added"), Some(Ipc::Added));
        assert_eq!(ipc("whatever"), None);
    }
}
