/// Clipboard read for the context-menu Paste; egui only delivers paste
/// text on the paste key event.
pub fn read_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
