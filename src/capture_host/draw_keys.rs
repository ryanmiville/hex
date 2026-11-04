//! Annotate's draw keys (Labs › Boxes and arrows, macOS), as tldraw and
//! Excalidraw have them. While a capture records, the draw shortcut
//! (`capture.draw_hotkey`, ⌥⇧A) switches between using the app and
//! drawing. While it draws, D, R, and A pick the pen, box, and arrow, and V
//! (or Escape, as before) goes back to the app. Anything else, and every key
//! while the app is in use, stays with the app; a letter with ⌘, ⌃, or ⌥
//! down is never taken. The session tap asks [`take`] on its own thread, so
//! a taken key reaches neither the app nor the listener.

use scrawl_capture::model::Tool;

/// What a key press does to drawing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawKey {
    /// The draw shortcut: drawing on with the picked tool, or off.
    Switch,
    /// Draw with this tool now.
    Tool(Tool),
    /// Back to the app.
    Leave,
}

/// What a press means, if anything: `recording` while a capture records,
/// `drawing` while it draws, `shortcut` when it is the draw shortcut's, and
/// `bare` when no ⌘, ⌃, or ⌥ is down; `typed` is the character it types.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn draw_key(
    recording: bool,
    drawing: bool,
    shortcut: bool,
    bare: bool,
    typed: &str,
) -> Option<DrawKey> {
    if !recording {
        return None;
    }
    if shortcut {
        return Some(DrawKey::Switch);
    }
    if !drawing || !bare {
        return None;
    }
    match typed.to_lowercase().as_str() {
        "d" => Some(DrawKey::Tool(Tool::Pen)),
        "r" => Some(DrawKey::Tool(Tool::Box)),
        "a" => Some(DrawKey::Tool(Tool::Arrow)),
        "v" => Some(DrawKey::Leave),
        _ => None,
    }
}

/// A key press on the session tap (`code` and CGEvent `flags`, typing
/// `typed`): true when it was a draw key, now done, so the app never gets
/// it. Nothing is taken while the draw shortcut isn't active
/// (`AppSettings::active_capture_draw_hotkey`).
#[cfg(target_os = "macos")]
pub fn take(code: u16, flags: u64, typed: &str) -> bool {
    let Some(shortcut) = crate::app_settings::runtime_hotkeys().capture_draw else {
        return false;
    };
    let held = super::modifiers(flags);
    let key = draw_key(
        super::wants_input(),
        super::drawing(),
        shortcut.matches_key_press(code, flags),
        !(held.command || held.control || held.option),
        typed,
    );
    match key {
        Some(DrawKey::Switch) => super::toggle_pen(),
        Some(DrawKey::Tool(tool)) => use_tool(tool),
        Some(DrawKey::Leave) => {
            super::exit_drawing();
        }
        None => return false,
    }
    true
}

/// Draws with `picked` from now on: drawing turns on, and a stroke under
/// way ends with the old tool.
pub fn use_tool(picked: Tool) {
    if super::tool() != picked {
        super::end_stroke();
    }
    *super::TOOL
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = picked;
    super::give_tool();
    super::set_pen(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_draw_shortcut_switches_only_while_a_capture_records() {
        assert_eq!(draw_key(false, false, true, false, "å"), None);
        assert_eq!(
            draw_key(true, false, true, false, "å"),
            Some(DrawKey::Switch)
        );
        assert_eq!(
            draw_key(true, true, true, false, "å"),
            Some(DrawKey::Switch)
        );
    }

    #[test]
    fn letters_pick_tools_only_while_drawing() {
        assert_eq!(
            draw_key(true, true, false, true, "d"),
            Some(DrawKey::Tool(Tool::Pen))
        );
        assert_eq!(
            draw_key(true, true, false, true, "R"),
            Some(DrawKey::Tool(Tool::Box))
        );
        assert_eq!(
            draw_key(true, true, false, true, "a"),
            Some(DrawKey::Tool(Tool::Arrow))
        );
        assert_eq!(draw_key(true, true, false, true, "v"), Some(DrawKey::Leave));
        // Using the app, every letter is the app's.
        assert_eq!(draw_key(true, false, false, true, "a"), None);
        assert_eq!(draw_key(false, false, false, true, "a"), None);
    }

    #[test]
    fn other_keys_and_letters_with_command_keys_stay_with_the_app() {
        assert_eq!(draw_key(true, true, false, true, "x"), None);
        assert_eq!(draw_key(true, true, false, true, ""), None);
        // ⌘A, ⌃R, ⌥D: the app's, even while drawing.
        assert_eq!(draw_key(true, true, false, false, "a"), None);
    }
}
