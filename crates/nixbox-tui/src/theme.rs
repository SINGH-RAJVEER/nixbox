use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

pub struct Theme {
    pub name: &'static str,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub version_color: Color,
    pub border_color: Color,
    pub title_color: Color,
    /// None = transparent (uses terminal emulator background)
    pub bg_color: Option<Color>,
}

impl Theme {
    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn version_style(&self) -> Style {
        Style::default().fg(self.version_color)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border_color)
    }

    pub fn title_style(&self) -> Style {
        Style::default()
            .fg(self.title_color)
            .add_modifier(Modifier::BOLD)
    }

    pub fn name_style(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    pub fn border_type(&self) -> BorderType {
        BorderType::Rounded
    }
}

pub const ALL: &[Theme] = &[DEFAULT, DRACULA, GRUVBOX, NORD, CATPPUCCIN, MONOKAI];

pub const DEFAULT: Theme = Theme {
    name: "default",
    selection_bg: Color::Blue,
    selection_fg: Color::White,
    version_color: Color::DarkGray,
    border_color: Color::White,
    title_color: Color::White,
    bg_color: None,
};

pub const DRACULA: Theme = Theme {
    name: "dracula",
    selection_bg: Color::Rgb(189, 147, 249),
    selection_fg: Color::Rgb(40, 42, 54),
    version_color: Color::Rgb(98, 114, 164),
    border_color: Color::Rgb(98, 114, 164),
    title_color: Color::Rgb(255, 121, 198),
    bg_color: Some(Color::Rgb(40, 42, 54)),
};

pub const GRUVBOX: Theme = Theme {
    name: "gruvbox",
    selection_bg: Color::Rgb(215, 153, 33),
    selection_fg: Color::Rgb(40, 40, 40),
    version_color: Color::Rgb(168, 153, 132),
    border_color: Color::Rgb(102, 92, 84),
    title_color: Color::Rgb(250, 189, 47),
    bg_color: Some(Color::Rgb(40, 40, 40)),
};

pub const NORD: Theme = Theme {
    name: "nord",
    selection_bg: Color::Rgb(94, 129, 172),
    selection_fg: Color::Rgb(236, 239, 244),
    version_color: Color::Rgb(76, 86, 106),
    border_color: Color::Rgb(67, 76, 94),
    title_color: Color::Rgb(136, 192, 208),
    bg_color: Some(Color::Rgb(46, 52, 64)),
};

pub const CATPPUCCIN: Theme = Theme {
    name: "catppuccin",
    selection_bg: Color::Rgb(203, 166, 247),
    selection_fg: Color::Rgb(30, 30, 46),
    version_color: Color::Rgb(108, 112, 134),
    border_color: Color::Rgb(88, 91, 112),
    title_color: Color::Rgb(137, 180, 250),
    bg_color: Some(Color::Rgb(30, 30, 46)),
};

pub const MONOKAI: Theme = Theme {
    name: "monokai",
    selection_bg: Color::Rgb(166, 226, 46),
    selection_fg: Color::Rgb(39, 40, 34),
    version_color: Color::Rgb(117, 113, 94),
    border_color: Color::Rgb(117, 113, 94),
    title_color: Color::Rgb(249, 38, 114),
    bg_color: Some(Color::Rgb(39, 40, 34)),
};

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn the_palettes_match_the_names_the_settings_file_accepts() {
        let palettes: Vec<&str> = ALL.iter().map(|theme| theme.name).collect();

        assert_eq!(palettes, nixbox_config::THEMES);
    }
}
