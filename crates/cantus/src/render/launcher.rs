use crate::{
    app::{Background, update},
    config::SearchProvider,
    interaction::Rect,
    platform::{DesktopApp, Platform},
    render::{
        Fragment, GAP, Globals, PADDING, TEXT_COLOR, TextFragment, UiContext,
        sdf::{PILL_MARGIN, SurfaceSample, VISIBLE_ALPHA, cantus_surface, presence, sample_pill},
    },
};
use fend_core::Context;
use isthmus::{
    ColorExt as _, Float as _, Image, Quad, Sdf,
    geometry::text,
    glam::{Vec2, Vec3, Vec4, vec2, vec3},
    shader,
    spirv_std::arch::kill,
};
use reqwest::Client;
use std::{collections::HashMap, error::Error, ops::Range, sync::OnceLock};

const PANEL_WIDTH: f32 = 520.0;
const ROW_HEIGHT: f32 = 50.0;
const HEADER_HEIGHT: f32 = ROW_HEIGHT + PADDING;
pub const BACKGROUND_RADIUS: i32 = 16;
/// Matched-app/calculator rows shown below the search bar.
pub const MAX_VISIBLE: usize = 8;

/// Side of the square icon tile at the left of every row.
const ICON_SIZE: f32 = 32.0;
const BADGE_HEIGHT: f32 = 21.0;
/// Icons, badge outlines and the magnifier all share one grey.
const ICON_COLOR: Vec3 = Vec3::splat(0.58);
const ACCENT_COLOR: Vec3 = vec3(0.44, 0.40, 0.80);
const ENTER_BADGE_WIDTH: f32 = 27.0;
const ALTERNATE_BADGE_WIDTH: f32 = 42.0;
const DETAIL_COLOR: Vec4 = Vec4::new(0.56, 0.63, 0.86, 1.0);
const MUTED_COLOR: Vec4 = Vec4::new(0.52, 0.55, 0.64, 1.0);
const CALCULATOR_ICON: u32 = 1;
const SEARCH_ICON: u32 = 2;

/// Currency rates relative to USD, fetched once and read by fend for currency conversions.
static EXCHANGE_RATES: OnceLock<HashMap<String, f64>> = OnceLock::new();

/// Coverage of a magnifying glass centered on the origin.
fn magnifier_icon(point: Vec2) -> f32 {
    let ring = Sdf::new(point.length() - 6.2).stroke(1.05);
    let handle = Sdf::segment(point, vec2(4.6, 4.6), vec2(8.8, 8.8)).stroke(1.05);
    ring.max(handle)
}

/// Straight color and coverage of the calculator badge shown beside a fend answer.
fn calculator_icon(point: Vec2) -> Vec4 {
    let badge = Sdf::rounded_box(point, Vec2::splat(13.0), 9.0).fill();
    let bar = |offset: f32| Sdf::rounded_box(point - vec2(0.0, offset), vec2(5.4, 1.1), 1.1).fill();
    let equals = bar(-3.1).max(bar(3.1));
    ACCENT_COLOR.lerp(Vec3::splat(0.96), equals).extend(badge)
}

/// "↵" or "⇧" glyph coverage, drawn around the origin.
fn key_glyph(point: Vec2, shift: bool) -> f32 {
    let distance = if shift {
        Sdf::segment(point, vec2(0.0, -4.0), vec2(-3.4, 0.2))
            .union(Sdf::segment(point, vec2(0.0, -4.0), vec2(3.4, 0.2)))
            .union(Sdf::segment(point, vec2(0.0, -0.6), vec2(0.0, 4.0)))
    } else {
        Sdf::segment(point, vec2(3.4, -3.6), vec2(3.4, 1.8))
            .union(Sdf::segment(point, vec2(3.4, 1.8), vec2(-2.6, 1.8)))
            .union(Sdf::segment(point, vec2(-2.6, 1.8), vec2(0.2, -0.8)))
            .union(Sdf::segment(point, vec2(-2.6, 1.8), vec2(0.2, 4.4)))
    };
    distance.stroke(0.8)
}

/// Straight color and coverage of one key badge; `half_width` of 0 leaves the slot empty.
fn action_badge(point: Vec2, half_width: f32, shift: bool) -> Vec4 {
    if half_width <= 0.0 {
        return Vec4::ZERO;
    }
    let outline = Sdf::rounded_box(point, vec2(half_width, BADGE_HEIGHT * 0.5), 6.0);
    let (body, edge) = (outline.fill(), outline.stroke(0.65));
    let glyph = if shift {
        key_glyph(point + vec2(8.5, 0.0), true).max(key_glyph(point - vec2(7.5, 0.0), false))
    } else {
        key_glyph(point, false)
    };
    let color = Vec3::splat(0.27).lerp(ICON_COLOR, edge).lerp(TEXT_COLOR, glyph);
    color.extend(body.max(edge).max(glyph))
}

fn sample_launcher(quad: Quad, pixel: Vec2, globals: Globals, time: f32) -> SurfaceSample {
    cantus_surface(quad, pixel, globals, time, |point| {
        Sdf::rounded_box(quad.local(point), quad.size * 0.5, BACKGROUND_RADIUS as f32)
    })
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
    pub fn clear(&mut self) {
        self.text.clear();
        self.set_cursor(0, false);
    }

    pub const fn selection(&self) -> Range<usize> {
        if self.cursor < self.anchor { self.cursor..self.anchor } else { self.anchor..self.cursor }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    pub const fn set_cursor(&mut self, index: usize, select: bool) {
        self.cursor = index;
        if !select {
            self.anchor = index;
        }
        self.touched = true;
    }

    pub const fn select_all(&mut self) {
        self.anchor = 0;
        self.set_cursor(self.text.len(), true);
    }

    /// Where the caret lands moving one character in `forward`'s direction.
    fn step(&self, forward: bool) -> usize {
        let (before, after) = self.text.split_at(self.cursor);
        if forward {
            self.cursor + after.chars().next().map_or(0, char::len_utf8)
        } else {
            self.cursor - before.chars().next_back().map_or(0, char::len_utf8)
        }
    }

    pub fn insert(&mut self, insertion: &str) {
        let range = self.selection();
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

#[derive(Clone, Copy)]
pub enum LauncherKey {
    Escape,
    Activate,
    Up,
    Down,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    SelectAll,
    Copy,
    Cut,
}

pub struct LauncherState {
    pub open: bool,
    pub session: u64,
    pub field: TextField,
    pub matches: Vec<usize>,
    /// The fend answer for the current query, if any.
    pub calc_result: Option<String>,
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

enum LauncherEntry<'a> {
    Answer(&'a str),
    App(&'a DesktopApp),
    Search(&'a SearchEngine),
}

struct EntryView<'a> {
    icon: EntryIcon<'a>,
    name: &'a str,
    detail: &'a str,
    action: &'static str,
    alternate: Option<&'a str>,
}

enum EntryIcon<'a> {
    Image(&'a Image),
    Calculator,
    Search,
}

impl<'a> LauncherEntry<'a> {
    fn view(self, search: &'a str) -> EntryView<'a> {
        match self {
            Self::App(app) => EntryView {
                icon: app.icon.as_ref().map_or(EntryIcon::Search, EntryIcon::Image),
                name: &app.name,
                detail: &app.comment,
                action: "Open",
                alternate: app.action.as_ref().map(|(label, _)| label.as_str()),
            },
            Self::Answer(answer) => {
                EntryView { icon: EntryIcon::Calculator, name: answer, detail: "", action: "Copy", alternate: None }
            }
            Self::Search(engine) => EntryView {
                icon: engine.icon.as_ref().map_or(EntryIcon::Search, EntryIcon::Image),
                name: &engine.label,
                detail: search,
                action: "Search",
                alternate: None,
            },
        }
    }
}

impl LauncherState {
    pub(crate) fn new(
        background: &Background,
        http: &Client,
        providers: impl IntoIterator<Item = SearchProvider>,
    ) -> Self {
        let mut calc = Context::new();
        let rates_http = http.clone();
        background.spawn(async move {
            #[derive(serde::Deserialize)]
            struct Rates {
                rates: HashMap<String, f64>,
            }
            if let Ok(response) = rates_http
                .get("https://open.er-api.com/v6/latest/USD")
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                && let Ok(rates) = response.json::<Rates>().await
            {
                let _ = EXCHANGE_RATES.set(rates.rates);
            }
        });
        calc.set_exchange_rate_handler_v2(ExchangeRates);
        let providers = providers
            .into_iter()
            .map(|config| SearchEngine { label: format!("Search with {}", config.name), config, icon: None })
            .collect::<Vec<_>>();
        background.spawn_update(async move {
            let mut apps = Platform::desktop_apps();
            apps.sort_by_key(|app| app.name.to_lowercase());
            Some(update(move |app| {
                app.launcher.apps = apps;
                app.launcher.refresh_matches();
            }))
        });
        for (index, provider) in providers.iter().enumerate() {
            let icon = provider.config.icon.clone();
            let http = http.clone();
            background.spawn_update(async move {
                let icon = Platform::provider_icon(http, icon).await?;
                Some(update(move |app| {
                    app.launcher.providers[index].icon = Some(icon);
                }))
            });
        }
        Self {
            open: false,
            session: 0,
            field: TextField::default(),
            matches: Vec::new(),
            calc_result: None,
            selected: 0,
            pending_copy: None,
            calc,
            apps: Vec::new(),
            providers,
        }
    }

    /// Opens or closes the launcher with a fresh query.
    pub fn toggle(&mut self) {
        self.session = self.session.wrapping_add(1);
        self.open = !self.open;
        self.field.clear();
        self.refresh_matches();
    }

    /// Runs one edit against the search field, then re-runs the query.
    pub fn edit(&mut self, edit: impl FnOnce(&mut TextField)) {
        edit(&mut self.field);
        self.refresh_matches();
    }

    pub(crate) fn key(&mut self, key: LauncherKey, shift: bool) {
        match key {
            LauncherKey::Escape => self.open = false,
            LauncherKey::Activate => self.activate(self.selected, shift),
            LauncherKey::Up => self.move_selection(-1),
            LauncherKey::Down => self.move_selection(1),
            LauncherKey::Backspace => self.edit(|field| field.erase(false)),
            LauncherKey::Delete => self.edit(|field| field.erase(true)),
            LauncherKey::Left => self.field.move_cursor(false, shift),
            LauncherKey::Right => self.field.move_cursor(true, shift),
            LauncherKey::Home => self.field.set_cursor(0, shift),
            LauncherKey::End => self.field.set_cursor(self.field.text.len(), shift),
            LauncherKey::SelectAll => self.field.select_all(),
            LauncherKey::Copy | LauncherKey::Cut => {
                self.pending_copy = Some(self.field.selected_text().to_owned());
                if matches!(key, LauncherKey::Cut) {
                    self.edit(|field| field.insert(""));
                }
            }
        }
    }

    pub fn refresh_matches(&mut self) {
        let (provider, query) = self.search_query();
        let explicit_search = provider.is_some();
        let query = query.to_owned();
        self.calc_result = (!explicit_search && query.len() >= 4)
            .then(|| fend_core::evaluate(&query, &mut self.calc).ok())
            .flatten()
            .map(|result| result.get_main_result().to_owned())
            .filter(|result| !result.is_empty() && result != &query);

        let lower_query = query.to_lowercase();
        let has_search = !self.providers.is_empty() && (!lower_query.is_empty() || explicit_search);
        let visible = MAX_VISIBLE - usize::from(self.calc_result.is_some()) - usize::from(has_search);
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
        self.matches = scored.into_iter().take(visible).map(|(index, _)| index).collect();
        self.selected = 0;
    }

    pub fn entry_count(&self) -> usize {
        usize::from(self.calc_result.is_some()) + self.matches.len() + usize::from(self.search_provider().is_some())
    }

    fn entry(&self, mut row: usize) -> Option<LauncherEntry<'_>> {
        if let Some(answer) = self.calc_result.as_deref() {
            if row == 0 {
                return Some(LauncherEntry::Answer(answer));
            }
            row -= 1;
        }
        self.matches
            .get(row)
            .and_then(|&app| self.apps.get(app))
            .map(LauncherEntry::App)
            .or_else(|| self.search_provider().filter(|_| row == self.matches.len()).map(LauncherEntry::Search))
    }

    /// Moves the highlight by `delta` rows, stopping at either end.
    pub fn move_selection(&mut self, delta: i32) {
        self.selected = self.selected.saturating_add_signed(delta as isize).min(self.entry_count().saturating_sub(1));
    }

    /// Runs row `index`'s action — its alternative one when `alternate` is set — then dismisses.
    pub fn activate(&mut self, index: usize, alternate: bool) {
        match self.entry(index) {
            Some(LauncherEntry::App(app)) => {
                Platform::spawn(app.action.as_ref().filter(|_| alternate).map_or(&app.exec, |(_, exec)| exec));
            }
            Some(LauncherEntry::Answer(answer)) => self.pending_copy = Some(answer.to_owned()),
            Some(LauncherEntry::Search(engine)) => {
                let terms = self.search_query().1;
                let encoded = form_urlencoded::byte_serialize(terms.as_bytes()).collect::<String>();
                Platform::open_url(&engine.config.url.replace("{searchTerms}", &encoded));
            }
            None => return,
        }
        self.open = false;
        self.field.clear();
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

    fn search_provider(&self) -> Option<&SearchEngine> {
        let (provider, query) = self.search_query();
        self.providers.get(provider.unwrap_or_default()).filter(|_| provider.is_some() || !query.is_empty())
    }

    pub(crate) fn bounds(&self, screen_size: Vec2) -> (Vec2, Vec2) {
        let rows = self.entry_count();
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
        let screen = Rect::new(0.0, 0.0, context.frame.screen_size.x, context.frame.screen_size.y);
        let backdrop = context.interaction.interact(screen);
        if backdrop.clicked()
            && !Rect::from_center(origin + size * 0.5, size * 0.5).contains(context.interaction.mouse_pos())
        {
            self.open = false;
            return;
        }

        self.show_search(context, Quad::from_min_max(origin, origin + size));
        self.show_entries(context, origin);
    }

    fn show_search(&mut self, context: &mut UiContext, quad: Quad) {
        let origin = quad.center - quad.size * 0.5;
        let size = quad.size;
        let (left, right) = (PADDING + 34.0, size.x - PADDING);
        if self.field.touched {
            self.field.touched = false;
            self.field.blink_start = context.frame.time;
        }
        let empty = self.field.text.is_empty();
        let query = if empty { "Search anything…" } else { &self.field.text };
        let mut line =
            context.frame.text.line(query, 18.0, 600.0).visible(vec2(left, HEADER_HEIGHT * 0.5), left..right);
        if empty {
            line = line.with_color(MUTED_COLOR);
        }
        let selection_range = self.field.selection();
        let blink = ((context.frame.time - self.field.blink_start) * 1.4).fract();
        let (caret, selection) = {
            let text = &context.frame.text;
            // Long queries are clipped rather than scrolled, so every offset maps straight to an x.
            let at = |offset: usize| (left + text.width(&self.field.text[..offset], 18.0, 600.0)).min(right);
            let caret = vec2(at(self.field.cursor), blink.smoothstep(0.62, 0.5));
            let selection = if selection_range.is_empty() {
                Vec2::ZERO
            } else {
                vec2(at(selection_range.start), at(selection_range.end))
            };
            (caret, selection)
        };
        context.frame.paint(
            quad.expanded(PILL_MARGIN),
            shader!(|fragment: Fragment, quad: Quad, size: Vec2, caret: Vec2, selection: Vec2| {
                let surface = sample_launcher(quad, fragment.pixel, fragment.globals, fragment.time);
                if surface.mask <= 0.0 {
                    kill();
                }
                let point = quad.local(surface.content_point(fragment.pixel)) + quad.size * 0.5;
                let mut color = Vec3::splat(0.09).lerp(
                    Vec3::splat(0.17),
                    Sdf::rounded_box(
                        surface.refracted - vec2(size.x * 0.5, HEADER_HEIGHT - 0.5),
                        vec2(size.x * 0.5, 0.5),
                        0.0,
                    )
                    .fill(),
                );
                color = color.lerp(ICON_COLOR, magnifier_icon(point - vec2(PADDING + 11.0, HEADER_HEIGHT * 0.5)));

                let selection_width = selection.y - selection.x;
                let highlight = Sdf::rounded_box(
                    point - vec2(f32::midpoint(selection.x, selection.y), HEADER_HEIGHT * 0.5),
                    vec2(selection_width * 0.5, 13.0),
                    3.0,
                )
                .fill();
                color = color.lerp(vec3(0.24, 0.28, 0.52), highlight * presence(selection_width));
                let caret_mask =
                    Sdf::rounded_box(point - vec2(caret.x, HEADER_HEIGHT * 0.5), vec2(0.9, 12.0), 0.9).fill();
                color = color.lerp(TEXT_COLOR, caret_mask * caret.y);

                surface.color(color).opacity(0.82)
            }),
        );
        context.frame.paint(
            line.expanded(line.size * 0.5 + 2.0).translated(origin),
            shader!(|text: TextFragment, quad: Quad| {
                sample_launcher(quad, text.pixel, text.globals, text.time).text(&text)
            }),
        );
    }

    /// Draws and interacts with calculator, application and search rows in visual order.
    fn show_entries(&mut self, context: &mut UiContext, origin: Vec2) {
        let (x, width) = (origin.x + PADDING, PANEL_WIDTH - PADDING * 2.0);
        let text_left = ROW_HEIGHT * 0.5 + ICON_SIZE * 0.5 + GAP * 2.0;

        let mut activated = None;
        for index in 0..self.entry_count() {
            let y = origin.y + HEADER_HEIGHT + PADDING + index as f32 * (ROW_HEIGHT + GAP);
            let pill = Rect::new(x, y, x + width, y + ROW_HEIGHT);
            let response = context.interaction.interact(pill);
            if response.hovered {
                self.selected = index;
            }
            if response.clicked() {
                activated = Some(index);
            }
            let pill: Quad = pill.into();
            let entry = self.entry(index).unwrap().view(self.search_query().1);

            let mut edge = width - ROW_HEIGHT * 0.5;
            let mut badge = |label: Option<&str>, width: f32| {
                let Some(label) = label.filter(|_| self.selected == index) else {
                    return (Vec2::ZERO, text::Line::default());
                };
                let badge = vec2(edge - width * 0.5, width * 0.5);
                edge -= width + GAP;
                let line = context
                    .frame
                    .text
                    .line(label, 13.0, 600.0)
                    .right(vec2(edge, ROW_HEIGHT * 0.5))
                    .with_color(MUTED_COLOR);
                edge -= context.frame.text.width(label, 13.0, 600.0) + GAP * 2.0;
                (badge, line)
            };
            let (enter_badge, action_line) = badge(Some(entry.action), ENTER_BADGE_WIDTH);
            let (alternate_badge, alternate_line) = badge(entry.alternate, ALTERNATE_BADGE_WIDTH);

            let clip = text_left..edge.max(text_left);
            let (name_y, detail_y) =
                if entry.detail.is_empty() { (ROW_HEIGHT * 0.5, 0.0) } else { (ROW_HEIGHT * 0.34, ROW_HEIGHT * 0.68) };
            let name_line =
                context.frame.text.line(entry.name, 16.0, 700.0).visible(vec2(text_left, name_y), clip.clone());
            let detail_line = if entry.detail.is_empty() {
                text::Line::default()
            } else {
                context
                    .frame
                    .text
                    .line(entry.detail, 13.0, 600.0)
                    .visible(vec2(text_left, detail_y), clip)
                    .with_color(DETAIL_COLOR)
            };

            let (image, icon_kind) = match entry.icon {
                EntryIcon::Image(image) => (Some(image), 0),
                EntryIcon::Calculator => (None, CALCULATOR_ICON),
                EntryIcon::Search => (None, SEARCH_ICON),
            };
            context.frame.paint(
                pill.expanded(PILL_MARGIN),
                shader!(|fragment: Fragment, pill: Quad, icon_kind: u32, enter_badge: Vec2, alternate_badge: Vec2| {
                    let surface = sample_pill(pill, fragment.pixel, fragment.globals, fragment.time);
                    if surface.alpha <= VISIBLE_ALPHA {
                        kill();
                    }
                    let mut color = Vec3::splat(0.15)
                        .lerp(Vec3::splat(0.235), presence(enter_badge.y))
                        .lerp(Vec3::splat(0.3), (surface.bulge() / 8.0).min(1.0));

                    let icon_point = surface.local - Vec2::splat(surface.size.y * 0.5);
                    if icon_kind == CALCULATOR_ICON {
                        let calculator = calculator_icon(icon_point);
                        color = color.lerp(calculator.truncate(), calculator.w);
                    } else if icon_kind == SEARCH_ICON {
                        color = color.lerp(ICON_COLOR, magnifier_icon(icon_point));
                    }

                    let point = pill.local(surface.content_point(fragment.pixel)) + pill.size * 0.5;
                    let paint_badge = |color: Vec3, badge: Vec2, shift: bool| {
                        let ink = action_badge(point - vec2(badge.x, ROW_HEIGHT * 0.5), badge.y, shift);
                        color.lerp(ink.truncate(), ink.w)
                    };
                    color = paint_badge(color, enter_badge, false);
                    color = paint_badge(color, alternate_badge, true);

                    surface.color(color)
                }),
            );
            if let Some(image) = image {
                let icon = Quad::new(
                    pill.center - vec2((pill.size.x - pill.size.y) * 0.5, 0.0),
                    Vec2::splat(ICON_SIZE),
                    Vec2::X,
                );
                context.frame.paint(icon, shader!(|fragment: Fragment, image: Image| image.sample(fragment.uv)));
            }

            let origin = vec2(x, y);
            for line in [name_line, detail_line, action_line, alternate_line] {
                context.frame.paint(
                    line.expanded(20.0).translated(origin),
                    shader!(|text: TextFragment, pill: Quad| {
                        sample_pill(pill, text.pixel, text.globals, text.time).text(&text)
                    }),
                );
            }
        }
        if let Some(index) = activated {
            self.activate(index, false);
        }
    }
}
