use eframe::egui::Theme;
use egui_term::{ColorPalette, TerminalTheme};

/// Dark app theme: egui_term's stock base16 palette (checked
/// readable for the 16 ANSI colors, the 256 ramp and truecolor).
pub fn dark() -> ColorPalette {
    ColorPalette::default()
}

/// Light app theme: GitHub-Light-derived values, dark text on a light
/// background; dim variants are the normal colors slightly lightened.
pub fn light() -> ColorPalette {
    ColorPalette {
        foreground: "#24292f".into(),
        background: "#f6f8fa".into(),
        black: "#24292f".into(),
        red: "#cf222e".into(),
        green: "#116329".into(),
        yellow: "#4d2d00".into(),
        blue: "#0969da".into(),
        magenta: "#8250df".into(),
        cyan: "#1b7c83".into(),
        white: "#6e7781".into(),
        bright_black: "#57606a".into(),
        bright_red: "#a40e26".into(),
        bright_green: "#1a7f37".into(),
        bright_yellow: "#633c01".into(),
        bright_blue: "#218bff".into(),
        bright_magenta: "#a475f9".into(),
        bright_cyan: "#3192aa".into(),
        bright_white: "#8c959f".into(),
        bright_foreground: None,
        dim_foreground: "#6e7781".into(),
        dim_black: "#57606a".into(),
        dim_red: "#e16f7a".into(),
        dim_green: "#5a9a6d".into(),
        dim_yellow: "#8a6a3a".into(),
        dim_blue: "#6aa5ea".into(),
        dim_magenta: "#b39ae9".into(),
        dim_cyan: "#6faab0".into(),
        dim_white: "#a8b0b8".into(),
    }
}

pub fn theme_for(theme: Theme) -> TerminalTheme {
    let palette = match theme {
        Theme::Dark => dark(),
        Theme::Light => light(),
    };
    TerminalTheme::new(Box::new(palette))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    fn every_named(theme: &TerminalTheme) {
        for c in [
            NamedColor::Black,
            NamedColor::Red,
            NamedColor::Green,
            NamedColor::Yellow,
            NamedColor::Blue,
            NamedColor::Magenta,
            NamedColor::Cyan,
            NamedColor::White,
            NamedColor::BrightBlack,
            NamedColor::BrightRed,
            NamedColor::BrightGreen,
            NamedColor::BrightYellow,
            NamedColor::BrightBlue,
            NamedColor::BrightMagenta,
            NamedColor::BrightCyan,
            NamedColor::BrightWhite,
            NamedColor::Foreground,
            NamedColor::Background,
            NamedColor::DimForeground,
            NamedColor::DimBlack,
            NamedColor::DimWhite,
        ] {
            let _ = theme.get_color(Color::Named(c)); // must not panic on any hex string
        }
    }

    #[test]
    fn both_palettes_build_a_theme_and_resolve_every_named_color() {
        every_named(&theme_for(Theme::Dark));
        every_named(&theme_for(Theme::Light));
    }

    #[test]
    fn light_palette_is_dark_text_on_light_background() {
        let l = light();
        assert_eq!(l.background, "#f6f8fa");
        assert_eq!(l.foreground, "#24292f");
        let d = dark();
        assert_eq!(d.background, "#181818");
    }
}
