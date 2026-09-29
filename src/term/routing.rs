use eframe::egui::{Key, KeyboardShortcut, Modifiers};

/// Decision D1: switch to `Key::F12` if the Backtick probe fails.
pub const TERM_KEY: Key = Key::Backtick;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The unsaved-changes dialog owns the UI: nothing is routed.
    Modal,
    Terminal,
    Central,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    ToggleTerminal,
    Save,
    ToggleEdit,
    Open,
    Find,
    Print,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Quit,
    /// Consume and do nothing (macOS Cmd+Q / Cmd+zoom while the terminal owns keys).
    Swallow,
}

pub fn owner(modal: bool, term_visible: bool, term_focused: bool) -> Owner {
    if modal {
        Owner::Modal
    } else if term_visible && term_focused {
        Owner::Terminal
    } else {
        Owner::Central
    }
}

fn sc(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::COMMAND, key)
}

/// Shortcuts the app consumes this frame for `owner`, in priority order.
/// Anything not listed is left in egui's event queue for the widget.
pub fn table(owner: Owner, mac: bool) -> Vec<(KeyboardShortcut, Route)> {
    match owner {
        Owner::Modal => Vec::new(),
        Owner::Terminal => {
            let mut t = vec![(sc(TERM_KEY), Route::ToggleTerminal)];
            if mac {
                // Cmd+Q / Cmd+zoom must never quit or zoom while the terminal
                // is focused; the physical-Ctrl forms are not listed, so they
                // fall through to the widget, which sends their control bytes.
                for key in [Key::Q, Key::Plus, Key::Equals, Key::Minus, Key::Num0] {
                    t.push((sc(key), Route::Swallow));
                }
            }
            t
        }
        Owner::Central => {
            let mut t = vec![
                (sc(Key::S), Route::Save),
                (sc(Key::E), Route::ToggleEdit),
                (sc(Key::O), Route::Open),
                (sc(Key::F), Route::Find),
                (sc(Key::P), Route::Print),
                (sc(Key::Plus), Route::ZoomIn),
                (sc(Key::Equals), Route::ZoomIn),
                (sc(Key::Minus), Route::ZoomOut),
                (sc(Key::Num0), Route::ZoomReset),
                (sc(TERM_KEY), Route::ToggleTerminal),
            ];
            if mac {
                t.push((sc(Key::Q), Route::Quit));
            }
            t
        }
    }
}

/// macOS delivers the menu's Cmd+Q as a window close request, not a key
/// event. While the terminal is focused that request is cancelled so the
/// shell can receive the physical-Ctrl form instead.
pub fn cancel_close_for_cmd_q(term_focused: bool, mac: bool, cmd_q_down: bool) -> bool {
    term_focused && mac && cmd_q_down
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(owner: Owner, mac: bool) -> Vec<Route> {
        table(owner, mac).into_iter().map(|(_, r)| r).collect()
    }
    fn has(owner: Owner, mac: bool, key: Key, mods: Modifiers) -> Option<Route> {
        table(owner, mac)
            .into_iter()
            .find(|(sc, _)| sc.logical_key == key && sc.modifiers == mods)
            .map(|(_, r)| r)
    }

    #[test]
    fn owner_precedence_is_modal_then_terminal_then_central() {
        assert_eq!(owner(true, true, true), Owner::Modal);
        assert_eq!(owner(false, true, true), Owner::Terminal);
        assert_eq!(owner(false, true, false), Owner::Central);
        assert_eq!(owner(false, false, true), Owner::Central); // hidden pane never owns keys
    }

    #[test]
    fn modal_routes_nothing() {
        assert!(routes(Owner::Modal, false).is_empty());
        assert!(routes(Owner::Modal, true).is_empty());
    }

    #[test]
    fn central_owner_gets_every_app_shortcut_including_zoom() {
        for (key, want) in [
            (Key::S, Route::Save),
            (Key::E, Route::ToggleEdit),
            (Key::O, Route::Open),
            (Key::F, Route::Find),
            (Key::P, Route::Print),
            (Key::Plus, Route::ZoomIn),
            (Key::Equals, Route::ZoomIn),
            (Key::Minus, Route::ZoomOut),
            (Key::Num0, Route::ZoomReset),
            (TERM_KEY, Route::ToggleTerminal),
        ] {
            assert_eq!(
                has(Owner::Central, false, key, Modifiers::COMMAND),
                Some(want),
                "{key:?}"
            );
        }
        assert_eq!(
            has(Owner::Central, false, Key::Q, Modifiers::COMMAND),
            None,
            "no quit shortcut on Windows/Linux today"
        );
        assert_eq!(
            has(Owner::Central, true, Key::Q, Modifiers::COMMAND),
            Some(Route::Quit)
        );
    }

    #[test]
    fn terminal_owner_keeps_only_the_toggle_on_windows_linux() {
        assert_eq!(routes(Owner::Terminal, false), vec![Route::ToggleTerminal]);
        assert_eq!(
            has(Owner::Terminal, false, Key::S, Modifiers::COMMAND),
            None
        );
        assert_eq!(
            has(Owner::Terminal, false, Key::Plus, Modifiers::COMMAND),
            None
        );
    }

    #[test]
    fn terminal_owner_on_mac_swallows_cmd_q_and_cmd_zoom_but_not_ctrl_forms() {
        assert_eq!(
            has(Owner::Terminal, true, TERM_KEY, Modifiers::COMMAND),
            Some(Route::ToggleTerminal)
        );
        for key in [Key::Q, Key::Plus, Key::Equals, Key::Minus, Key::Num0] {
            assert_eq!(
                has(Owner::Terminal, true, key, Modifiers::COMMAND),
                Some(Route::Swallow),
                "{key:?}"
            );
            assert_eq!(
                has(Owner::Terminal, true, key, Modifiers::CTRL),
                None,
                "physical Ctrl+{key:?} must reach the shell"
            );
        }
        assert_eq!(
            has(Owner::Terminal, true, Key::S, Modifiers::COMMAND),
            None,
            "Cmd+S is offered to the widget too"
        );
    }

    #[test]
    fn os_level_cmd_q_close_is_cancelled_only_while_terminal_focused_on_mac() {
        assert!(cancel_close_for_cmd_q(true, true, true));
        assert!(!cancel_close_for_cmd_q(false, true, true));
        assert!(!cancel_close_for_cmd_q(true, false, true));
        assert!(!cancel_close_for_cmd_q(true, true, false));
    }
}
