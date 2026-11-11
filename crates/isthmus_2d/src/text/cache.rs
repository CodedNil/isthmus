//! Shaping text on the host: cached runs, glyph residency, and per-frame placement records.
use super::{
    Text,
    glyph::PlacedGlyph,
    outline::{Outline, WDTH, WGHT},
};
use hashbrown::HashMap;
use isthmus::{
    ResourceData, Resources, ShaderData,
    glam::{Vec2, vec2},
};
use skrifa::{
    FontRef, MetadataProvider, Tag,
    instance::{LocationRef, Size},
};
use std::{
    borrow::Cow,
    mem,
    sync::{Arc, Weak},
};

impl TextCache {
    fn place_glyph(&mut self, shaped: &ShapedGlyph, field: bool) -> u32 {
        let outline = &shaped.outline;
        let key = (Arc::as_ptr(outline) as usize, field);
        self.resident
            .entry(key)
            .or_insert_with(|| {
                let base = self.words.len() as u32;
                let data = if field { outline.field() } else { &outline.data };
                self.words.extend_from_slice(&data.words);
                let mut glyph = data.glyph;
                glyph.start += base;
                glyph.field_start += base;
                // Keep the allocation identity alive until this frame ends.
                (Arc::downgrade(outline), glyph.append(&mut self.words))
            })
            .1
    }
}

struct ShapedGlyph {
    placed: PlacedGlyph,
    outline: Arc<Outline>,
}

fn axis_range(face: &FontRef<'_>, tag: Tag, fallback: f32) -> (f32, f32) {
    face.axes()
        .iter()
        .find(|axis| axis.tag() == tag)
        .map_or((fallback, fallback), |axis| (axis.min_value(), axis.max_value()))
}

/// Reusable glyph placement and metrics before positioning on screen.
#[derive(Clone, Default)]
pub struct ShapedLine {
    glyphs: Arc<[ShapedGlyph]>,
    field: bool,
    /// Unplaced text geometry and metrics.
    pub text: Text,
}

type RunCache = HashMap<String, HashMap<(u32, u32, u32), (ShapedLine, bool)>>;

/// Font outlines, cached text runs, and per-frame glyph placements.
pub struct TextCache {
    fonts: Vec<Cow<'static, [u8]>>,
    weight_range: (f32, f32),
    width_range: (f32, f32),
    glyphs: HashMap<(usize, u32, u32, u32), Weak<Outline>>,
    words: Vec<u32>,
    resident: HashMap<(usize, bool), (Weak<Outline>, u32)>,
    runs: RunCache,
    span: f32,
    baseline: f32,
}

impl Resources for TextCache {
    fn data(&self) -> ResourceData<'_> {
        &self.words
    }

    fn begin_frame(&mut self) {
        self.words.clear();
        self.resident.clear();
        self.runs.retain(|_, sizes| {
            sizes.retain(|_, (_, used)| mem::replace(used, false));
            !sizes.is_empty()
        });
        self.glyphs.retain(|_, glyph| glyph.strong_count() != 0);
    }
}

impl ShapedLine {
    /// Uses cached distance fields for small text, materialized on first placement.
    pub fn cached_distance(mut self) -> Self {
        self.field |= self.text.size <= 32.0;
        self
    }
}

impl TextCache {
    /// Prepares text at an ascent-to-descent pixel size and font-axis weight for placement.
    pub fn line(&mut self, content: &str, size: f32, weight: f32) -> Text {
        let shaped = self.shape(content, size, weight);
        self.place(&shaped, Vec2::ZERO)
    }

    /// Uses the first font's metrics and later fonts for missing characters.
    /// Static font bytes are borrowed; dynamic font buffers can be moved in.
    pub fn new(fonts: impl IntoIterator<Item = impl Into<Cow<'static, [u8]>>>) -> Self {
        let fonts: Vec<_> = fonts.into_iter().map(Into::into).collect();
        let faces: Vec<_> = fonts.iter().map(|font| FontRef::new(font).expect("parse font")).collect();
        let metrics = faces.first().expect("at least one font").metrics(Size::unscaled(), LocationRef::default());
        let span = metrics.ascent - metrics.descent;
        let baseline = f32::midpoint(metrics.ascent, metrics.descent) / span;
        let range = |tag, fallback| {
            faces
                .iter()
                .map(|face| axis_range(face, tag, fallback))
                .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
                .unwrap()
        };
        let weight_range = range(WGHT, 400.0);
        let width_range = range(WDTH, 100.0);
        Self {
            fonts,
            weight_range,
            width_range,
            glyphs: HashMap::new(),
            words: Vec::new(),
            resident: HashMap::new(),
            runs: RunCache::default(),
            span: span / f32::from(metrics.units_per_em),
            baseline,
        }
    }

    /// Places a cached run with its left advance edge and vertical center at the position.
    pub fn place(&mut self, shaped: &ShapedLine, position: Vec2) -> Text {
        // Materialize outlines before the contiguous placement records. Only
        // glyphs visible in this frame occupy the GPU resource arena.
        for glyph in &*shaped.glyphs {
            self.place_glyph(glyph, shaped.field);
        }
        let start = self.words.len().next_multiple_of(PlacedGlyph::WORDS);
        self.words.resize(start, 0);
        let first = (start / PlacedGlyph::WORDS) as u32;
        for glyph in &*shaped.glyphs {
            PlacedGlyph { glyph: self.place_glyph(glyph, shaped.field), ..glyph.placed }.append(&mut self.words);
        }
        Text { first, ..shaped.text }.translated(position)
    }

    /// Caches a run using character-to-glyph mapping and advances, without kerning or complex shaping.
    pub fn shape(&mut self, text: &str, size: f32, weight: f32) -> ShapedLine {
        self.shape_width(text, size, weight, 100.0)
    }

    pub fn shape_width(&mut self, text: &str, size: f32, weight: f32, width: f32) -> ShapedLine {
        let key = (size.to_bits(), weight.to_bits(), width.to_bits());
        if let Some((line, used)) = self.runs.get_mut(text).and_then(|sizes| sizes.get_mut(&key)) {
            *used = true;
            return line.clone();
        }
        let line = self.shape_uncached(text, size, weight, width);
        self.runs.entry_ref(text).or_default().insert(key, (line.clone(), true));
        line
    }

    fn shape_uncached(&mut self, text: &str, size: f32, font_weight: f32, font_width: f32) -> ShapedLine {
        if !size.is_finite() || size <= 0.0 {
            return ShapedLine::default();
        }
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        let weight = font_weight.clamp(self.weight_range.0, self.weight_range.1);
        let width_axis = font_width.clamp(self.width_range.0, self.width_range.1);
        let mut glyphs = Vec::new();
        let faces: Vec<_> = self.fonts.iter().map(|font| FontRef::new(font).expect("parse font")).collect();
        let locations: Vec<_> =
            faces.iter().map(|face| face.axes().location([(WGHT, weight), (WDTH, width_axis)])).collect();
        let fonts: Vec<_> = faces
            .iter()
            .zip(&locations)
            .map(|(face, location)| {
                (
                    face.charmap(),
                    face.glyph_metrics(Size::unscaled(), location),
                    self.span * f32::from(face.metrics(Size::unscaled(), LocationRef::default()).units_per_em),
                )
            })
            .collect();
        let mut x = 0.0;
        for character in text.chars().filter(|character| !matches!(character, '\u{fe0e}' | '\u{fe0f}')) {
            let (font, id) = fonts
                .iter()
                .enumerate()
                .find_map(|(index, (charmap, ..))| charmap.map(character).map(|id| (index, id)))
                .unwrap_or_default();
            let (_, metrics, span) = &fonts[font];
            let outline = Outline::get(&mut self.glyphs, &faces[font], weight, width_axis, *span, font, id.to_u32());
            let data = outline.data.glyph;
            if data.count > 0 {
                min = min.min(vec2(x + data.min.x, -data.max.y));
                max = max.max(vec2(x + data.max.x, -data.min.y));
                glyphs.push(ShapedGlyph {
                    placed: PlacedGlyph { x, y: 0.0, glyph: 0, left: x + data.min.x, right: x + data.max.x },
                    outline,
                });
            }
            x += metrics.advance_width(id).unwrap_or(0.0) / span;
        }
        let width = x.max(0.0) * size;
        glyphs.sort_by(|a, b| a.placed.left.total_cmp(&b.placed.left));
        let mut right = f32::NEG_INFINITY;
        for glyph in &mut glyphs {
            right = right.max(glyph.placed.right);
            glyph.placed.right = right;
        }
        ShapedLine {
            text: Text { min: min * size, max: max * size, size, width, count: glyphs.len() as u32, ..Text::default() }
                .translated(vec2(0.0, self.baseline * size)),
            glyphs: glyphs.into(),
            field: false,
        }
    }
}
