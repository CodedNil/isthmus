use crate::{
    app::{Background, CantusApp, fetch_json},
    config::SearchProvider,
    interaction::key,
    platform::{self, DesktopApp},
    render::{
        GAP, PADDING, Program, TEXT_COLOR, UiContext,
        sdf::{deform, glass, presence},
    },
};
use fend_core::Context;
use isthmus::prelude::*;
use isthmus_sdf::prelude::*;
use serde::Deserialize;
use std::{collections::HashMap, error::Error, ops::Range, sync::OnceLock};
use unicode_segmentation::UnicodeSegmentation;

const PANEL_WIDTH: f32 = 520.0;
const ROW_HEIGHT: f32 = 50.0;
const HEADER_HEIGHT: f32 = ROW_HEIGHT + PADDING;
pub const BACKGROUND_RADIUS: i32 = 16;
/// Matched-app/calculator rows shown below the search bar.
pub const MAX_VISIBLE: usize = 8;

/// Side of the square icon tile at the left of every row.
const ICON_SIZE: f32 = 32.0;
/// Icons, badge outlines and the magnifier all share one grey.
const ICON_COLOR: Vec3 = Vec3::splat(0.58);
const ACCENT_COLOR: Vec3 = vec3(0.44, 0.40, 0.80);
const DETAIL_COLOR: Vec3 = Vec3::new(0.56, 0.63, 0.86);
const MUTED_COLOR: Vec3 = Vec3::new(0.52, 0.55, 0.64);
const CALCULATOR_ICON: u32 = 1;
const SEARCH_ICON: u32 = 2;

/// Currency rates relative to USD, fetched once and read by fend for currency conversions.
static EXCHANGE_RATES: OnceLock<HashMap<String, f64>> = OnceLock::new();

/// Coverage of a magnifying glass centered on the origin.
fn magnifier_icon(point: Vec2) -> f32 {
    Shape::circle(Vec2::ZERO, 6.2).stroke(2.1).union(Shape::segment(vec2(4.6, 4.6), vec2(8.8, 8.8), 2.1)).fill_at(point)
}

/// Colour of the calculator badge shown beside a fend answer.
fn calculator_icon(point: Vec2) -> Vec4 {
    let badge = Shape::rounded_rect(Vec2::splat(26.0), 9.0).fill_at(point);
    let equals = Shape::segment(vec2(-4.3, 0.0), vec2(4.3, 0.0), 2.2).fill_at(vec2(point.x, point.y.abs() - 3.1));
    ACCENT_COLOR.lerp(Vec3::splat(0.96), equals).extend(badge)
}

/// "↵" or "⇧" glyph coverage, drawn around the origin.
fn key_glyph(point: Vec2, shift: bool) -> f32 {
    if shift {
        Shape::segment(vec2(0.0, -4.0), vec2(-3.4, 0.2), 1.6)
            .union(Shape::segment(vec2(0.0, -4.0), vec2(3.4, 0.2), 1.6))
            .union(Shape::segment(vec2(0.0, -0.6), vec2(0.0, 4.0), 1.6))
            .fill_at(point)
    } else {
        Shape::segment(vec2(3.4, -3.6), vec2(3.4, 1.8), 1.6)
            .union(Shape::segment(vec2(3.4, 1.8), vec2(-2.6, 1.8), 1.6))
            .union(Shape::segment(vec2(-2.6, 1.8), vec2(0.2, -0.8), 1.6))
            .union(Shape::segment(vec2(-2.6, 1.8), vec2(0.2, 4.4), 1.6))
            .fill_at(point)
    }
}

/// Colour of one key badge; `half_width` of 0 leaves the slot empty.
fn action_badge(point: Vec2, half_width: f32, shift: bool) -> Vec4 {
    if half_width <= 0.0 {
        return Vec4::ZERO;
    }
    let sample = Shape::rounded_rect(vec2(half_width * 2.0, 21.0), 6.0).sample_at(point);
    let (body, edge) = (sample.fill(), sample.band(-0.65..0.65));
    let glyph = if shift {
        key_glyph(point + vec2(8.5, 0.0), true).max(key_glyph(point - vec2(7.5, 0.0), false))
    } else {
        key_glyph(point, false)
    };
    let color = Vec3::splat(0.27).lerp(ICON_COLOR, edge).lerp(TEXT_COLOR, glyph);
    color.extend(body.max(edge).max(glyph))
}

#[derive(Default)]
pub struct TextField {
    pub text: String,
    /// Byte offset of the caret, and of the other end of the selection.
    cursor: usize,
    anchor: usize,
    /// Frame time the caret blink last restarted at, so typing keeps it solid.
    blink_start: f32,
    /// Set by an edit, consumed by the next context to restart the blink.
    touched: bool,
}

impl TextField {
    pub fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }

    pub fn set_cursor(&mut self, index: usize, select: bool) {
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .find(|&offset| offset >= index)
            .unwrap_or(self.text.len());
        if !select {
            self.anchor = self.cursor;
        }
        self.touched = true;
    }

    /// Where the caret lands moving one character in `forward`'s direction.
    fn step(&self, forward: bool) -> usize {
        if forward {
            self.cursor + self.text[self.cursor..].graphemes(true).next().map_or(0, str::len)
        } else {
            self.cursor - self.text[..self.cursor].graphemes(true).next_back().map_or(0, str::len)
        }
    }

    pub fn insert(&mut self, insertion: &str) {
        let range = self.selection();
        let remaining = 64usize.saturating_sub(self.text.chars().count() - self.text[range.clone()].chars().count());
        let insertion = &insertion[..insertion.char_indices().nth(remaining).map_or(insertion.len(), |(end, _)| end)];
        self.text.replace_range(range.clone(), insertion);
        self.set_cursor(range.start + insertion.len(), false);
    }

    /// Deletes the selection, or one character in `forward`'s direction.
    pub fn erase(&mut self, forward: bool) {
        if self.selection().is_empty() {
            self.set_cursor(self.step(forward), true);
        }
        self.insert("");
    }

    /// Moves the caret, collapsing an existing selection unless `select` extends it.
    pub fn move_cursor(&mut self, forward: bool, select: bool) {
        let range = self.selection();
        let target = if select || range.is_empty() {
            self.step(forward)
        } else if forward {
            range.end
        } else {
            range.start
        };
        self.set_cursor(target, select);
    }
}

#[derive(Default)]
pub struct LauncherState {
    pub open: bool = Self::ALWAYS_OPEN,
    pub session: u64,
    pub field: TextField,
    entries: Vec<Entry>,
    /// Index of the highlighted entry, which enter and shift+enter act on.
    pub selected: usize,
    /// Text waiting to be put on the system clipboard by the platform layer.
    pub pending_copy: Option<String>,
    calc: Context,
    apps: Vec<DesktopApp>,
    providers: Vec<SearchEngine>,
}

struct SearchEngine {
    config: SearchProvider,
    label: String,
    icon: Option<Image>,
}

enum Entry {
    Answer(String),
    App(usize),
    Search(usize),
}

#[derive(Default)]
struct EntryView<'a> {
    image: Option<&'a Image>,
    icon_kind: u32 = SEARCH_ICON,
    name: &'a str,
    detail: &'a str,
    action: &'static str = "Open",
    alternate: Option<&'a str>,
}

impl LauncherState {
    const ALWAYS_OPEN: bool = cfg!(target_arch = "wasm32");

    pub(crate) fn new(background: &Background, providers: impl IntoIterator<Item = SearchProvider>) -> Self {
        let mut calc = Context::new();
        let rates_http = background.http.clone();
        platform::spawn_task(async move {
            #[derive(Deserialize)]
            struct Rates {
                rates: HashMap<String, f64>,
            }
            if let Ok(rates) = fetch_json::<Rates>(rates_http.get("https://open.er-api.com/v6/latest/USD")).await {
                let _ = EXCHANGE_RATES.set(rates.rates);
            }
        });
        calc.set_exchange_rate_handler_v2(ExchangeRates);
        let providers = providers
            .into_iter()
            .map(|config| SearchEngine { label: format!("Search with {}", config.name), config, icon: None })
            .collect::<Vec<_>>();
        background.spawn_update(async move {
            let mut apps = platform::desktop_apps();
            apps.sort_by_key(|app| app.name.to_lowercase());
            Some(move |app: &mut CantusApp| {
                app.launcher.apps = apps;
                app.launcher.refresh_matches();
            })
        });
        for (index, provider) in providers.iter().enumerate() {
            let icon = provider.config.icon.clone();
            let http = background.http.clone();
            background.spawn_update(async move {
                let bytes = http.get(icon).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?;
                let icon = platform::decode_icon(&bytes)?;
                Some(move |app: &mut CantusApp| {
                    app.launcher.providers[index].icon = Some(icon);
                })
            });
        }
        Self { calc, providers, ..Default::default() }
    }

    pub fn toggle(&mut self) {
        self.session = self.session.wrapping_add(1);
        self.open = Self::ALWAYS_OPEN || !self.open;
        self.field.text.clear();
        self.field.set_cursor(0, false);
        self.refresh_matches();
    }

    /// Runs one edit against the search field, then re-runs the query.
    pub fn edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        edit(&mut self.field);
        self.refresh_matches();
    }

    pub(crate) fn input(&mut self, key: &str, shift: bool, control: bool) -> bool {
        if !self.open {
            return false;
        }
        match key {
            "Escape" => self.open = Self::ALWAYS_OPEN,
            "Enter" => self.activate(self.selected, shift),
            "ArrowUp" => self.move_selection(-1),
            "ArrowDown" => self.move_selection(1),
            "Backspace" => self.edit(|field| field.erase(false)),
            "Delete" => self.edit(|field| field.erase(true)),
            "ArrowLeft" => self.field.move_cursor(false, shift),
            "ArrowRight" => self.field.move_cursor(true, shift),
            "Home" => self.field.set_cursor(0, shift),
            "End" => self.field.set_cursor(self.field.text.len(), shift),
            "a" | "A" if control => {
                self.field.anchor = 0;
                self.field.set_cursor(self.field.text.len(), true);
            }
            "c" | "C" | "x" | "X" if control => {
                self.pending_copy = Some(self.field.text[self.field.selection()].to_owned());
                if key.eq_ignore_ascii_case("x") {
                    self.edit(|field| field.insert(""));
                }
            }
            _ if !control && key.chars().count() == 1 && !key.chars().any(char::is_control) => {
                self.edit(|field| field.insert(key));
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn paste(&mut self, session: u64, text: &str) {
        if self.open && self.session == session {
            self.edit(|field| field.insert(&text.replace(['\n', '\r'], " ")));
        }
    }

    pub fn refresh_matches(&mut self) {
        let (provider, query) = self.search_query();
        let explicit_search = provider.is_some();
        let query = query.to_owned();
        self.entries = (!explicit_search && query.len() >= 4)
            .then(|| fend_core::evaluate(&query, &mut self.calc).ok())
            .flatten()
            .map(|result| result.get_main_result().to_owned())
            .filter(|result| !result.is_empty() && result != &query)
            .map(Entry::Answer)
            .into_iter()
            .collect();

        let lower_query = query.to_lowercase();
        let has_search = !self.providers.is_empty() && (!lower_query.is_empty() || explicit_search);
        let visible = MAX_VISIBLE - self.entries.len() - usize::from(has_search);
        let mut scored = self
            .apps
            .iter()
            .enumerate()
            .filter(|_| !explicit_search)
            .filter_map(|(index, app)| {
                let name = app.name.to_lowercase();
                name.contains(&lower_query).then(|| (index, name.starts_with(&lower_query)))
            })
            .collect::<Vec<_>>();
        scored.sort_by_key(|&(_, prefix_match)| !prefix_match);
        self.entries.extend(scored.into_iter().take(visible).map(|(index, _)| Entry::App(index)));
        if has_search {
            self.entries.push(Entry::Search(provider.unwrap_or_default()));
        }
        self.selected = 0;
    }

    /// Moves the highlight by `delta` rows, stopping at either end.
    pub fn move_selection(&mut self, delta: i32) {
        self.selected = self.selected.saturating_add_signed(delta as isize).min(self.entries.len().saturating_sub(1));
    }

    /// Runs row `index`'s action — its alternative one when `alternate` is set — then dismisses.
    pub fn activate(&mut self, index: usize, alternate: bool) {
        match self.entries.get(index) {
            Some(Entry::App(index)) => {
                let app = &self.apps[*index];
                platform::spawn(app.action.as_ref().filter(|_| alternate).map_or(&app.exec, |(_, exec)| exec));
            }
            Some(Entry::Answer(answer)) => self.pending_copy = Some(answer.to_owned()),
            Some(Entry::Search(index)) => {
                let engine = &self.providers[*index];
                let terms = self.search_query().1;
                let encoded = form_urlencoded::byte_serialize(terms.as_bytes()).collect::<String>();
                platform::open_url(&engine.config.url.replace("{searchTerms}", &encoded));
            }
            None => return,
        }
        self.open = Self::ALWAYS_OPEN;
        self.field.text.clear();
        self.field.set_cursor(0, false);
        self.refresh_matches();
    }

    fn search_query(&self) -> (Option<usize>, &str) {
        let query = self.field.text.trim();
        self.providers
            .iter()
            .position(|provider| {
                !provider.config.alias.is_empty()
                    && (query == provider.config.alias
                        || query
                            .strip_prefix(provider.config.alias.as_str())
                            .is_some_and(|rest| rest.starts_with(char::is_whitespace)))
            })
            .map_or((None, query), |index| (Some(index), query[self.providers[index].config.alias.len()..].trim()))
    }

    pub(crate) fn bounds(&self, screen_size: Vec2) -> (Vec2, Vec2) {
        let rows = self.entries.len();
        let height = HEADER_HEIGHT + PADDING * 2.0 + rows as f32 * ROW_HEIGHT + rows.saturating_sub(1) as f32 * GAP;
        let size = vec2(PANEL_WIDTH, height);
        ((screen_size - size) * 0.5, size)
    }
}

struct ExchangeRates;

impl fend_core::ExchangeRateFnV2 for ExchangeRates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn Error + Send + Sync>> {
        EXCHANGE_RATES
            .get()
            .and_then(|rates| rates.get(currency))
            .copied()
            .ok_or_else(|| "exchange rates not loaded yet".into())
    }
}

impl LauncherState {
    /// Draws the panel and its search field, then the rows beneath it.
    pub fn show(&mut self, context: &mut UiContext) {
        if !self.open {
            return;
        }
        let (origin, size) = self.bounds(context.frame.screen_size);
        let rect = Rect::new(origin, origin + size);
        let panel = Shape::rounded_rect(rect, BACKGROUND_RADIUS as f32);
        let screen = Rect::new(Vec2::ZERO, context.frame.screen_size);
        context.interaction.input_region(screen);
        let backdrop = context.interaction.interact("launcher-backdrop", Shape::rectangle(screen).difference(panel));
        if !Self::ALWAYS_OPEN && backdrop.clicked {
            self.open = false;
            return;
        }

        self.show_search(context, rect);
        self.show_entries(context, origin);
    }

    fn show_search(&mut self, context: &mut UiContext, rect: Rect) {
        let origin = rect.min;
        let (left, right) = (PADDING + 34.0, rect.size().x - PADDING);
        if self.field.touched {
            self.field.touched = false;
            self.field.blink_start = context.frame.time;
        }
        let empty = self.field.text.is_empty();
        let query = if empty { "Search anything…" } else { &self.field.text };
        let line = context.frame.resources.line(query, 18.0, 600.0).translated(vec2(left, HEADER_HEIGHT * 0.5));
        let selection_range = self.field.selection();
        let blink = ((context.frame.time - self.field.blink_start) * 1.4).fract();
        let (caret, selection): (Vec2, Vec2) = {
            let text = &mut *context.frame.resources;
            let mut at =
                |offset: usize| (left + text.shape(&self.field.text[..offset], 18.0, 600.0).text.width).min(right);
            let caret = vec2(at(self.field.cursor), blink.smoothstep(0.62, 0.5));
            let selection = if selection_range.is_empty() {
                Vec2::ZERO
            } else {
                vec2(at(selection_range.start), at(selection_range.end))
            };
            (caret, selection)
        };

        shader!(
            context
                .frame
                .upload({
                    let caret: Vec2;
                    let selection: Vec2;
                    let rect: Rect;
                })
                .primitive(|frame| deform(Shape::rounded_rect(rect, BACKGROUND_RADIUS as f32), frame))
                .fragment(|_, surface| {
                    let point = surface.content - rect.min;
                    let mut color = Vec3::splat(0.09)
                        .lerp(ICON_COLOR, magnifier_icon(point - vec2(PADDING + 11.0, HEADER_HEIGHT * 0.5)));
                    let selection_width = selection.y - selection.x;
                    let highlight = Shape::rounded_rect(vec2(selection_width, 26.0), 3.0)
                        .fill_at(point - vec2(f32::midpoint(selection.x, selection.y), HEADER_HEIGHT * 0.5));
                    color = color.lerp(vec3(0.24, 0.28, 0.52), highlight * presence(selection_width));
                    let caret_mask = Shape::pill(vec2(1.8, 24.0)).fill_at(point - vec2(caret.x, HEADER_HEIGHT * 0.5));
                    color = color.lerp(TEXT_COLOR, caret_mask * caret.y);
                    glass(surface, color) * vec4(1.0, 1.0, 1.0, 0.82)
                })
        );
        context.paint_text(
            rect,
            BACKGROUND_RADIUS as f32,
            line.translated(origin),
            (if empty { MUTED_COLOR } else { TEXT_COLOR }).extend(1.0),
        );
    }

    /// Draws and interacts with calculator, application and search rows in visual order.
    fn show_entries(&mut self, context: &mut UiContext, origin: Vec2) {
        let (x, width) = (origin.x + PADDING, PANEL_WIDTH - PADDING * 2.0);
        let text_left = ROW_HEIGHT * 0.5 + ICON_SIZE * 0.5 + GAP * 2.0;

        let mut activated = None;
        for index in 0..self.entries.len() {
            let y = origin.y + HEADER_HEIGHT + PADDING + index as f32 * (ROW_HEIGHT + GAP);
            let pill = Rect::new(vec2(x, y), vec2(x + width, y + ROW_HEIGHT));
            let identity = match &self.entries[index] {
                Entry::App(index) => key(("app", &self.apps[*index].exec)),
                Entry::Answer(answer) => key(("answer", answer)),
                Entry::Search(index) => key(("search", &self.providers[*index].config.url, self.search_query().1)),
            };
            let response = context.interaction.interact(identity, Shape::pill(pill));
            if response.hovered {
                self.selected = index;
            }
            if response.clicked {
                activated = Some(index);
            }
            let entry = match &self.entries[index] {
                Entry::App(index) => {
                    let app = &self.apps[*index];
                    EntryView {
                        image: app.icon.as_ref(),
                        name: &app.name,
                        detail: &app.comment,
                        alternate: app.action.as_ref().map(|(label, _)| label.as_str()),
                        ..Default::default()
                    }
                }
                Entry::Answer(answer) => {
                    EntryView { icon_kind: CALCULATOR_ICON, name: answer, action: "Copy", ..Default::default() }
                }
                Entry::Search(index) => {
                    let engine = &self.providers[*index];
                    EntryView {
                        image: engine.icon.as_ref(),
                        name: &engine.label,
                        detail: self.search_query().1,
                        action: "Search",
                        ..Default::default()
                    }
                }
            };

            let mut edge = width - ROW_HEIGHT * 0.5;
            let mut badge = |label: Option<&str>, width: f32| {
                let Some(label) = label.filter(|_| self.selected == index) else {
                    return (Vec2::ZERO, Text::default());
                };
                let badge = vec2(edge - width * 0.5, width * 0.5);
                edge -= width + GAP;
                let line = context.frame.resources.line(label, 13.0, 600.0).right(vec2(edge, ROW_HEIGHT * 0.5));
                edge -= line.width + GAP * 2.0;
                (badge, line)
            };
            let (enter_badge, action_line) = badge(Some(entry.action), 27.0);
            let (alternate_badge, alternate_line) = badge(entry.alternate, 42.0);

            let (name_y, detail_y) =
                if entry.detail.is_empty() { (ROW_HEIGHT * 0.5, 0.0) } else { (ROW_HEIGHT * 0.34, ROW_HEIGHT * 0.68) };
            let name_line = context.frame.resources.line(entry.name, 16.0, 700.0).translated(vec2(text_left, name_y));
            let detail_line =
                context.frame.resources.line(entry.detail, 13.0, 600.0).translated(vec2(text_left, detail_y));

            shader!(
                context
                    .frame
                    .upload({
                        let icon_kind: u32 = if entry.image.is_some() { 0 } else { entry.icon_kind };
                        let enter_badge: Vec2;
                        let alternate_badge: Vec2;
                        let pill: Rect;
                    })
                    .primitive(|frame| deform(Shape::pill(pill), frame))
                    .fragment(|_, surface| {
                        let mut color = Vec3::splat(0.15)
                            .lerp(Vec3::splat(0.235), presence(enter_badge.y))
                            .lerp(Vec3::splat(0.3), (surface.bulge / 8.0).min(1.0));
                        let icon_point = pill.local(surface.pixel) + vec2((pill.size().x - pill.size().y) * 0.5, 0.0);
                        if icon_kind == CALCULATOR_ICON {
                            color = source_over(calculator_icon(icon_point), color.extend(1.0)).truncate();
                        } else if icon_kind == SEARCH_ICON {
                            color = color.lerp(ICON_COLOR, magnifier_icon(icon_point));
                        }
                        let point = surface.content - pill.min;
                        let paint_badge = |color: Vec3, badge: Vec2, shift: bool| {
                            let ink = action_badge(point - vec2(badge.x, ROW_HEIGHT * 0.5), badge.y, shift);
                            source_over(ink, color.extend(1.0)).truncate()
                        };
                        color = paint_badge(color, enter_badge, false);
                        color = paint_badge(color, alternate_badge, true);
                        glass(surface, color) * vec4(1.0, 1.0, 1.0, 0.7)
                    })
            );
            if let Some(image) = entry.image {
                shader!(
                    context
                        .frame
                        .upload({
                            let image: &Image;
                            let rect: Rect = Rect::from_center_size(
                                pill.center() - vec2((pill.size().x - pill.size().y) * 0.5, 0.0),
                                Vec2::splat(ICON_SIZE),
                            );
                        })
                        .primitive(rect)
                        .fragment(|_, surface| image.sample(surface.uv))
                );
            }

            let origin = vec2(x, y);
            for (line, color) in [
                (name_line, TEXT_COLOR),
                (detail_line, DETAIL_COLOR),
                (action_line, MUTED_COLOR),
                (alternate_line, MUTED_COLOR),
            ] {
                context.paint_text(pill, ROW_HEIGHT * 0.5, line.translated(origin), color.extend(1.0));
            }
        }
        if let Some(index) = activated {
            self.activate(index, false);
        }
    }
}
