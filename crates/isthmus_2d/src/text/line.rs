//! Shaping text: fonts, cached lines, and sending a line to a draw.
//!
//! Shaping needs no renderer. A [`Line`] is shaped and cached on first use, and [`Line::at`] gives the [`Text`] to
//! draw it with, whose glyphs are written to the frame when it is sent to a shader.
use super::{
    Text, TextRecord,
    glyph::PlacedGlyph,
    outline::{Outline, OutlineKey, WDTH, WGHT, next_id},
};
use harfrust::{Buffer, ShapeOptions, ShaperFont, shape};
use hashbrown::HashMap;
use isthmus::{
    Arena, Record, ShaderData, Upload,
    glam::{Vec2, vec2},
};
use skrifa::{
    Font, ScalerFont, Tag,
    font::{Blob, Source},
};
use std::{
    borrow::Cow,
    mem,
    ops::Range,
    sync::{Arc, Mutex, Weak},
};

/// The fonts and caches every line is shaped with.
struct Shaper {
    fonts: Vec<Font>,
    /// The weight and width ranges the installed fonts cover.
    ranges: [(f32, f32); 2],
    /// Each font at the weights and widths it has been asked for, which shapes and draws at that location.
    instances: HashMap<(usize, [u32; 2]), Instance>,
    glyphs: HashMap<OutlineKey, Weak<Outline>>,
    /// Shaped lines by text, then by size, weight and width, with whether each was used since the last sweep.
    runs: HashMap<String, Vec<([u32; 3], Line, bool)>>,
    /// How many lines are cached, and how many it takes to sweep out the unused ones.
    cached: usize,
    limit: usize,
    buffer: Buffer,
    /// The first font's ascent-to-descent span as a fraction of its em, to which all fonts are scaled.
    span: f32,
    baseline: f32,
}

/// A font at one weight and width.
struct Instance {
    font: Font,
    scaler: ScalerFont,
    /// Font units to the ascent-to-descent span, which is one unit of a line's coordinates.
    span: f32,
}

/// How many lines are cached before the first sweep.
const MIN_SWEEP: usize = 256;

static SHAPER: Mutex<Option<Shaper>> = Mutex::new(None);

/// Installs the fonts that lines are shaped with, replacing any installed before.
///
/// The first font's metrics are used, and later fonts supply characters it lacks. Static font bytes are borrowed;
/// dynamic buffers can be moved in.
pub fn install_fonts(fonts: impl IntoIterator<Item = impl Into<Cow<'static, [u8]>>>) {
    let fonts: Vec<Font> = fonts
        .into_iter()
        .map(|bytes| {
            let source = match bytes.into() {
                Cow::Borrowed(bytes) => Source::from(Blob::from(bytes)),
                Cow::Owned(bytes) => Source::from(Blob::from(bytes)),
            };
            Font::new(source, 0).expect("parse font")
        })
        .collect();
    let first = fonts.first().expect("at least one font");
    let line = first.metrics().h_line().expect("the first font has line metrics");
    let (ascent, descent) = (line.ascender.to_f32(), line.descender.to_f32());
    let range = |tag: Tag, fallback| {
        fonts
            .iter()
            .map(|font| font.axes().get_by_tag(tag).map_or((fallback, fallback), |a| (a.min_value(), a.max_value())))
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
            .unwrap_or_default()
    };
    let shaper = Shaper {
        ranges: [range(WGHT, 400.0), range(WDTH, 100.0)],
        span: (ascent - descent) / f32::from(first.units_per_em()),
        baseline: f32::midpoint(ascent, descent) / (ascent - descent),
        fonts,
        instances: HashMap::new(),
        glyphs: HashMap::new(),
        runs: HashMap::new(),
        cached: 0,
        limit: MIN_SWEEP,
        buffer: Buffer::new(),
    };
    *SHAPER.lock().expect("shaper lock") = Some(shaper);
}

fn with_shaper<T>(use_shaper: impl FnOnce(&mut Shaper) -> T) -> T {
    let mut shaper = SHAPER.lock().expect("shaper lock");
    use_shaper(shaper.as_mut().expect("install fonts with `install_fonts` before shaping text"))
}

struct ShapedGlyph {
    placed: PlacedGlyph,
    outline: Arc<Outline>,
}

/// Shaped text, reusable and independent of any position or renderer. Cheap to clone.
#[derive(Clone, Default)]
pub struct Line {
    /// Distinguishes this shaped line from every other, to share its placed glyphs between draws.
    id: usize,
    glyphs: Arc<[ShapedGlyph]>,
    field: bool,
    /// Unplaced text geometry and metrics.
    pub text: TextRecord,
}

impl Line {
    /// Shapes text at an ascent-to-descent pixel size and font-axis weight, using the installed fonts. Shaped lines
    /// are cached for as long as they keep being used.
    pub fn new(text: &str, size: f32, weight: f32) -> Self {
        Self::fitted(text, size, weight, 100.0)
    }

    /// Like [`new`](Self::new), at a font width axis value. Text is shaped with `HarfRust`, so kerning, ligatures and
    /// complex scripts work; text runs left to right or right to left by its script, without mixed-direction reordering.
    pub fn fitted(text: &str, size: f32, weight: f32, width: f32) -> Self {
        with_shaper(|shaper| shaper.shape(text, size, weight, width))
    }

    /// Uses cached distance fields for small text, materialized on first use.
    pub fn cached_distance(mut self) -> Self {
        self.field |= self.text.size <= 32.0;
        self
    }

    /// The text to draw, with its left advance edge and vertical center at the position. Send it to a shader with
    /// `.with(..)` and it is written to the frame then.
    pub fn at(&self, position: Vec2) -> Text<'_> {
        Text::of(self, self.text).translated(position)
    }

    /// Writes the glyphs to `arena` and returns the placement to read back with [`Text::on`], as the GPU would.
    pub fn placed(&self, arena: &mut Arena, position: Vec2) -> TextRecord {
        let first = self.place(arena);
        Text::on(TextRecord { first, ..self.text }, &[]).translated(position).line
    }

    /// Writes the line's glyphs into the frame's shared words, once however many draws use the line and once for each
    /// outline however many lines share it, and returns the index of the line's first placed glyph.
    pub(super) fn place(&self, arena: &mut Arena) -> u32 {
        arena.resident((self.id, self.field), |arena| {
            let outline = |arena: &mut Arena, outline: &Outline| {
                arena.resident((outline.id, self.field), |arena| {
                    let at = arena.shared.len() as u32;
                    arena.shared.extend_from_slice(outline.words(self.field));
                    at
                })
            };
            // The outlines go in first, so that the placement records that refer to them are contiguous.
            for glyph in &*self.glyphs {
                outline(arena, &glyph.outline);
            }
            let start = arena.shared.len().next_multiple_of(PlacedGlyph::WORDS);
            arena.shared.resize(start, 0);
            for glyph in &*self.glyphs {
                let glyph = PlacedGlyph { glyph: outline(arena, &glyph.outline), ..glyph.placed };
                glyph.append(&mut arena.shared);
            }
            (start / PlacedGlyph::WORDS) as u32
        })
    }
}

/// Text is written to the frame when it is sent to a shader, then sampled there.
impl<'draw> Upload<'draw> for Text<'_> {
    fn upload(self, record: &mut Record<'draw>) {
        let first = self.shaped.map_or(self.line.first, |line| line.place(record.arena()));
        record.push(TextRecord { first, ..self.line });
    }
}

impl Shaper {
    /// Drops the cached lines, outlines and font instances that went unused since the last sweep.
    fn sweep(&mut self) {
        self.runs.retain(|_, sizes| {
            sizes.retain_mut(|(_, _, used)| mem::take(used));
            !sizes.is_empty()
        });
        self.glyphs.retain(|_, glyph| glyph.strong_count() != 0);
        // Instances are only needed to shape a line, and are cheap to make again.
        self.instances.clear();
        self.cached = self.runs.values().map(Vec::len).sum();
        self.limit = (self.cached * 2).max(MIN_SWEEP);
    }

    fn shape(&mut self, text: &str, size: f32, weight: f32, width: f32) -> Line {
        let key = [size, weight, width].map(f32::to_bits);
        if let Some((_, line, used)) =
            self.runs.get_mut(text).and_then(|sizes| sizes.iter_mut().find(|run| run.0 == key))
        {
            *used = true;
            return line.clone();
        }
        // Sweeping first keeps the line about to be cached, and the lines in use, safe.
        if self.cached >= self.limit {
            self.sweep();
        }
        let line = self.shape_uncached(text, size, [weight, width]);
        self.runs.entry_ref(text).or_default().push((key, line.clone(), true));
        self.cached += 1;
        line
    }

    /// Splits text into the runs that each installed font covers, so that every run shapes with one font.
    fn runs(&self, text: &str) -> Vec<(usize, Range<usize>)> {
        let mut runs: Vec<(usize, Range<usize>)> = Vec::new();
        let mut previous = 0;
        for (at, character) in text.char_indices() {
            let covers = |font: &Font| font.charmap().map_unicode(character).is_some();
            // A character no font has stays with the one before it, which keeps its mark or selector together.
            let font = self.fonts.iter().position(covers).unwrap_or(previous);
            let end = at + character.len_utf8();
            match runs.last_mut() {
                Some((last, range)) if *last == font => range.end = end,
                _ => runs.push((font, at..end)),
            }
            previous = font;
        }
        runs
    }

    fn shape_uncached(&mut self, text: &str, size: f32, axes: [f32; 2]) -> Line {
        if !size.is_finite() || size <= 0.0 {
            return Line::default();
        }
        let axes = [0, 1].map(|axis| axes[axis].clamp(self.ranges[axis].0, self.ranges[axis].1));
        let bits = axes.map(f32::to_bits);
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let mut glyphs = Vec::new();
        let mut x = 0.0;
        for (font, range) in self.runs(text) {
            let Self { fonts, instances, glyphs: outlines, buffer, span, .. } = &mut *self;
            let instance = instances.entry((font, bits)).or_insert_with(|| {
                let font = fonts[font].instance_builder().variations([(WGHT, axes[0]), (WDTH, axes[1])]).build();
                Instance {
                    scaler: ScalerFont::unhinted(font.clone(), None),
                    span: *span * f32::from(font.units_per_em()),
                    font,
                }
            });
            buffer.clear();
            let Some(run) = text.get(range) else { continue };
            buffer.push_str(run);
            buffer.guess_segment_properties();
            if shape(&ShaperFont::new(&instance.font), buffer, ShapeOptions::new()).is_err() {
                continue;
            }
            for (info, position) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
                let outline =
                    Outline::get(outlines, (font, info.glyph_id, bits[0], bits[1]), &instance.scaler, instance.span);
                let at =
                    vec2(x + position.x_offset as f32 / instance.span, -(position.y_offset as f32) / instance.span);
                let data = outline.glyph;
                if data.count > 0 {
                    min = min.min(vec2(at.x + data.min.x, at.y - data.max.y));
                    max = max.max(vec2(at.x + data.max.x, at.y - data.min.y));
                    glyphs.push(ShapedGlyph {
                        placed: PlacedGlyph {
                            x: at.x,
                            y: at.y,
                            glyph: 0,
                            left: at.x + data.min.x,
                            right: at.x + data.max.x,
                        },
                        outline,
                    });
                }
                x += position.x_advance as f32 / instance.span;
            }
        }
        glyphs.sort_by(|a, b| a.placed.left.total_cmp(&b.placed.left));
        let mut right = f32::NEG_INFINITY;
        for glyph in &mut glyphs {
            right = right.max(glyph.placed.right);
            glyph.placed.right = right;
        }
        let record = TextRecord {
            min: min * size,
            max: max * size,
            size,
            width: x.max(0.0) * size,
            count: glyphs.len() as u32,
            ..TextRecord::default()
        };
        Line {
            id: next_id(),
            text: Text::on(record, &[]).translated(vec2(0.0, self.baseline * size)).line,
            glyphs: glyphs.into(),
            field: false,
        }
    }
}
