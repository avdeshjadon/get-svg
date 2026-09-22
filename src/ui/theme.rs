//! Color palette and styles. Never color-only information: selection and
//! status always carry a text/shape signal too.

use ratatui::style::{Color, Modifier, Style};

use crate::config::Settings;

#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub text: Color,
    pub dim: Color,
    pub border: Color,
    pub border_active: Color,
    pub error: Color,
    pub success: Color,
    pub warn: Color,
    pub selected_bg: Color,
    pub animations: bool,
}

impl Theme {
    /// Resolve a theme by name. Unknown names fall back to `default`.
    pub fn resolve(name: &str, animations: bool) -> Theme {
        match name {
            "high_contrast" | "high-contrast" | "hc" => Theme::high_contrast(animations),
            _ => Theme::default_theme(animations),
        }
    }

    pub fn load(settings: &Settings) -> Theme {
        Theme::resolve(&settings.theme, settings.animations)
    }

    pub fn default_theme(animations: bool) -> Theme {
        Theme {
            accent: Color::Cyan,
            text: Color::Gray,
            dim: Color::DarkGray,
            border: Color::DarkGray,
            border_active: Color::Cyan,
            error: Color::Red,
            success: Color::Green,
            warn: Color::Yellow,
            selected_bg: Color::DarkGray,
            animations,
        }
    }

    pub fn high_contrast(animations: bool) -> Theme {
        Theme {
            accent: Color::Yellow,
            text: Color::White,
            dim: Color::White,
            border: Color::White,
            border_active: Color::Yellow,
            error: Color::Red,
            success: Color::Green,
            warn: Color::Yellow,
            selected_bg: Color::Blue,
            animations,
        }
    }

    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent)
    }
    pub fn bold_accent(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }
    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }
    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.dim)
    }
    pub fn error_style(&self) -> Style {
        Style::default().fg(self.error).add_modifier(Modifier::BOLD)
    }
    pub fn success_style(&self) -> Style {
        Style::default().fg(self.success)
    }
    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.warn)
    }
    pub fn selected_style(&self) -> Style {
        Style::default()
            .fg(self.text)
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }
    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }
    pub fn border_active_style(&self) -> Style {
        Style::default().fg(self.border_active)
    }
    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    /// Progress bar glyphs: unicode by default, ASCII when animations are off
    /// or the terminal looks limited.
    pub fn bar_chars(&self) -> (&'static str, &'static str) {
        if self.animations {
            ("█", "░")
        } else {
            ("#", "-")
        }
    }
}
