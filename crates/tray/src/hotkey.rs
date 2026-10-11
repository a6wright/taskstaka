//! The global hotkey: parsing the user's `alt+space`, and how to print it the
//! way the platform does (⌥Space on macOS).

use global_hotkey::hotkey::HotKey;

/// Parse a combo like `alt+space` or `ctrl+shift+k` (global-hotkey's grammar:
/// modifiers first, case-insensitive).
pub fn parse(combo: &str) -> Result<HotKey, String> {
    combo
        .parse::<HotKey>()
        .map_err(|e| format!("bad hotkey {combo:?}: {e}"))
}

/// The combo as the platform writes it in menus.
pub fn label(combo: &str) -> String {
    let parts: Vec<&str> = combo.split('+').map(str::trim).collect();
    let (mods, key) = parts.split_at(parts.len().saturating_sub(1));
    let key = key.first().copied().unwrap_or("");
    let key = key_name(key);
    if cfg!(target_os = "macos") {
        // Apple's order: ⌃ ⌥ ⇧ ⌘.
        let rank = |m: &str| match m {
            "ctrl" | "control" => 0,
            "alt" | "option" => 1,
            "shift" => 2,
            _ => 3,
        };
        let glyph = |m: &str| match m {
            "ctrl" | "control" => "⌃",
            "alt" | "option" => "⌥",
            "shift" => "⇧",
            "cmd" | "command" | "meta" | "super" | "cmdorctrl" | "commandorcontrol" => "⌘",
            _ => "",
        };
        let mut ms: Vec<String> = mods.iter().map(|m| m.to_ascii_lowercase()).collect();
        ms.sort_by_key(|m| rank(m));
        let mut out: String = ms.iter().map(|m| glyph(m)).collect();
        out.push_str(&key);
        out
    } else {
        let mut out: Vec<String> = mods
            .iter()
            .map(|m| match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmdorctrl" | "commandorcontrol" => "Ctrl".to_string(),
                "alt" | "option" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "cmd" | "command" | "meta" | "super" => "Super".to_string(),
                other => other.to_string(),
            })
            .collect();
        out.push(key);
        out.join("+")
    }
}

fn key_name(key: &str) -> String {
    let k = key
        .strip_prefix("Key")
        .or_else(|| key.strip_prefix("Digit"))
        .unwrap_or(key);
    let mut c = k.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_use_the_platform_glyphs() {
        if cfg!(target_os = "macos") {
            assert_eq!(label("alt+space"), "⌥Space");
            assert_eq!(label("ctrl+alt+space"), "⌃⌥Space");
            assert_eq!(label("cmd+shift+k"), "⇧⌘K");
            assert_eq!(label("super+T"), "⌘T");
        } else {
            assert_eq!(label("alt+space"), "Alt+Space");
            assert_eq!(label("ctrl+shift+k"), "Ctrl+Shift+K");
        }
    }

    #[test]
    fn a_bad_combo_is_reported_not_registered() {
        assert!(parse("alt+").is_err());
        assert!(parse("alt+space").is_ok());
    }
}
