//! How the model is drawn: the display style (CAD's shaded / edges / hidden-line modes) and the viewport background.
//! Pure data plus a small per-user settings file; the renderer and the menus read it, nothing here touches the GPU.
use serde::{Deserialize, Serialize};

/// CAD display styles. Edges are the model's feature edges (outline and sharp creases), never the mesh's triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DisplayStyle {
    Shaded,
    ShadedEdges,
    /// Hidden Lines Visible: visible edges solid, hidden edges dim.
    HiddenLinesVisible,
    /// Hidden Lines Removed: only the edges in front of the surface.
    HiddenLinesRemoved,
    #[default]
    Wireframe,
}
impl DisplayStyle {
    pub const ALL: [DisplayStyle; 5] =
        [DisplayStyle::Shaded, DisplayStyle::ShadedEdges, DisplayStyle::HiddenLinesVisible, DisplayStyle::HiddenLinesRemoved, DisplayStyle::Wireframe];
    pub fn label(self) -> &'static str {
        match self {
            Self::Shaded => "Shaded",
            Self::ShadedEdges => "Shaded with edges",
            Self::HiddenLinesVisible => "Hidden lines visible",
            Self::HiddenLinesRemoved => "Hidden lines removed",
            Self::Wireframe => "Wireframe",
        }
    }
    /// The surface is painted in colour (shading or contour).
    pub fn paints_surface(self) -> bool {
        matches!(self, Self::Shaded | Self::ShadedEdges)
    }
    /// The surface hides what lies behind it (it writes depth, with or without colour).
    pub fn occludes(self) -> bool {
        !matches!(self, Self::Wireframe)
    }
    /// Feature edges are drawn.
    pub fn draws_edges(self) -> bool {
        !matches!(self, Self::Shaded)
    }
    /// Hidden edges are drawn too, dimmed.
    pub fn draws_hidden_edges(self) -> bool {
        matches!(self, Self::HiddenLinesVisible)
    }
}

/// The viewport background: a solid colour or a top-to-bottom gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Background {
    /// The colour of the window theme.
    #[default]
    Theme,
    Black,
    White,
    Grey,
    GreyToWhite,
    BlackToGrey,
    Solid([u8; 3]),
    Gradient { top: [u8; 3], bottom: [u8; 3] },
}
impl Background {
    /// The presets the menus offer (the custom colours come from the picker).
    pub const PRESETS: [Background; 6] = [Background::Theme, Background::Black, Background::White, Background::Grey, Background::GreyToWhite, Background::BlackToGrey];
    pub fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Black => "Black",
            Self::White => "White",
            Self::Grey => "Grey",
            Self::GreyToWhite => "Gradient: grey to white",
            Self::BlackToGrey => "Gradient: black to grey",
            Self::Solid(_) => "Custom colour",
            Self::Gradient { .. } => "Custom gradient",
        }
    }
    /// `(top, bottom)` sRGB; `theme` is the window theme's own background.
    pub fn colors(self, theme: [u8; 3]) -> ([u8; 3], [u8; 3]) {
        match self {
            Self::Theme => (theme, theme),
            Self::Black => ([0; 3], [0; 3]),
            Self::White => ([255; 3], [255; 3]),
            Self::Grey => ([128; 3], [128; 3]),
            Self::GreyToWhite => ([112; 3], [255; 3]),
            Self::BlackToGrey => ([0; 3], [112; 3]),
            Self::Solid(c) => (c, c),
            Self::Gradient { top, bottom } => (top, bottom),
        }
    }
}

/// Edge and mesh-line colours (sRGB, 0..1) that stay readable on the given background: light lines on a dark
/// ground, dark on a light one. Hidden lines and the mesh are the visible colour toned toward the background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineColors {
    pub visible: [f32; 3],
    pub hidden: [f32; 3],
    pub mesh: [f32; 3],
}
pub fn line_colors(top: [u8; 3], bottom: [u8; 3]) -> LineColors {
    let mean = |k: usize| (f32::from(top[k]) + f32::from(bottom[k])) / 510.0;
    let ground = [mean(0), mean(1), mean(2)];
    let luma = 0.2126 * ground[0] + 0.7152 * ground[1] + 0.0722 * ground[2];
    let visible = if luma < 0.45 { [0.93, 0.95, 0.98] } else { [0.05, 0.07, 0.10] };
    let toward = |t: f32| -> [f32; 3] { std::array::from_fn(|k| visible[k] + (ground[k] - visible[k]) * t) };
    LineColors { visible, hidden: toward(0.72), mesh: toward(0.5) }
}

/// The colours of the model's own state in a geometry view: body, support, load, selected. The defaults are the contour
/// colour map at 0, 1/3, 2/3 and 1 (the scene codes 0..3), which is what the model was always drawn with.
pub fn default_key_colours() -> [[u8; 3]; 4] {
    std::array::from_fn(|k| super::text::colormap(k as f32 / 3.0))
}
fn default_key() -> [[u8; 3]; 4] {
    default_key_colours()
}

/// What persists between sessions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySettings {
    pub style: DisplayStyle,
    pub background: Background,
    /// Body, support, load and selected colours of the colour key.
    #[serde(default = "default_key")]
    pub key_colours: [[u8; 3]; 4],
    /// Where the colour key and the result legend were left, as fractions of the viewport (`None`: their starting places).
    #[serde(default)]
    pub panel_pos: [Option<[f32; 2]>; 2],
}
impl Default for DisplaySettings {
    fn default() -> Self {
        Self { style: DisplayStyle::default(), background: Background::default(), key_colours: default_key_colours(), panel_pos: [None; 2] }
    }
}
/// A stored panel position kept only when it is finite, clamped into the viewport.
pub fn sanitize_panel_pos(p: Option<[f32; 2]>) -> Option<[f32; 2]> {
    p.filter(|p| p.iter().all(|v| v.is_finite())).map(|p| p.map(|v| v.clamp(0.0, 1.0)))
}
const FILE: &str = "display.json";
impl DisplaySettings {
    /// `<app data>/fea-editor/display.json`, when the platform has an app-data folder.
    pub fn path() -> Option<std::path::PathBuf> {
        crate::paths::app_data_dir().map(|d| d.join("fea-editor").join(FILE))
    }
    /// The stored settings; the defaults when there is no file or it cannot be read.
    pub fn load_from(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_style_is_wireframe_and_each_style_has_one_meaning() {
        assert_eq!(DisplayStyle::default(), DisplayStyle::Wireframe);
        let row = |s: DisplayStyle| (s.paints_surface(), s.occludes(), s.draws_edges(), s.draws_hidden_edges());
        assert_eq!(row(DisplayStyle::Shaded), (true, true, false, false));
        assert_eq!(row(DisplayStyle::ShadedEdges), (true, true, true, false));
        assert_eq!(row(DisplayStyle::HiddenLinesVisible), (false, true, true, true));
        assert_eq!(row(DisplayStyle::HiddenLinesRemoved), (false, true, true, false));
        assert_eq!(row(DisplayStyle::Wireframe), (false, false, true, false));
    }

    #[test]
    fn presets_resolve_and_the_gradients_run_top_to_bottom() {
        let theme = [14, 19, 24];
        assert_eq!(Background::Theme.colors(theme), (theme, theme));
        assert_eq!(Background::Black.colors(theme), ([0; 3], [0; 3]));
        let (top, bottom) = Background::GreyToWhite.colors(theme);
        assert!(top[0] < bottom[0] && bottom == [255; 3]);
        let (top, bottom) = Background::BlackToGrey.colors(theme);
        assert!(top == [0; 3] && bottom[0] > 0);
        assert_eq!(Background::Gradient { top: [1, 2, 3], bottom: [4, 5, 6] }.colors(theme), ([1, 2, 3], [4, 5, 6]));
    }

    #[test]
    fn the_default_key_is_the_contour_map_and_an_old_settings_file_without_it_reads_as_the_default() {
        let k = default_key_colours();
        assert_eq!(k[0], super::super::text::colormap(0.0));
        assert_eq!(k[3], super::super::text::colormap(1.0));
        let old = r#"{"style":"Wireframe","background":"Theme"}"#;
        let s: DisplaySettings = serde_json::from_str(old).unwrap();
        assert_eq!(s.key_colours, k);
        assert_eq!(s.panel_pos, [None; 2], "an old file has no stored positions");
    }

    #[test]
    fn a_stored_panel_position_is_clamped_and_a_non_finite_one_is_dropped() {
        assert_eq!(sanitize_panel_pos(Some([-0.5, 3.0])), Some([0.0, 1.0]));
        assert_eq!(sanitize_panel_pos(Some([f32::NAN, 0.5])), None);
        assert_eq!(sanitize_panel_pos(Some([f32::INFINITY, 0.5])), None);
        assert_eq!(sanitize_panel_pos(None), None);
    }

    #[test]
    fn line_colours_contrast_with_the_ground() {
        let on_black = line_colors([0; 3], [0; 3]);
        let on_white = line_colors([255; 3], [255; 3]);
        assert!(on_black.visible[0] > 0.8 && on_white.visible[0] < 0.2);
        // Hidden lines are quieter than visible ones: closer to the ground.
        assert!((on_black.hidden[0] - 0.0).abs() < (on_black.visible[0] - 0.0).abs());
        assert!((on_white.hidden[0] - 1.0).abs() < (on_white.visible[0] - 1.0).abs());
        // A gradient is judged by its mean.
        assert!(line_colors([0; 3], [255; 3]).visible[0] < 0.2 || line_colors([0; 3], [255; 3]).visible[0] > 0.8);
    }

    #[test]
    fn settings_round_trip_and_a_missing_or_broken_file_gives_the_defaults() {
        let dir = std::env::temp_dir().join(format!("display-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("fea-editor").join(FILE);
        assert_eq!(DisplaySettings::load_from(&path), DisplaySettings::default());
        let s = DisplaySettings { style: DisplayStyle::HiddenLinesRemoved, background: Background::Gradient { top: [10, 20, 30], bottom: [200, 210, 220] }, key_colours: [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]], panel_pos: [Some([0.25, 0.5]), None] };
        s.save_to(&path).unwrap();
        assert_eq!(DisplaySettings::load_from(&path), s);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(DisplaySettings::load_from(&path), DisplaySettings::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
