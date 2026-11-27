/// Rendering colors. Defaults follow Mermaid's `default` theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub node_fill: String,
    pub node_stroke: String,
    pub line: String,
    pub text: String,
    pub edge_label_background: String,
    pub cluster_fill: String,
    pub cluster_stroke: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            node_fill: "#ECECFF".into(),
            node_stroke: "#9370DB".into(),
            line: "#333333".into(),
            text: "#333333".into(),
            edge_label_background: "#E8E8E8".into(),
            cluster_fill: "#FFFFDE".into(),
            cluster_stroke: "#AAAA33".into(),
        }
    }
}

/// SVG rendering options.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub font_family: String,
    pub font_size: f64,
    /// Space between two nodes of the same rank (Mermaid: `nodeSpacing`).
    pub node_spacing: f64,
    /// Space between two ranks (Mermaid: `rankSpacing`).
    pub rank_spacing: f64,
    /// Margin around the diagram.
    pub padding: f64,
    /// Prefix of SVG `id`s; change it when several diagrams share a page.
    pub id_prefix: String,
    pub theme: Theme,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            font_family: "\"trebuchet ms\", verdana, arial, sans-serif".into(),
            font_size: 16.0,
            node_spacing: 50.0,
            rank_spacing: 50.0,
            padding: 8.0,
            id_prefix: "oxmaid".into(),
            theme: Theme::default(),
        }
    }
}
