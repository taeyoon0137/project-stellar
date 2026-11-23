//! 저장된 문단/문자의 명시적 스타일로 layout을 만들고 같은 layout에서 picking·caret을 구합니다.
//!
//! 문서가 정하지 않은 값은 CSS initial의 화면 해석을 씁니다(ADR-0067 §5): family 없음은 기본 chain, medium은
//! 16px, 글자색 없음은 검정입니다. 글꼴 파일은 [`super::fonts::FaceLoader`]가 document thread 밖에서 읽습니다.
use super::fonts::FaceLoader;
use super::*;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use stellar_core_types::{
    color::Color,
    font::FontSource,
    ids::StableId,
    typography::{FontSize as CssSize, FontStyle, LetterSpacing, LineHeight as CssHeight},
    value::Transform2D,
};
use stellar_document::{
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
    line_break::{line_break_opportunities, LineBreakOptions},
    line_layout::WrapMode,
    paragraph::TextStyleSpec,
    registry::InMemoryFontRegistry,
    shaping::{FontFeature, LanguageTag},
    text_layout::{ParagraphLayoutStyle, TextAlign, TextBox},
};

// 프로세스 안에서만 쓰는 동일 paint 식별자. wrap되면 새 식별자 발급을 중단합니다.
static NEXT_PAINT: AtomicU64 = AtomicU64::new(1);

/// CSS `font-size: medium`의 화면 해석(1/64 px). design profile이 없을 때의 UA 기본값입니다.
const MEDIUM_SUBPIXELS: u32 = 16 * 64;

/// CSS initial `color`(canvastext)의 밝은 화면 해석.
fn initial_text_color() -> Color {
    Color::new(stellar_core_types::color::ColorSpace::Srgb, 0., 0., 0., 1.).expect("불투명 검정")
}

/// 한 줄의 문자/byte 경계와 문서 scalar offset 시작점.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParagraphOffsets {
    start: u32,
    bytes: Vec<usize>,
}

/// 문서 Text 배치의 줄바꿈 옵션(배치와 min-content가 같이 씀).
fn line_break_options() -> Option<LineBreakOptions> {
    Some(LineBreakOptions::new(LanguageTag::parse("und").ok()?))
}

/// glyph paint와 동일한 layout에서 가져온 caret과 줄 영역.
#[derive(Clone, Debug)]
pub struct DocumentText {
    /// 모양별 단 하나의 지역 좌표 경로.
    pub outlines: Vec<PathGeometry>,
    /// painter 순서의 pen 위치 및 outlines index.
    pub glyphs: Vec<(Vec2, u32)>,
    /// 새로 계산한 투영마다 고유합니다. 동일한 Arc를 재사용할 때만 같은 값입니다.
    pub paint_token: u64,
    /// 실제 줄 단위 picking 영역.
    pub lines: Vec<Rect>,
    /// 명시된 글자색.
    pub fill: Color,
    layout: TextLayout,
    offsets: Vec<ParagraphOffsets>,
}

/// `paint_token`은 셸 cache 식별자일 뿐 내용이 아니므로 비교에서 뺍니다.
impl PartialEq for DocumentText {
    fn eq(&self, other: &Self) -> bool {
        self.outlines == other.outlines
            && self.glyphs == other.glyphs
            && self.lines == other.lines
            && self.fill == other.fill
            && self.layout == other.layout
            && self.offsets == other.offsets
    }
}

impl Eq for DocumentText {}

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

    /// 첫 줄 baseline(text box 위쪽 기준 px). 줄이 없으면 `None`입니다.
    pub(crate) fn first_baseline(&self) -> Option<f32> {
        self.layout.first_baseline().map(px)
    }

    /// CSS min-content 너비(px). 이 배치는 줄 너비 0으로 한 결과여야 합니다(ADR-0134 결정 1).
    ///
    /// text 엔진은 기회 사이 구간이 줄보다 넓으면 비상 줄바꿈(`overflow-wrap: break-word`)을 합니다. CSS에서 이 비상
    /// 줄바꿈은 min-content에 들어가지 않으므로, 줄바꿈 기회(같은 엔진의 [`line_break_opportunities`], 배치와 같은 옵션)에서
    /// 끝나지 않은 줄을 다음 줄과 이어 붙인 단위 중 가장 넓은 것을 돌려줍니다. `content`는 배치한 Text 내용입니다.
    pub(crate) fn min_content_width(&self, content: &str) -> Option<f32> {
        let options = line_break_options()?;
        let paragraphs: Vec<&str> = content.split('\n').collect();
        let mut opportunities: Option<(usize, std::collections::HashSet<usize>)> = None;
        let (mut widest, mut unit) = (0i64, 0i64);
        for line in &self.layout.lines {
            let text = paragraphs.get(line.paragraph)?;
            if opportunities.as_ref().map(|(p, _)| *p) != Some(line.paragraph) {
                widest = widest.max(unit);
                unit = 0;
                let offsets = line_break_opportunities(text, &options)
                    .into_iter()
                    .map(|b| b.offset)
                    .collect();
                opportunities = Some((line.paragraph, offsets));
            }
            unit += i64::from(line.width.raw());
            let ends_unit = line.range.end >= text.len()
                || opportunities
                    .as_ref()
                    .is_some_and(|(_, o)| o.contains(&line.range.end));
            if ends_unit {
                widest = widest.max(unit);
                unit = 0;
            }
        }
        Some(px(TextUnit::from_i64(widest.max(unit)).ok()?))
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
    loader: Arc<FaceLoader>,
    registrations: BTreeMap<FontFileHash, DocumentFontAsset>,
    registry: InMemoryFontRegistry,
    faces: LoadedFaces,
    resolver: GlyphResolver,
    atlas: ShelfGlyphAtlas,
    /// registry에 넣은 기본 chain. loader가 아직 읽는 중이면 `None`입니다.
    defaults: Option<Vec<stellar_core_types::font::FontFaceRef>>,
}

/// 테스트는 호출한 thread에서 바로 읽는 loader를 씁니다.
#[cfg(test)]
impl Default for FontCache {
    fn default() -> Self {
        Self::new(FaceLoader::inline())
    }
}

impl FontCache {
    pub(crate) fn new(loader: Arc<FaceLoader>) -> Self {
        Self {
            loader,
            registrations: BTreeMap::new(),
            registry: InMemoryFontRegistry::new(),
            faces: LoadedFaces::new(),
            resolver: GlyphResolver::new(),
            atlas: ShelfGlyphAtlas::new(AtlasConfig::default()).expect("기본 atlas 설정"),
            defaults: None,
        }
    }

    pub(crate) fn sync_registrations(&mut self, styles: &DocumentTextStyles) {
        if self.registrations != styles.fonts {
            *self = Self {
                registrations: styles.fonts.clone(),
                ..Self::new(Arc::clone(&self.loader))
            };
        }
    }

    fn insert(
        &mut self,
        source: FontSource,
        face: &stellar_core_types::font::FontFaceRef,
        font: shaping::FontFace,
    ) -> Option<()> {
        let handle = self
            .registry
            .register(source, FontFaceKey::from(face))
            .ok()?;
        self.faces.insert(handle, font);
        Some(())
    }

    /// 기본 chain을 registry에 한 번 넣고 그 참조를 돌려줍니다. 아직 읽는 중이거나 하나도 없으면 `None`입니다.
    fn default_chain(&mut self) -> Option<Vec<stellar_core_types::font::FontFaceRef>> {
        if self.defaults.is_none() {
            let faces = self.loader.defaults()?;
            let mut refs = Vec::with_capacity(faces.len());
            for (reference, face) in faces {
                if self.registry.lookup(FontFaceKey::from(reference)).is_none() {
                    self.insert(FontSource::System, reference, face.clone())?;
                }
                refs.push(reference.clone());
            }
            self.defaults = Some(refs);
        }
        self.defaults.clone().filter(|refs| !refs.is_empty())
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
            // 읽는 중이면 이번 layout은 기본 chain으로 대체하고, 다 읽은 뒤 scene을 다시 만듭니다.
            let font = self.loader.project_face(root, styles, face)?;
            self.insert(FontSource::Project, face, font)?;
        }
        Some(())
    }

    #[cfg(test)]
    pub(crate) fn layout(
        &mut self,
        model: &stellar_document::model::DocumentModel,
        root: &Path,
        id: StableId,
        payload: &TextPayload,
    ) -> Option<(DocumentText, Size2, Transform2D)> {
        self.layout_input(
            root,
            id,
            payload,
            &model.definition().text_styles,
            model.modifier_stack(id),
            &mut IncrementalTextLayout::new(),
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
        self.layout_input_width(root, id, payload, table, modifiers, engine, None)
    }

    /// [`Self::layout_input`]에 줄 너비(`max_width`, 없으면 줄바꿈 없음)를 더한 것입니다. layout 흐름의 Text 측정과
    /// 그리기가 이 함수 하나를 씁니다(ADR-0134 결정 1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_input_width(
        &mut self,
        root: &Path,
        id: StableId,
        payload: &TextPayload,
        table: &DocumentTextStyles,
        modifiers: Option<&ModifierStack>,
        engine: &mut IncrementalTextLayout,
        max_width: Option<TextUnit>,
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
            let fill = modifier_fill
                .or(style.fill)
                .unwrap_or_else(initial_text_color);
            if color.replace(fill).is_some_and(|previous| previous != fill) {
                return None;
            }
            let typography = style.typography.record().ok()?;
            let size = match typography.size {
                CssSize::Subpixels(size) => size,
                CssSize::Medium => MEDIUM_SUBPIXELS,
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
                // CSS initial family(빈 목록)이거나 지정한 face를 하나도 쓸 수 없으면 기본 chain으로 그립니다.
                refs = self.default_chain()?;
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
            line_break: &line_break_options()?,
            target_profile: TargetProfileKey(0),
            max_width,
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
        let mut glyphs = Vec::with_capacity(geometry.glyphs.len());
        let mut outlines = Vec::new();
        let mut indices: HashMap<GlyphKey, Option<u32>> = HashMap::new();
        for g in geometry.glyphs {
            if let Some(index) = indices.get(&g.key) {
                if let Some(index) = index {
                    glyphs.push((g.origin, *index));
                }
                continue;
            }
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
            let index = match draw {
                GlyphDraw::Exact(GlyphSource::Outline(path)) => {
                    let index = u32::try_from(outlines.len()).ok()?;
                    outlines.push(path);
                    glyphs.push((g.origin, index));
                    Some(index)
                }
                GlyphDraw::Empty(EmptyReason::NoInk) => None,
                _ => return None,
            };
            indices.insert(g.key, index);
        }
        let bounds = Size2::new(px(layout.width), px(layout.height)).ok()?;
        let paint_token = NEXT_PAINT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()?;
        Some((
            DocumentText {
                paint_token,
                outlines,
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
