//! 저장된 문단/문자의 명시적 스타일로 layout을 만들고 같은 layout에서 picking·caret을 구합니다.
use super::*;
use std::{collections::BTreeMap, path::Path};
use stellar_core_types::{
    color::Color,
    font::FontSource,
    ids::StableId,
    typography::{FontSize as CssSize, FontStyle, LetterSpacing, LineHeight as CssHeight},
    value::Transform2D,
};
use stellar_document::{
    model::DocumentModel,
    modifier::{ModifierPayload, ModifierStack},
    text::{TextAlignment, TextPayload, WrappingMode},
    text_styles::{DocumentFontAsset, DocumentTextStyles},
};
use stellar_renderer_core::glyph::{
    AtlasConfig, EmptyReason, GlyphDraw, GlyphResolver, GlyphSource, ShelfGlyphAtlas,
};
use stellar_text::{
    bidi::BaseDirection,
    cache::TargetProfileKey,
    caret::{self, Affinity, TextPosition},
    fallback::FontFallbackChain,
    font_runs::{LoadedFaces, ShapingStyle},
    incremental::{IncrementalTextLayout, LayoutRequest, ParagraphSpec},
    itemize::StyleSpan,
    line_break::LineBreakOptions,
    line_layout::WrapMode,
    paragraph::TextStyleSpec,
    registry::InMemoryFontRegistry,
    shaping::{FontFeature, LanguageTag},
    text_layout::{ParagraphLayoutStyle, TextAlign, TextBox},
};

/// 한 줄의 문자/byte 경계와 문서 scalar offset 시작점.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParagraphOffsets {
    start: u32,
    bytes: Vec<usize>,
}

/// glyph paint와 동일한 layout에서 가져온 caret과 줄 영역.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentText {
    /// 같은 glyph의 outline 데이터는 PathGeometry 내부에서 공유합니다.
    pub glyphs: Vec<(Vec2, PathGeometry)>,
    /// 실제 줄 단위 picking 영역.
    pub lines: Vec<Rect>,
    /// 명시된 글자색.
    pub fill: Color,
    layout: TextLayout,
    offsets: Vec<ParagraphOffsets>,
}

impl DocumentText {
    fn position(&self, offset: u32, affinity: Affinity) -> Option<TextPosition> {
        let index = self
            .offsets
            .partition_point(|p| p.start <= offset)
            .checked_sub(1)?;
        let paragraph = &self.offsets[index];
        Some(TextPosition {
            paragraph: index,
            offset: *paragraph.bytes.get((offset - paragraph.start) as usize)?,
            affinity,
        })
    }

    /// local 좌표에서 문서 Unicode scalar offset과 affinity를 구합니다.
    pub fn hit(&self, x: f32, y: f32) -> Option<(u32, bool)> {
        let unit = |v: f32| -> Option<TextUnit> {
            let n = (f64::from(v) * 64.).round();
            (n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX))
                .then(|| TextUnit::from_raw(n as i32))
        };
        let hit = caret::hit_test(&self.layout, unit(x)?, unit(y)?)?.position;
        let p = self.offsets.get(hit.paragraph)?;
        Some((
            p.start
                .checked_add(u32::try_from(p.bytes.binary_search(&hit.offset).ok()?).ok()?)?,
            hit.affinity == Affinity::Upstream,
        ))
    }

    /// text box local caret rect. UTF-8/cluster 경계 내부는 거부합니다.
    pub fn caret_rect(&self, offset: u32, upstream: bool) -> Option<Rect> {
        let position = self.position(
            offset,
            if upstream {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            },
        )?;
        let rect = caret::caret_position(&self.layout, &position).ok()?;
        Some(Rect::new(
            Vec2::new(px(rect.x), px(rect.top)).ok()?,
            Size2::new(0., px(rect.height)).ok()?,
        ))
    }

    /// 문단 경계를 넘는 selection을 visual rect로 반환합니다.
    pub fn selection_rects(&self, start: u32, end: u32) -> Option<Vec<Rect>> {
        if start > end {
            return None;
        }
        let first = self.position(start, Affinity::Downstream)?;
        let last = self.position(end, Affinity::Upstream)?;
        let mut result = Vec::new();
        for i in first.paragraph..=last.paragraph {
            let from = if i == first.paragraph {
                first.offset
            } else {
                0
            };
            let to = if i == last.paragraph {
                last.offset
            } else {
                *self.offsets[i].bytes.last()?
            };
            for r in caret::selection_rects(&self.layout, i, from..to).ok()? {
                result.push(Rect::new(
                    Vec2::new(px(r.x), px(r.top)).ok()?,
                    Size2::new(px(r.width), px(r.height)).ok()?,
                ));
            }
        }
        Some(result)
    }
}

#[derive(Debug)]
pub(crate) struct FontCache {
    registrations: BTreeMap<FontFileHash, DocumentFontAsset>,
    registry: InMemoryFontRegistry,
    faces: LoadedFaces,
    resolver: GlyphResolver,
    atlas: ShelfGlyphAtlas,
}

impl Default for FontCache {
    fn default() -> Self {
        Self {
            registrations: BTreeMap::new(),
            registry: InMemoryFontRegistry::new(),
            faces: LoadedFaces::new(),
            resolver: GlyphResolver::new(),
            atlas: ShelfGlyphAtlas::new(AtlasConfig::default()).expect("기본 atlas 설정"),
        }
    }
}

impl FontCache {
    pub(crate) fn sync_registrations(&mut self, styles: &DocumentTextStyles) {
        if self.registrations != styles.fonts {
            *self = Self {
                registrations: styles.fonts.clone(),
                ..Self::default()
            };
        }
    }

    pub(crate) fn insert(
        &mut self,
        face: &stellar_core_types::font::FontFaceRef,
        font: shaping::FontFace,
    ) -> Option<()> {
        let handle = self
            .registry
            .register(FontSource::Project, FontFaceKey::from(face))
            .ok()?;
        self.faces.insert(handle, font);
        Some(())
    }

    fn register(
        &mut self,
        styles: &DocumentTextStyles,
        root: &Path,
        face: &stellar_core_types::font::FontFaceRef,
    ) -> Option<()> {
        if face.source() != FontSource::Project || !styles.fonts.contains_key(&face.file_hash()) {
            return None;
        }
        let key = FontFaceKey::from(face);
        if self.registry.lookup(key).is_none() {
            let font =
                stellar_project::fonts::document::load_document_face(root, styles, face).ok()?;
            self.insert(face, font)?;
        }
        Some(())
    }

    pub(crate) fn layout(
        &mut self,
        model: &DocumentModel,
        root: &Path,
        id: StableId,
        payload: &TextPayload,
    ) -> Option<(DocumentText, Size2, Transform2D)> {
        self.layout_with(model, root, id, payload, &mut IncrementalTextLayout::new())
    }

    pub(super) fn layout_with(
        &mut self,
        model: &DocumentModel,
        root: &Path,
        id: StableId,
        payload: &TextPayload,
        engine: &mut IncrementalTextLayout,
    ) -> Option<(DocumentText, Size2, Transform2D)> {
        self.layout_input(
            root,
            id,
            payload,
            &model.definition().text_styles,
            model.modifier_stack(id),
            engine,
        )
    }

    pub(crate) fn layout_input(
        &mut self,
        root: &Path,
        id: StableId,
        payload: &TextPayload,
        table: &DocumentTextStyles,
        modifiers: Option<&ModifierStack>,
        engine: &mut IncrementalTextLayout,
    ) -> Option<(DocumentText, Size2, Transform2D)> {
        let mut transform = Transform2D::identity();
        let mut modifier_fill = None;
        if let Some(stack) = modifiers {
            for entry in stack.entries().iter().filter(|e| e.enabled) {
                match entry.payload {
                    ModifierPayload::Transform(next) if modifier_fill.is_none() => {
                        transform = transform.then(&next).ok()?;
                    }
                    ModifierPayload::Fill { color } if modifier_fill.is_none() => {
                        modifier_fill = Some(color);
                    }
                    _ => return None,
                }
            }
        }
        // 저장 이전/legacy run은 겹치거나 순서가 바뀔 수 있습니다. 편집기의 나중 run 우선 규칙을 씁니다.
        let raw_runs = payload.style_runs();
        let normalized;
        let runs = if raw_runs
            .windows(2)
            .all(|p| p[0].range.end <= p[1].range.start)
        {
            raw_runs
        } else {
            normalized = stellar_document::text::normalize_style_runs(raw_runs, &[]);
            &normalized
        };
        let mut paragraphs = Vec::new();
        let mut offsets = Vec::new();
        let mut spans = Vec::new();
        let mut scalar = 0u32;
        let mut style_ids = BTreeMap::new();
        let mut cursor = 0;
        for (p, line) in payload
            .paragraphs()
            .iter()
            .zip(payload.content().split('\n'))
        {
            let paragraph = table.paragraphs.get(&p.style)?;
            if !matches!(
                paragraph.line_height,
                stellar_document::text::LineHeight::Auto
            ) {
                return None;
            }
            let mut bytes: Vec<_> = line.char_indices().map(|(pos, _)| pos).collect();
            bytes.push(line.len());
            let len = u32::try_from(bytes.len() - 1).ok()?;
            let end = scalar.checked_add(len)?;
            let mut line_spans = Vec::new();
            let mut lo = scalar;
            while lo < end || (len == 0 && line_spans.is_empty()) {
                while runs.get(cursor).is_some_and(|r| r.range.end.0 <= lo) {
                    cursor += 1;
                }
                let (run, hi) = match runs.get(cursor) {
                    Some(r) if r.range.start.0 <= lo && lo < r.range.end.0 => {
                        (Some(r.style), end.min(r.range.end.0))
                    }
                    Some(r) => (None, end.min(r.range.start.0)),
                    None => (None, end),
                };
                let next = u32::try_from(style_ids.len()).ok()?;
                let index = *style_ids.entry((p.style, run)).or_insert(next);
                line_spans.push(StyleSpan {
                    range: bytes[(lo - scalar) as usize]..bytes[(hi - scalar) as usize],
                    style: index,
                    language: None,
                });
                if hi == end {
                    break;
                }
                lo = hi;
            }
            offsets.push(ParagraphOffsets {
                start: scalar,
                bytes,
            });
            scalar = end.checked_add(1)?;
            paragraphs.push((
                line,
                ParagraphLayoutStyle {
                    align: match paragraph.alignment {
                        TextAlignment::Start => TextAlign::Start,
                        TextAlignment::End => TextAlign::End,
                        TextAlignment::Center => TextAlign::Center,
                        TextAlignment::Justify => TextAlign::Justify,
                    },
                    wrap: match paragraph.wrapping {
                        WrappingMode::Wrap => WrapMode::Wrap,
                        WrappingMode::NoWrap => WrapMode::NoWrap,
                    },
                    ..Default::default()
                },
            ));
            spans.push(line_spans);
        }
        let mut keys: Vec<_> = style_ids.into_iter().collect();
        keys.sort_by_key(|(_, i)| *i);
        let mut fonts = Vec::new();
        let mut color = None;
        for ((paragraph, run), _) in keys {
            let style = table.resolve(paragraph, run)?;
            let fill = modifier_fill.or(style.fill)?;
            if color.replace(fill).is_some_and(|previous| previous != fill) {
                return None;
            }
            let typography = style.typography.record().ok()?;
            let CssSize::Subpixels(size) = typography.size else {
                return None;
            };
            let spacing = style
                .run
                .and_then(|r| r.letter_spacing)
                .unwrap_or(typography.letter_spacing);
            if !matches!(spacing, LetterSpacing::Normal)
                || typography.line_height != CssHeight::Normal
                || typography.weight.get() != 400.
                || typography.stretch_percent.get() != 100.
                || typography.style != FontStyle::Normal
            {
                return None;
            }
            let mut refs = Vec::new();
            for family in &typography.families {
                if let Some(face) = &family.face {
                    if self.register(table, root, face).is_some() {
                        refs.push(face.clone());
                    }
                }
            }
            if refs.is_empty() {
                return None;
            }
            let mut features = Vec::new();
            let mut axes = Vec::new();
            if let Some(r) = style.run {
                for (t, v) in r.features() {
                    features.push(FontFeature {
                        tag: OpenTypeTag::new(t.bytes())?,
                        value: *v,
                    });
                }
                for (t, v) in r.variations() {
                    axes.push(FontVariation {
                        tag: OpenTypeTag::new(t.bytes())?,
                        value: *v,
                    });
                }
            }
            fonts.push((
                FontFallbackChain::new(refs).ok()?,
                ShapingStyle::new(features, axes),
                FontSize::from_raw(i32::try_from(size).ok()?).ok()?,
            ));
        }
        let styles: Vec<_> = fonts
            .iter()
            .map(|(chain, shaping, size)| TextStyleSpec {
                chain: chain.resolve(&self.registry),
                shaping: shaping.clone(),
                font_size: *size,
            })
            .collect();
        let specs: Vec<_> = paragraphs
            .iter()
            .zip(&spans)
            .map(|((line, style), spans)| ParagraphSpec {
                text: line,
                style: *style,
                spans,
                base_direction: BaseDirection::Auto,
            })
            .collect();
        let request = LayoutRequest {
            paragraphs: &specs,
            styles: &styles,
            line_break: &LineBreakOptions::new(LanguageTag::parse("und").ok()?),
            target_profile: TargetProfileKey(0),
            max_width: None,
            paragraph_spacing: TextUnit::ZERO,
            text_box: TextBox::default(),
        };
        let (layout, _) = engine.layout(&request, &self.faces).ok()?;
        let key = TextLayoutKey::from_bytes(id.to_bytes());
        let geometry = CanvasTextGeometry::from_layout(key, &layout).ok()?;
        let rasterizer = TextOutlineRasterizer {
            registry: &self.registry,
            faces: &self.faces,
        };
        let mut glyphs = Vec::new();
        for g in geometry.glyphs {
            let mut draw = self.resolver.resolve(&mut self.atlas, &g.key);
            if matches!(
                draw,
                GlyphDraw::Empty(EmptyReason::Pending) | GlyphDraw::Substitute { .. }
            ) {
                let image = rasterizer.rasterize(&g.key);
                self.resolver
                    .complete(&mut self.atlas, g.key.clone(), image);
                draw = self.resolver.resolve(&mut self.atlas, &g.key);
            }
            match draw {
                GlyphDraw::Exact(GlyphSource::Outline(path)) => glyphs.push((g.origin, path)),
                GlyphDraw::Empty(EmptyReason::NoInk) => {}
                _ => return None,
            }
        }
        let bounds = Size2::new(px(layout.width), px(layout.height)).ok()?;
        Some((
            DocumentText {
                glyphs,
                lines: geometry.line_rects,
                fill: color?,
                layout,
                offsets,
            },
            bounds,
            transform,
        ))
    }
}
