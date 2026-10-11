//! What a click means. Pure: the menu reports an id, this says what to do.

use taskstaka_core::Status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Open this URL in the default browser.
    Open(String),
    /// Show the quick-add panel.
    Panel,
    /// `POST /api/v1/commands` set_status.
    SetStatus {
        id: String,
        status: Status,
        cascade: bool,
    },
    Quit,
}

/// `base` is the daemon's URL (`http://127.0.0.1:7407`).
pub fn effect(id: &str, base: &str) -> Option<Effect> {
    let status = |id: &str, status, cascade| {
        (!id.is_empty()).then(|| Effect::SetStatus {
            id: id.to_string(),
            status,
            cascade,
        })
    };
    match id.split_once(':') {
        // Done cascades like the web's ✓, so a story closes with its subtree.
        Some(("done", t)) => status(t, Status::Done, true),
        Some(("putback", t)) => status(t, Status::Todo, false),
        Some(("start", t)) => status(t, Status::Doing, false),
        Some(("task", t)) if !t.is_empty() => Some(Effect::Open(format!("{base}/t/{t}"))),
        Some(_) => None,
        None => match id {
            "quick" => Some(Effect::Panel),
            "open" => Some(Effect::Open(format!("{base}/next"))),
            "quit" => Some(Effect::Quit),
            _ => None,
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use taskstaka_core::Status;

    const B: &str = "http://127.0.0.1:7407";

    #[test]
    fn plate_actions_become_status_changes() {
        assert_eq!(
            effect("done:abc234", B),
            Some(Effect::SetStatus {
                id: "abc234".into(),
                status: Status::Done,
                cascade: true
            })
        );
        assert_eq!(
            effect("putback:abc234", B),
            Some(Effect::SetStatus {
                id: "abc234".into(),
                status: Status::Todo,
                cascade: false
            })
        );
        assert_eq!(
            effect("start:abc234", B),
            Some(Effect::SetStatus {
                id: "abc234".into(),
                status: Status::Doing,
                cascade: false
            })
        );
    }

    #[test]
    fn navigation_opens_the_web_pages() {
        assert_eq!(
            effect("task:abc234", B),
            Some(Effect::Open(format!("{B}/t/abc234")))
        );
        assert_eq!(effect("quick", B), Some(Effect::Panel));
        assert_eq!(effect("open", B), Some(Effect::Open(format!("{B}/next"))));
    }

    #[test]
    fn quit_and_unknown() {
        assert_eq!(effect("quit", B), Some(Effect::Quit));
        assert_eq!(effect("label", B), None);
        assert_eq!(effect("done:", B), None);
    }
}
