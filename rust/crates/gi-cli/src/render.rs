use std::fmt::Write as FmtWrite;
use std::io::{self, Write};
use std::sync::RwLock;

use crossterm::cursor::{MoveToColumn, RestorePosition, SavePosition};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor, Stylize};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{execute, queue};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTheme {
    heading: Color,
    emphasis: Color,
    strong: Color,
    inline_code: Color,
    link: Color,
    quote: Color,
    table_border: Color,
    code_block_border: Color,
    spinner_active: Color,
    spinner_done: Color,
    spinner_failed: Color,
}

/// Process-wide runtime theme override, stored as a canonical theme name.
///
/// Seeded from persisted config at startup and updated live by the `/theme`
/// slash command. `ColorTheme::default()` consults it *after* the
/// `GI_THEME` environment variable so env selection always wins (keeping
/// CI and scripted runs deterministic).
static RUNTIME_THEME: RwLock<Option<String>> = RwLock::new(None);

/// Where the effective theme came from, for `status`/`doctor` reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSource {
    /// Pinned by the `GI_THEME` environment variable.
    Env,
    /// Selected via `/theme` and/or persisted in `~/.gi/settings.json`.
    Config,
    /// Built-in fallback palette (no env var, no override).
    Default,
}

impl ThemeSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeSource::Env => "env",
            ThemeSource::Config => "config",
            ThemeSource::Default => "default",
        }
    }
}

/// Map any accepted alias to its canonical theme name, or `None` if unknown.
#[must_use]
pub fn canonical_theme_name(name: &str) -> Option<&'static str> {
    match normalize_theme_name(name).as_str() {
        "gi-dark" | "dark" => Some("gi-dark"),
        "gi-light" | "light" => Some("gi-light"),
        "gi-matcha" | "matcha" | "green" => Some("gi-matcha"),
        "gi-sumi" | "sumi" | "ink" | "mono" => Some("gi-sumi"),
        "gi-sunrise" | "sunrise" | "warm" => Some("gi-sunrise"),
        _ => None,
    }
}

/// All selectable canonical theme names, in display order. Slice 13.
pub const THEME_NAMES: [&str; 5] = ["gi-dark", "gi-light", "gi-matcha", "gi-sumi", "gi-sunrise"];

/// A one-line preview of a theme: colored swatches (heading · strong · link ·
/// code) followed by the name. Without color (or for an unknown name) it
/// returns just the name. Slice 13.
#[must_use]
pub fn theme_swatch(name: &str, use_color: bool) -> String {
    let Some(theme) = ColorTheme::named(name) else {
        return name.to_string();
    };
    if !use_color {
        return name.to_string();
    }
    let block = |color: Color| match color {
        Color::Rgb { r, g, b } => format!("\x1b[38;2;{r};{g};{b}m███\x1b[0m"),
        _ => "███".to_string(),
    };
    format!(
        "{} {} {} {}  {name}",
        block(theme.heading),
        block(theme.strong),
        block(theme.link),
        block(theme.inline_code),
    )
}

/// SGR foreground sequence for a theme `Color` (truecolor / 256-color; named
/// colors fall back to no explicit color so the bold attribute still reads).
fn theme_fg(color: Color) -> String {
    match color {
        Color::Rgb { r, g, b } => format!("\x1b[38;2;{r};{g};{b}m"),
        Color::AnsiValue(n) => format!("\x1b[38;5;{n}m"),
        _ => String::new(),
    }
}

/// Semantic accent color for a non-default operating mode, used to tint the
/// prompt box + `❯` glyph so the active mode is unmistakable. `default`/empty
/// → `None` (keep the neutral theme color). These are intentional status
/// colors (like the `spinner_*` palette), not theme-driven. Slice 15.
#[must_use]
pub fn mode_accent(mode: &str) -> Option<Color> {
    match mode {
        "plan" => Some(Color::Rgb {
            r: 96,
            g: 165,
            b: 250,
        }), // calm blue — read-only
        "edit" => Some(Color::Rgb {
            r: 122,
            g: 199,
            b: 120,
        }), // green — active editing
        "mugen" => Some(Color::Rgb {
            r: 236,
            g: 72,
            b: 120,
        }), // loud red/magenta — autonomous
        _ => None,
    }
}

/// The themed prompt indicator that replaces the bare `> ` (a colored `❯ `).
/// Its visible width is always 2. `NO_COLOR` → plain `"❯ "`. An `accent` (from
/// [`mode_accent`]) tints the glyph per active mode; `None` keeps the theme
/// color. Slice 14a / 15.
#[must_use]
pub fn prompt_glyph(use_color: bool, accent: Option<Color>) -> String {
    if !use_color {
        return "❯ ".to_string();
    }
    let color = accent.unwrap_or_else(|| ColorTheme::default().heading);
    format!("{}\x1b[1m❯\x1b[0m ", theme_fg(color))
}

/// A themed bounding-box header shown above the prompt, labeling the active
/// `mode` and `agent` (e.g. `╭─ edit · reviewer ─╮`). Returns `None` when there
/// is nothing to show. `NO_COLOR` → the plain box-drawing form. A non-default
/// `mode` tints the border + title via [`mode_accent`]. Slice 14a / 15.
#[must_use]
pub fn prompt_header(agent: Option<&str>, mode: Option<&str>, use_color: bool) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(mode) = mode.filter(|value| !value.is_empty()) {
        parts.push(mode);
    }
    if let Some(agent) = agent.filter(|value| !value.is_empty()) {
        parts.push(agent);
    }
    if parts.is_empty() {
        return None;
    }
    let label = parts.join(" · ");
    if !use_color {
        return Some(format!("╭─ {label} ─╮"));
    }
    let theme = ColorTheme::default();
    let accent = mode.and_then(mode_accent);
    let border = theme_fg(accent.unwrap_or(theme.code_block_border));
    let title = theme_fg(accent.unwrap_or(theme.heading));
    let reset = "\x1b[0m";
    Some(format!(
        "{border}╭─ {reset}{title}{label}{reset}{border} ─╮{reset}"
    ))
}

/// Pure precedence resolver: env over runtime override over built-in default.
/// Returns the canonical theme name (`None` means the built-in fallback) and
/// the source it was selected from.
fn resolve_theme(env: Option<&str>, runtime: Option<&str>) -> (Option<&'static str>, ThemeSource) {
    if let Some(name) = env.and_then(canonical_theme_name) {
        return (Some(name), ThemeSource::Env);
    }
    if let Some(name) = runtime.and_then(canonical_theme_name) {
        return (Some(name), ThemeSource::Config);
    }
    (None, ThemeSource::Default)
}

fn env_theme_var() -> Option<String> {
    std::env::var("GI_THEME").ok()
}

fn runtime_theme_name() -> Option<String> {
    RUNTIME_THEME.read().ok().and_then(|slot| slot.clone())
}

/// Set the process-wide runtime theme override by (aliased) name.
///
/// Returns `true` when the name resolves to a known theme and the override was
/// stored; `false` for an unknown name (the override is left unchanged).
pub fn set_runtime_theme(name: &str) -> bool {
    match canonical_theme_name(name) {
        Some(canonical) => {
            if let Ok(mut slot) = RUNTIME_THEME.write() {
                *slot = Some(canonical.to_string());
            }
            true
        }
        None => false,
    }
}

/// Clear the process-wide runtime theme override (falls back to env/default).
pub fn clear_runtime_theme() {
    if let Ok(mut slot) = RUNTIME_THEME.write() {
        *slot = None;
    }
}

/// The effective theme name and where it was resolved from, honoring the same
/// precedence as [`ColorTheme::default`]. The name is `"default"` for the
/// built-in fallback palette.
#[must_use]
pub fn effective_theme() -> (&'static str, ThemeSource) {
    let env = env_theme_var();
    let runtime = runtime_theme_name();
    match resolve_theme(env.as_deref(), runtime.as_deref()) {
        (Some(name), source) => (name, source),
        (None, source) => ("default", source),
    }
}

impl Default for ColorTheme {
    fn default() -> Self {
        match effective_theme() {
            ("gi-dark", _) => return Self::gi_dark(),
            ("gi-light", _) => return Self::gi_light(),
            ("gi-matcha", _) => return Self::gi_matcha(),
            ("gi-sumi", _) => return Self::gi_sumi(),
            ("gi-sunrise", _) => return Self::gi_sunrise(),
            _ => {}
        }
        Self {
            heading: Color::Cyan,
            emphasis: Color::Magenta,
            strong: Color::Yellow,
            inline_code: Color::Green,
            link: Color::Blue,
            quote: Color::DarkGrey,
            table_border: Color::DarkCyan,
            code_block_border: Color::DarkGrey,
            spinner_active: Color::Blue,
            spinner_done: Color::Green,
            spinner_failed: Color::Red,
        }
    }
}

impl ColorTheme {
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match canonical_theme_name(name)? {
            "gi-dark" => Some(Self::gi_dark()),
            "gi-light" => Some(Self::gi_light()),
            "gi-matcha" => Some(Self::gi_matcha()),
            "gi-sumi" => Some(Self::gi_sumi()),
            "gi-sunrise" => Some(Self::gi_sunrise()),
            _ => None,
        }
    }

    #[must_use]
    pub const fn gi_dark() -> Self {
        Self {
            heading: Color::Rgb {
                r: 113,
                g: 187,
                b: 226,
            },
            emphasis: Color::Rgb {
                r: 242,
                g: 154,
                b: 180,
            },
            strong: Color::Rgb {
                r: 245,
                g: 184,
                b: 92,
            },
            inline_code: Color::Rgb {
                r: 131,
                g: 201,
                b: 168,
            },
            link: Color::Rgb {
                r: 139,
                g: 170,
                b: 247,
            },
            quote: Color::Rgb {
                r: 151,
                g: 164,
                b: 181,
            },
            table_border: Color::Rgb {
                r: 79,
                g: 169,
                b: 185,
            },
            code_block_border: Color::Rgb {
                r: 92,
                g: 105,
                b: 128,
            },
            spinner_active: Color::Rgb {
                r: 96,
                g: 202,
                b: 212,
            },
            spinner_done: Color::Rgb {
                r: 129,
                g: 206,
                b: 157,
            },
            spinner_failed: Color::Rgb {
                r: 235,
                g: 111,
                b: 146,
            },
        }
    }

    #[must_use]
    pub const fn gi_light() -> Self {
        Self {
            heading: Color::Rgb {
                r: 26,
                g: 92,
                b: 140,
            },
            emphasis: Color::Rgb {
                r: 169,
                g: 71,
                b: 105,
            },
            strong: Color::Rgb {
                r: 171,
                g: 91,
                b: 24,
            },
            inline_code: Color::Rgb {
                r: 54,
                g: 119,
                b: 85,
            },
            link: Color::Rgb {
                r: 52,
                g: 85,
                b: 161,
            },
            quote: Color::Rgb {
                r: 103,
                g: 105,
                b: 110,
            },
            table_border: Color::Rgb {
                r: 57,
                g: 137,
                b: 147,
            },
            code_block_border: Color::Rgb {
                r: 146,
                g: 140,
                b: 127,
            },
            spinner_active: Color::Rgb {
                r: 0,
                g: 124,
                b: 139,
            },
            spinner_done: Color::Rgb {
                r: 68,
                g: 134,
                b: 91,
            },
            spinner_failed: Color::Rgb {
                r: 188,
                g: 69,
                b: 86,
            },
        }
    }

    /// Matcha — calm green/tea palette for dark terminals. Slice 13.
    #[must_use]
    pub const fn gi_matcha() -> Self {
        Self {
            heading: rgb(126, 176, 109),
            emphasis: rgb(214, 158, 92),
            strong: rgb(196, 130, 78),
            inline_code: rgb(150, 190, 140),
            link: rgb(110, 178, 162),
            quote: rgb(140, 150, 135),
            table_border: rgb(120, 150, 110),
            code_block_border: rgb(95, 110, 90),
            spinner_active: rgb(150, 200, 120),
            spinner_done: rgb(130, 200, 150),
            spinner_failed: rgb(214, 120, 110),
        }
    }

    /// Sumi — near-monochrome ink palette (a faint indigo keeps links legible).
    /// Slice 13.
    #[must_use]
    pub const fn gi_sumi() -> Self {
        Self {
            heading: rgb(222, 222, 226),
            emphasis: rgb(182, 184, 190),
            strong: rgb(238, 238, 240),
            inline_code: rgb(160, 165, 172),
            link: rgb(150, 170, 204),
            quote: rgb(120, 122, 128),
            table_border: rgb(110, 112, 118),
            code_block_border: rgb(90, 92, 98),
            spinner_active: rgb(200, 202, 208),
            spinner_done: rgb(150, 200, 160),
            spinner_failed: rgb(210, 120, 120),
        }
    }

    /// Sunrise — warm rising-sun palette (oranges/vermilion). Slice 13.
    #[must_use]
    pub const fn gi_sunrise() -> Self {
        Self {
            heading: rgb(236, 140, 92),
            emphasis: rgb(228, 120, 140),
            strong: rgb(240, 176, 84),
            inline_code: rgb(224, 150, 110),
            link: rgb(220, 126, 100),
            quote: rgb(180, 150, 140),
            table_border: rgb(216, 150, 110),
            code_block_border: rgb(150, 110, 95),
            spinner_active: rgb(244, 160, 100),
            spinner_done: rgb(150, 200, 150),
            spinner_failed: rgb(224, 96, 96),
        }
    }
}

/// Compact truecolor constructor used by the palette definitions.
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
}

fn normalize_theme_name(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-")
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Spinner {
    frame_index: usize,
}

impl Spinner {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(
        &mut self,
        label: &str,
        theme: &ColorTheme,
        out: &mut impl Write,
    ) -> io::Result<()> {
        let frame = Self::FRAMES[self.frame_index % Self::FRAMES.len()];
        self.frame_index += 1;
        queue!(
            out,
            SavePosition,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(theme.spinner_active),
            Print(format!("{frame} {label}")),
            ResetColor,
            RestorePosition
        )?;
        out.flush()
    }

    pub fn finish(
        &mut self,
        label: &str,
        theme: &ColorTheme,
        out: &mut impl Write,
    ) -> io::Result<()> {
        self.frame_index = 0;
        execute!(
            out,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(theme.spinner_done),
            Print(format!("✔ {label}\n")),
            ResetColor
        )?;
        out.flush()
    }

    pub fn fail(
        &mut self,
        label: &str,
        theme: &ColorTheme,
        out: &mut impl Write,
    ) -> io::Result<()> {
        self.frame_index = 0;
        execute!(
            out,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(theme.spinner_failed),
            Print(format!("✘ {label}\n")),
            ResetColor
        )?;
        out.flush()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ListKind {
    Unordered,
    Ordered { next_index: u64 },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct TableState {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
    in_head: bool,
}

impl TableState {
    fn push_cell(&mut self) {
        let cell = self.current_cell.trim().to_string();
        self.current_row.push(cell);
        self.current_cell.clear();
    }

    fn finish_row(&mut self) {
        if self.current_row.is_empty() {
            return;
        }
        let row = std::mem::take(&mut self.current_row);
        if self.in_head {
            self.headers = row;
        } else {
            self.rows.push(row);
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RenderState {
    emphasis: usize,
    strong: usize,
    heading_level: Option<u8>,
    quote: usize,
    list_stack: Vec<ListKind>,
    link_stack: Vec<LinkState>,
    table: Option<TableState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkState {
    destination: String,
    text: String,
}

impl RenderState {
    fn style_text(&self, text: &str, theme: &ColorTheme) -> String {
        let mut style = text.stylize();

        if matches!(self.heading_level, Some(1 | 2)) || self.strong > 0 {
            style = style.bold();
        }
        if self.emphasis > 0 {
            style = style.italic();
        }

        if let Some(level) = self.heading_level {
            style = match level {
                1 => style.with(theme.heading),
                2 => style.white(),
                3 => style.with(Color::Blue),
                _ => style.with(Color::Grey),
            };
        } else if self.strong > 0 {
            style = style.with(theme.strong);
        } else if self.emphasis > 0 {
            style = style.with(theme.emphasis);
        }

        if self.quote > 0 {
            style = style.with(theme.quote);
        }

        format!("{style}")
    }

    fn append_raw(&mut self, output: &mut String, text: &str) {
        if let Some(link) = self.link_stack.last_mut() {
            link.text.push_str(text);
        } else if let Some(table) = self.table.as_mut() {
            table.current_cell.push_str(text);
        } else {
            output.push_str(text);
        }
    }

    fn append_styled(&mut self, output: &mut String, text: &str, theme: &ColorTheme) {
        let styled = self.style_text(text, theme);
        self.append_raw(output, &styled);
    }
}

#[derive(Debug)]
pub struct TerminalRenderer {
    syntax_set: SyntaxSet,
    syntax_theme: Theme,
    color_theme: ColorTheme,
}

impl Default for TerminalRenderer {
    fn default() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let syntax_theme = ThemeSet::load_defaults()
            .themes
            .remove("base16-ocean.dark")
            .unwrap_or_default();
        Self {
            syntax_set,
            syntax_theme,
            color_theme: ColorTheme::default(),
        }
    }
}

impl TerminalRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn color_theme(&self) -> &ColorTheme {
        &self.color_theme
    }

    #[must_use]
    pub fn render_markdown(&self, markdown: &str) -> String {
        let normalized = normalize_nested_fences(markdown);
        let mut output = String::new();
        let mut state = RenderState::default();
        let mut code_language = String::new();
        let mut code_buffer = String::new();
        let mut in_code_block = false;

        for event in Parser::new_ext(&normalized, Options::all()) {
            self.render_event(
                event,
                &mut state,
                &mut output,
                &mut code_buffer,
                &mut code_language,
                &mut in_code_block,
            );
        }

        output.trim_end().to_string()
    }

    #[must_use]
    pub fn markdown_to_ansi(&self, markdown: &str) -> String {
        self.render_markdown(markdown)
    }

    #[allow(clippy::too_many_lines)]
    fn render_event(
        &self,
        event: Event<'_>,
        state: &mut RenderState,
        output: &mut String,
        code_buffer: &mut String,
        code_language: &mut String,
        in_code_block: &mut bool,
    ) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                Self::start_heading(state, level as u8, output);
            }
            Event::End(TagEnd::Paragraph) => output.push_str("\n\n"),
            Event::Start(Tag::BlockQuote(..)) => self.start_quote(state, output),
            Event::End(TagEnd::BlockQuote(..)) => {
                state.quote = state.quote.saturating_sub(1);
                output.push('\n');
            }
            Event::End(TagEnd::Heading(..)) => {
                state.heading_level = None;
                output.push_str("\n\n");
            }
            Event::End(TagEnd::Item) | Event::SoftBreak | Event::HardBreak => {
                state.append_raw(output, "\n");
            }
            Event::Start(Tag::List(first_item)) => {
                let kind = match first_item {
                    Some(index) => ListKind::Ordered { next_index: index },
                    None => ListKind::Unordered,
                };
                state.list_stack.push(kind);
            }
            Event::End(TagEnd::List(..)) => {
                state.list_stack.pop();
                output.push('\n');
            }
            Event::Start(Tag::Item) => Self::start_item(state, output),
            Event::Start(Tag::CodeBlock(kind)) => {
                *in_code_block = true;
                *code_language = match kind {
                    CodeBlockKind::Indented => String::from("text"),
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                };
                code_buffer.clear();
                self.start_code_block(code_language, output);
            }
            Event::End(TagEnd::CodeBlock) => {
                self.finish_code_block(code_buffer, code_language, output);
                *in_code_block = false;
                code_language.clear();
                code_buffer.clear();
            }
            Event::Start(Tag::Emphasis) => state.emphasis += 1,
            Event::End(TagEnd::Emphasis) => state.emphasis = state.emphasis.saturating_sub(1),
            Event::Start(Tag::Strong) => state.strong += 1,
            Event::End(TagEnd::Strong) => state.strong = state.strong.saturating_sub(1),
            Event::Code(code) => {
                let rendered =
                    format!("{}", format!("`{code}`").with(self.color_theme.inline_code));
                state.append_raw(output, &rendered);
            }
            Event::Rule => output.push_str("---\n"),
            Event::Text(text) => {
                self.push_text(text.as_ref(), state, output, code_buffer, *in_code_block);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                state.append_raw(output, &html);
            }
            Event::FootnoteReference(reference) => {
                state.append_raw(output, &format!("[{reference}]"));
            }
            Event::TaskListMarker(done) => {
                state.append_raw(output, if done { "[x] " } else { "[ ] " });
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                state.append_raw(output, &math);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                state.link_stack.push(LinkState {
                    destination: dest_url.to_string(),
                    text: String::new(),
                });
            }
            Event::End(TagEnd::Link) => {
                if let Some(link) = state.link_stack.pop() {
                    let label = if link.text.is_empty() {
                        link.destination.clone()
                    } else {
                        link.text
                    };
                    let rendered = format!(
                        "{}",
                        format!("[{label}]({})", link.destination)
                            .underlined()
                            .with(self.color_theme.link)
                    );
                    state.append_raw(output, &rendered);
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let rendered = format!(
                    "{}",
                    format!("[image:{dest_url}]").with(self.color_theme.link)
                );
                state.append_raw(output, &rendered);
            }
            Event::Start(Tag::Table(..)) => state.table = Some(TableState::default()),
            Event::End(TagEnd::Table) => {
                if let Some(table) = state.table.take() {
                    output.push_str(&self.render_table(&table));
                    output.push_str("\n\n");
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(table) = state.table.as_mut() {
                    table.in_head = true;
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(table) = state.table.as_mut() {
                    table.finish_row();
                    table.in_head = false;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(table) = state.table.as_mut() {
                    table.current_row.clear();
                    table.current_cell.clear();
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(table) = state.table.as_mut() {
                    table.finish_row();
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(table) = state.table.as_mut() {
                    table.current_cell.clear();
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(table) = state.table.as_mut() {
                    table.push_cell();
                }
            }
            Event::Start(Tag::Paragraph | Tag::MetadataBlock(..) | _)
            | Event::End(TagEnd::Image | TagEnd::MetadataBlock(..) | _) => {}
        }
    }

    fn start_heading(state: &mut RenderState, level: u8, output: &mut String) {
        state.heading_level = Some(level);
        if !output.is_empty() {
            output.push('\n');
        }
    }

    fn start_quote(&self, state: &mut RenderState, output: &mut String) {
        state.quote += 1;
        let _ = write!(output, "{}", "│ ".with(self.color_theme.quote));
    }

    fn start_item(state: &mut RenderState, output: &mut String) {
        let depth = state.list_stack.len().saturating_sub(1);
        output.push_str(&"  ".repeat(depth));

        let marker = match state.list_stack.last_mut() {
            Some(ListKind::Ordered { next_index }) => {
                let value = *next_index;
                *next_index += 1;
                format!("{value}. ")
            }
            _ => "• ".to_string(),
        };
        output.push_str(&marker);
    }

    fn start_code_block(&self, code_language: &str, output: &mut String) {
        let label = if code_language.is_empty() {
            "code".to_string()
        } else {
            code_language.to_string()
        };
        let _ = writeln!(
            output,
            "{}",
            format!("╭─ {label}")
                .bold()
                .with(self.color_theme.code_block_border)
        );
    }

    fn finish_code_block(&self, code_buffer: &str, code_language: &str, output: &mut String) {
        output.push_str(&self.highlight_code(code_buffer, code_language));
        let _ = write!(
            output,
            "{}",
            "╰─".bold().with(self.color_theme.code_block_border)
        );
        output.push_str("\n\n");
    }

    fn push_text(
        &self,
        text: &str,
        state: &mut RenderState,
        output: &mut String,
        code_buffer: &mut String,
        in_code_block: bool,
    ) {
        if in_code_block {
            code_buffer.push_str(text);
        } else {
            state.append_styled(output, text, &self.color_theme);
        }
    }

    fn render_table(&self, table: &TableState) -> String {
        let mut rows = Vec::new();
        if !table.headers.is_empty() {
            rows.push(table.headers.clone());
        }
        rows.extend(table.rows.iter().cloned());

        if rows.is_empty() {
            return String::new();
        }

        let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths = (0..column_count)
            .map(|column| {
                rows.iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| visible_width(cell))
                    .max()
                    .unwrap_or(0)
            })
            .collect::<Vec<_>>();

        let border = format!("{}", "│".with(self.color_theme.table_border));
        let separator = widths
            .iter()
            .map(|width| "─".repeat(*width + 2))
            .collect::<Vec<_>>()
            .join(&format!("{}", "┼".with(self.color_theme.table_border)));
        let separator = format!("{border}{separator}{border}");

        let mut output = String::new();
        if !table.headers.is_empty() {
            output.push_str(&self.render_table_row(&table.headers, &widths, true));
            output.push('\n');
            output.push_str(&separator);
            if !table.rows.is_empty() {
                output.push('\n');
            }
        }

        for (index, row) in table.rows.iter().enumerate() {
            output.push_str(&self.render_table_row(row, &widths, false));
            if index + 1 < table.rows.len() {
                output.push('\n');
            }
        }

        output
    }

    fn render_table_row(&self, row: &[String], widths: &[usize], is_header: bool) -> String {
        let border = format!("{}", "│".with(self.color_theme.table_border));
        let mut line = String::new();
        line.push_str(&border);

        for (index, width) in widths.iter().enumerate() {
            let cell = row.get(index).map_or("", String::as_str);
            line.push(' ');
            if is_header {
                let _ = write!(line, "{}", cell.bold().with(self.color_theme.heading));
            } else {
                line.push_str(cell);
            }
            let padding = width.saturating_sub(visible_width(cell));
            line.push_str(&" ".repeat(padding + 1));
            line.push_str(&border);
        }

        line
    }

    #[must_use]
    pub fn highlight_code(&self, code: &str, language: &str) -> String {
        let syntax = self
            .syntax_set
            .find_syntax_by_token(language)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());
        let mut syntax_highlighter = HighlightLines::new(syntax, &self.syntax_theme);
        let mut colored_output = String::new();

        for line in LinesWithEndings::from(code) {
            match syntax_highlighter.highlight_line(line, &self.syntax_set) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                    colored_output.push_str(&apply_code_block_background(&escaped));
                }
                Err(_) => colored_output.push_str(&apply_code_block_background(line)),
            }
        }

        colored_output
    }

    pub fn stream_markdown(&self, markdown: &str, out: &mut impl Write) -> io::Result<()> {
        let rendered_markdown = self.markdown_to_ansi(markdown);
        write!(out, "{rendered_markdown}")?;
        if !rendered_markdown.ends_with('\n') {
            writeln!(out)?;
        }
        out.flush()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MarkdownStreamState {
    pending: String,
}

impl MarkdownStreamState {
    #[must_use]
    pub fn push(&mut self, renderer: &TerminalRenderer, delta: &str) -> Option<String> {
        self.pending.push_str(delta);
        let split = find_stream_safe_boundary(&self.pending)?;
        let ready = self.pending[..split].to_string();
        self.pending.drain(..split);
        Some(renderer.markdown_to_ansi(&ready))
    }

    #[must_use]
    pub fn flush(&mut self, renderer: &TerminalRenderer) -> Option<String> {
        if self.pending.trim().is_empty() {
            self.pending.clear();
            None
        } else {
            let pending = std::mem::take(&mut self.pending);
            Some(renderer.markdown_to_ansi(&pending))
        }
    }
}

fn apply_code_block_background(line: &str) -> String {
    let trimmed = line.trim_end_matches('\n');
    let trailing_newline = if trimmed.len() == line.len() {
        ""
    } else {
        "\n"
    };
    let with_background = trimmed.replace("\u{1b}[0m", "\u{1b}[0;48;5;236m");
    format!("\u{1b}[48;5;236m{with_background}\u{1b}[0m{trailing_newline}")
}

/// Pre-process raw markdown so that fenced code blocks whose body contains
/// fence markers of equal or greater length are wrapped with a longer fence.
///
/// LLMs frequently emit triple-backtick code blocks that contain triple-backtick
/// examples.  `CommonMark` (and pulldown-cmark) treats the inner marker as the
/// closing fence, breaking the render.  This function detects the situation and
/// upgrades the outer fence to use enough backticks (or tildes) that the inner
/// markers become ordinary content.
#[allow(
    clippy::too_many_lines,
    clippy::items_after_statements,
    clippy::manual_repeat_n,
    clippy::manual_str_repeat
)]
fn normalize_nested_fences(markdown: &str) -> String {
    // A fence line is either "labeled" (has an info string ⇒ always an opener)
    // or "bare" (no info string ⇒ could be opener or closer).
    #[derive(Debug, Clone)]
    struct FenceLine {
        char: char,
        len: usize,
        has_info: bool,
        indent: usize,
    }

    fn parse_fence_line(line: &str) -> Option<FenceLine> {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        let indent = trimmed.chars().take_while(|c| *c == ' ').count();
        if indent > 3 {
            return None;
        }
        let rest = &trimmed[indent..];
        let ch = rest.chars().next()?;
        if ch != '`' && ch != '~' {
            return None;
        }
        let len = rest.chars().take_while(|c| *c == ch).count();
        if len < 3 {
            return None;
        }
        let after = &rest[len..];
        if ch == '`' && after.contains('`') {
            return None;
        }
        let has_info = !after.trim().is_empty();
        Some(FenceLine {
            char: ch,
            len,
            has_info,
            indent,
        })
    }

    let lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    // Handle final line that may lack trailing newline.
    // split_inclusive already keeps the original chunks, including a
    // final chunk without '\n' if the input doesn't end with one.

    // First pass: classify every line.
    let fence_info: Vec<Option<FenceLine>> = lines.iter().map(|l| parse_fence_line(l)).collect();

    // Second pass: pair openers with closers using a stack, recording
    // (opener_idx, closer_idx) pairs plus the max fence length found between
    // them.
    struct StackEntry {
        line_idx: usize,
        fence: FenceLine,
    }

    let mut stack: Vec<StackEntry> = Vec::new();
    // Paired blocks: (opener_line, closer_line, max_inner_fence_len)
    let mut pairs: Vec<(usize, usize, usize)> = Vec::new();

    for (i, fi) in fence_info.iter().enumerate() {
        let Some(fl) = fi else { continue };

        if fl.has_info {
            // Labeled fence ⇒ always an opener.
            stack.push(StackEntry {
                line_idx: i,
                fence: fl.clone(),
            });
        } else {
            // Bare fence ⇒ try to close the top of the stack if compatible.
            let closes_top = stack
                .last()
                .is_some_and(|top| top.fence.char == fl.char && fl.len >= top.fence.len);
            if closes_top {
                let opener = stack.pop().unwrap();
                // Find max fence length of any fence line strictly between
                // opener and closer (these are the nested fences).
                let inner_max = fence_info[opener.line_idx + 1..i]
                    .iter()
                    .filter_map(|fi| fi.as_ref().map(|f| f.len))
                    .max()
                    .unwrap_or(0);
                pairs.push((opener.line_idx, i, inner_max));
            } else {
                // Treat as opener.
                stack.push(StackEntry {
                    line_idx: i,
                    fence: fl.clone(),
                });
            }
        }
    }

    // Determine which lines need rewriting.  A pair needs rewriting when
    // its opener length <= max inner fence length.
    struct Rewrite {
        char: char,
        new_len: usize,
        indent: usize,
    }
    let mut rewrites: std::collections::HashMap<usize, Rewrite> = std::collections::HashMap::new();

    for (opener_idx, closer_idx, inner_max) in &pairs {
        let opener_fl = fence_info[*opener_idx].as_ref().unwrap();
        if opener_fl.len <= *inner_max {
            let new_len = inner_max + 1;
            let info_part = {
                let trimmed = lines[*opener_idx]
                    .trim_end_matches('\n')
                    .trim_end_matches('\r');
                let rest = &trimmed[opener_fl.indent..];
                rest[opener_fl.len..].to_string()
            };
            rewrites.insert(
                *opener_idx,
                Rewrite {
                    char: opener_fl.char,
                    new_len,
                    indent: opener_fl.indent,
                },
            );
            let closer_fl = fence_info[*closer_idx].as_ref().unwrap();
            rewrites.insert(
                *closer_idx,
                Rewrite {
                    char: closer_fl.char,
                    new_len,
                    indent: closer_fl.indent,
                },
            );
            // Store info string only in the opener; closer keeps the trailing
            // portion which is already handled through the original line.
            // Actually, we rebuild both lines from scratch below, including
            // the info string for the opener.
            let _ = info_part; // consumed in rebuild
        }
    }

    if rewrites.is_empty() {
        return markdown.to_string();
    }

    // Rebuild.
    let mut out = String::with_capacity(markdown.len() + rewrites.len() * 4);
    for (i, line) in lines.iter().enumerate() {
        if let Some(rw) = rewrites.get(&i) {
            let fence_str: String = std::iter::repeat(rw.char).take(rw.new_len).collect();
            let indent_str: String = std::iter::repeat(' ').take(rw.indent).collect();
            // Recover the original info string (if any) and trailing newline.
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            let fi = fence_info[i].as_ref().unwrap();
            let info = &trimmed[fi.indent + fi.len..];
            let trailing = &line[trimmed.len()..];
            out.push_str(&indent_str);
            out.push_str(&fence_str);
            out.push_str(info);
            out.push_str(trailing);
        } else {
            out.push_str(line);
        }
    }
    out
}

fn find_stream_safe_boundary(markdown: &str) -> Option<usize> {
    let mut open_fence: Option<FenceMarker> = None;
    let mut last_boundary = None;

    for (offset, line) in markdown.split_inclusive('\n').scan(0usize, |cursor, line| {
        let start = *cursor;
        *cursor += line.len();
        Some((start, line))
    }) {
        let line_without_newline = line.trim_end_matches('\n');
        if let Some(opener) = open_fence {
            if line_closes_fence(line_without_newline, opener) {
                open_fence = None;
                last_boundary = Some(offset + line.len());
            }
            continue;
        }

        if let Some(opener) = parse_fence_opener(line_without_newline) {
            open_fence = Some(opener);
            continue;
        }

        if line_without_newline.trim().is_empty() {
            last_boundary = Some(offset + line.len());
        }
    }

    last_boundary
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceMarker {
    character: char,
    length: usize,
}

fn parse_fence_opener(line: &str) -> Option<FenceMarker> {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let character = rest.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = rest.chars().take_while(|c| *c == character).count();
    if length < 3 {
        return None;
    }
    let info_string = &rest[length..];
    if character == '`' && info_string.contains('`') {
        return None;
    }
    Some(FenceMarker { character, length })
}

fn line_closes_fence(line: &str, opener: FenceMarker) -> bool {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let length = rest.chars().take_while(|c| *c == opener.character).count();
    if length < opener.length {
        return false;
    }
    rest[length..].chars().all(|c| c == ' ' || c == '\t')
}

fn visible_width(input: &str) -> usize {
    strip_ansi(input).chars().count()
}

pub(crate) fn strip_ansi(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            output.push(ch);
        }
    }

    output
}

/// Best-effort terminal width, clamped to a readable range. Falls back to 80
/// columns when the size can't be queried (pipes, CI, non-TTY). Slice 11.
#[must_use]
pub fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map_or(80, |(cols, _)| cols as usize)
        .clamp(40, 100)
}

/// Render `body` inside a width-aware rounded panel matching gi's tool-call box
/// style (grey border, optional cyan title). `body` may contain ANSI escapes;
/// padding is computed from the *visible* width. The panel is sized to its
/// content but never wider than the terminal. Emits no ANSI when `use_color` is
/// false (box-drawing characters are still used — `NO_COLOR` means no color,
/// not no Unicode). Slice 11.
#[must_use]
pub fn panel(title: Option<&str>, body: &str, use_color: bool) -> String {
    const PAD: usize = 1;
    let lines: Vec<&str> = body.split('\n').collect();
    let content_w = lines
        .iter()
        .map(|line| visible_width(line))
        .max()
        .unwrap_or(0);
    let title_w = title.map_or(0, |title| title.chars().count());
    let max_inner = terminal_width().saturating_sub(2 + PAD * 2).max(8);
    let inner = content_w.max(title_w).min(max_inner);
    let span = inner + PAD * 2;

    let (border, title_color, reset) = if use_color {
        ("\x1b[38;5;245m", "\x1b[1;36m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    let pad = " ".repeat(PAD);

    let mut out = String::new();
    match title {
        Some(title) => {
            // ╭─ title ───────╮  (─ title  consumes 2 + title_w + 1 cells)
            let dashes = span.saturating_sub(2 + title_w + 1);
            out.push_str(&format!(
                "{border}╭─ {reset}{title_color}{title}{reset}{border} {dashes}╮{reset}\n",
                dashes = "─".repeat(dashes),
            ));
        }
        None => out.push_str(&format!("{border}╭{}╮{reset}\n", "─".repeat(span))),
    }
    for line in &lines {
        let fill = " ".repeat(inner.saturating_sub(visible_width(line)));
        out.push_str(&format!(
            "{border}│{reset}{pad}{line}{fill}{pad}{border}│{reset}\n"
        ));
    }
    out.push_str(&format!("{border}╰{}╯{reset}", "─".repeat(span)));
    out
}

/// Prefix every line of `body` with `prefix` (a left gutter / margin). `body`
/// may contain ANSI escapes; the prefix is applied verbatim. Slice 11.
#[must_use]
pub fn with_gutter(body: &str, prefix: &str) -> String {
    body.split('\n')
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        clear_runtime_theme, effective_theme, panel, resolve_theme, set_runtime_theme, strip_ansi,
        terminal_width, with_gutter, ColorTheme, MarkdownStreamState, Spinner, TerminalRenderer,
        ThemeSource,
    };

    #[test]
    fn resolves_gi_theme_names() {
        assert_eq!(ColorTheme::named("gi-dark"), Some(ColorTheme::gi_dark()));
        assert_eq!(ColorTheme::named("gi_light"), Some(ColorTheme::gi_light()));
        assert_eq!(ColorTheme::named("unknown"), None);
    }

    #[test]
    fn resolves_new_palettes_and_aliases() {
        assert_eq!(ColorTheme::named("matcha"), Some(ColorTheme::gi_matcha()));
        assert_eq!(ColorTheme::named("gi-sumi"), Some(ColorTheme::gi_sumi()));
        assert_eq!(ColorTheme::named("warm"), Some(ColorTheme::gi_sunrise()));
        // Each declared theme name resolves to a distinct palette.
        let palettes: Vec<_> = super::THEME_NAMES
            .iter()
            .map(|name| ColorTheme::named(name).expect("named theme"))
            .collect();
        assert_eq!(palettes.len(), super::THEME_NAMES.len());
        for (i, a) in palettes.iter().enumerate() {
            for b in &palettes[i + 1..] {
                assert_ne!(a, b, "palettes should be distinct");
            }
        }
    }

    #[test]
    fn terminal_width_is_clamped_to_readable_range() {
        let width = terminal_width();
        assert!((40..=100).contains(&width), "got {width}");
    }

    #[test]
    fn panel_frames_content_as_a_rectangle() {
        let rendered = panel(Some("Status"), "model: opus\nbranch: main", false);
        let lines: Vec<&str> = rendered.lines().collect();
        // Top has the title, bottom is a closing border, body rows are bordered.
        assert!(lines[0].starts_with("╭─ Status"));
        assert!(lines[0].ends_with('╮'));
        assert!(lines.last().unwrap().starts_with('╰'));
        assert!(lines.last().unwrap().ends_with('╯'));
        assert!(lines[1].starts_with('│') && lines[1].contains("model: opus"));
        // Every visible row is the same width (a true rectangle).
        let widths: Vec<usize> = lines
            .iter()
            .map(|line| strip_ansi(line).chars().count())
            .collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "ragged panel: {widths:?}"
        );
        // NO_COLOR variant carries no ANSI.
        assert!(!rendered.contains('\u{1b}'));
        // Colored variant does.
        assert!(panel(Some("Status"), "x", true).contains("\u{1b}["));
    }

    #[test]
    fn with_gutter_prefixes_every_line() {
        assert_eq!(with_gutter("a\nb", "▏ "), "▏ a\n▏ b");
    }

    #[test]
    fn prompt_glyph_and_header_are_themed_and_no_color_safe() {
        assert_eq!(super::prompt_glyph(false, None), "❯ ");
        assert!(super::prompt_glyph(true, None).contains('❯'));
        assert!(super::prompt_glyph(true, None).contains('\u{1b}'));

        // Header labels mode · agent; empty when both absent.
        assert_eq!(
            super::prompt_header(Some("reviewer"), Some("edit"), false),
            Some("╭─ edit · reviewer ─╮".to_string())
        );
        assert_eq!(
            super::prompt_header(Some("reviewer"), None, false),
            Some("╭─ reviewer ─╮".to_string())
        );
        assert_eq!(super::prompt_header(None, None, false), None);
        assert!(super::prompt_header(None, Some("plan"), true)
            .unwrap()
            .contains('\u{1b}'));
    }

    #[test]
    fn mode_accent_tints_non_default_modes() {
        // Default / empty stay neutral (no accent); the other modes get one.
        assert_eq!(super::mode_accent("default"), None);
        assert_eq!(super::mode_accent(""), None);
        assert!(super::mode_accent("plan").is_some());
        assert!(super::mode_accent("edit").is_some());
        assert!(super::mode_accent("mugen").is_some());

        // The mugen accent (loud red/magenta) shows up in the colored header SGR.
        let accent = super::mode_accent("mugen").unwrap();
        let super::Color::Rgb { r, g, b } = accent else {
            panic!("expected a truecolor accent");
        };
        let sgr = format!("\x1b[38;2;{r};{g};{b}m");
        assert!(super::prompt_header(Some("ss"), Some("mugen"), true)
            .unwrap()
            .contains(&sgr));
        // The glyph honors the accent too, and stays visible-width 2.
        let glyph = super::prompt_glyph(true, Some(accent));
        assert!(glyph.contains(&sgr));
        assert_eq!(super::strip_ansi(&glyph).chars().count(), 2);
    }

    #[test]
    fn theme_swatch_previews_colors_or_plain_name() {
        // Colored: truecolor swatches + name; NO_COLOR: just the name.
        let colored = super::theme_swatch("gi-matcha", true);
        assert!(colored.contains("\u{1b}[38;2;"));
        assert!(colored.contains("gi-matcha"));
        assert_eq!(super::theme_swatch("gi-matcha", false), "gi-matcha");
        // Unknown name passes through.
        assert_eq!(super::theme_swatch("nope", true), "nope");
    }

    #[test]
    fn theme_precedence_prefers_env_then_runtime_then_default() {
        // Env wins over a runtime override.
        assert_eq!(
            resolve_theme(Some("gi-light"), Some("gi-dark")),
            (Some("gi-light"), ThemeSource::Env)
        );
        // Runtime override applies when no env var is set.
        assert_eq!(
            resolve_theme(None, Some("gi-dark")),
            (Some("gi-dark"), ThemeSource::Config)
        );
        // Unknown names are ignored and fall through to the built-in default.
        assert_eq!(
            resolve_theme(Some("bogus"), None),
            (None, ThemeSource::Default)
        );
        // Aliases normalize to their canonical names.
        assert_eq!(
            resolve_theme(None, Some("light")),
            (Some("gi-light"), ThemeSource::Config)
        );
    }

    #[test]
    fn runtime_override_changes_default_theme() {
        // This test mutates a process-global slot; clear it on every exit path.
        assert!(set_runtime_theme("gi-light"));
        assert_eq!(ColorTheme::default(), ColorTheme::gi_light());
        assert_eq!(effective_theme(), ("gi-light", ThemeSource::Config));

        assert!(set_runtime_theme("dark"));
        assert_eq!(ColorTheme::default(), ColorTheme::gi_dark());

        // Unknown names leave the override untouched and report failure.
        assert!(!set_runtime_theme("nope"));
        assert_eq!(ColorTheme::default(), ColorTheme::gi_dark());

        clear_runtime_theme();
        assert_eq!(effective_theme().1, ThemeSource::Default);
    }

    #[test]
    fn renders_markdown_with_styling_and_lists() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output = terminal_renderer
            .render_markdown("# Heading\n\nThis is **bold** and *italic*.\n\n- item\n\n`code`");

        assert!(markdown_output.contains("Heading"));
        assert!(markdown_output.contains("• item"));
        assert!(markdown_output.contains("code"));
        assert!(markdown_output.contains('\u{1b}'));
    }

    #[test]
    fn renders_links_as_colored_markdown_labels() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output =
            terminal_renderer.render_markdown("See [Gi](https://example.com/docs) now.");
        let plain_text = strip_ansi(&markdown_output);

        assert!(plain_text.contains("[Gi](https://example.com/docs)"));
        assert!(markdown_output.contains('\u{1b}'));
    }

    #[test]
    fn highlights_fenced_code_blocks() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output =
            terminal_renderer.markdown_to_ansi("```rust\nfn hi() { println!(\"hi\"); }\n```");
        let plain_text = strip_ansi(&markdown_output);

        assert!(plain_text.contains("╭─ rust"));
        assert!(plain_text.contains("fn hi"));
        assert!(markdown_output.contains('\u{1b}'));
        assert!(markdown_output.contains("[48;5;236m"));
    }

    #[test]
    fn renders_ordered_and_nested_lists() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output =
            terminal_renderer.render_markdown("1. first\n2. second\n   - nested\n   - child");
        let plain_text = strip_ansi(&markdown_output);

        assert!(plain_text.contains("1. first"));
        assert!(plain_text.contains("2. second"));
        assert!(plain_text.contains("  • nested"));
        assert!(plain_text.contains("  • child"));
    }

    #[test]
    fn renders_tables_with_alignment() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output = terminal_renderer
            .render_markdown("| Name | Value |\n| ---- | ----- |\n| alpha | 1 |\n| beta | 22 |");
        let plain_text = strip_ansi(&markdown_output);
        let lines = plain_text.lines().collect::<Vec<_>>();

        assert_eq!(lines[0], "│ Name  │ Value │");
        assert_eq!(lines[1], "│───────┼───────│");
        assert_eq!(lines[2], "│ alpha │ 1     │");
        assert_eq!(lines[3], "│ beta  │ 22    │");
        assert!(markdown_output.contains('\u{1b}'));
    }

    #[test]
    fn streaming_state_waits_for_complete_blocks() {
        let renderer = TerminalRenderer::new();
        let mut state = MarkdownStreamState::default();

        assert_eq!(state.push(&renderer, "# Heading"), None);
        let flushed = state
            .push(&renderer, "\n\nParagraph\n\n")
            .expect("completed block");
        let plain_text = strip_ansi(&flushed);
        assert!(plain_text.contains("Heading"));
        assert!(plain_text.contains("Paragraph"));

        assert_eq!(state.push(&renderer, "```rust\nfn main() {}\n"), None);
        let code = state
            .push(&renderer, "```\n")
            .expect("closed code fence flushes");
        assert!(strip_ansi(&code).contains("fn main()"));
    }

    #[test]
    fn streaming_state_holds_outer_fence_with_nested_inner_fence() {
        let renderer = TerminalRenderer::new();
        let mut state = MarkdownStreamState::default();

        assert_eq!(
            state.push(&renderer, "````markdown\n```rust\nfn inner() {}\n"),
            None,
            "inner triple backticks must not close the outer four-backtick fence"
        );
        assert_eq!(
            state.push(&renderer, "```\n"),
            None,
            "closing the inner fence must not flush the outer fence"
        );
        let flushed = state
            .push(&renderer, "````\n")
            .expect("closing the outer four-backtick fence flushes the buffered block");
        let plain_text = strip_ansi(&flushed);
        assert!(plain_text.contains("fn inner()"));
        assert!(plain_text.contains("```rust"));
    }

    #[test]
    fn streaming_state_distinguishes_backtick_and_tilde_fences() {
        let renderer = TerminalRenderer::new();
        let mut state = MarkdownStreamState::default();

        assert_eq!(state.push(&renderer, "~~~text\n"), None);
        assert_eq!(
            state.push(&renderer, "```\nstill inside tilde fence\n"),
            None,
            "a backtick fence cannot close a tilde-opened fence"
        );
        assert_eq!(state.push(&renderer, "```\n"), None);
        let flushed = state
            .push(&renderer, "~~~\n")
            .expect("matching tilde marker closes the fence");
        let plain_text = strip_ansi(&flushed);
        assert!(plain_text.contains("still inside tilde fence"));
    }

    #[test]
    fn renders_nested_fenced_code_block_preserves_inner_markers() {
        let terminal_renderer = TerminalRenderer::new();
        let markdown_output =
            terminal_renderer.markdown_to_ansi("````markdown\n```rust\nfn nested() {}\n```\n````");
        let plain_text = strip_ansi(&markdown_output);

        assert!(plain_text.contains("╭─ markdown"));
        assert!(plain_text.contains("```rust"));
        assert!(plain_text.contains("fn nested()"));
    }

    #[test]
    fn spinner_advances_frames() {
        let terminal_renderer = TerminalRenderer::new();
        let mut spinner = Spinner::new();
        let mut out = Vec::new();
        spinner
            .tick("Working", terminal_renderer.color_theme(), &mut out)
            .expect("tick succeeds");
        spinner
            .tick("Working", terminal_renderer.color_theme(), &mut out)
            .expect("tick succeeds");

        let output = String::from_utf8_lossy(&out);
        assert!(output.contains("Working"));
    }
}
