//! grapheme 단위 font fallback과 missing glyph cluster 보존 (X.9.5).
//!
//! - [`assign_fonts`]: itemization 결과(X.9.2)의 각 item 안에서 **grapheme cluster마다** style의
//!   fallback chain(X.8.3)을 기록 순서대로 검사해, cluster의 모든 비무시 문자를 cmap에 가진 첫 face를
//!   고릅니다. 같은 face가 이어지는 cluster는 하나의 [`FontRun`]으로 합칩니다. item(=script·방향·언어
//!   run) 경계는 넘지 않으므로 결과 run은 grapheme·script run 단위입니다.
//! - 체인 안 어느 face도 cluster를 덮지 못하면 체인 밖 font로 바꾸지 않고(X.8.3), 체인의 첫 사용 가능
//!   face로 shaping해 `.notdef`로 표시하며 [`MissingGlyphCluster`]로 기록합니다. 사용 가능한 face가
//!   하나도 없으면 run은 face 없이 남습니다.
//! - [`shape_font_runs`]: run마다 shaping하고, shaping 결과 `.notdef` glyph가 나온 cluster도 missing
//!   으로 보존합니다(cmap에는 있지만 glyph가 없는 경우 등).
//! - **feature·variation (X.9.6)**: style의 [`ShapingStyle`] feature를 run shaping에 넘기고, variation은
//!   선택된 chain 항목([`crate::font::FontFaceRef`])의 axes 위에 style axes를 덮어(같은 tag는 style 우선)
//!   face instance를 만듭니다.
//! - Default_Ignorable 문자(ZWJ, variation selector, BiDi 제어 등)는 coverage 판단에서 제외합니다.
//!   무시 문자만으로 된 cluster는 앞 cluster의 face를 따릅니다.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::fallback::{FallbackSelection, ResolvedFallbackChain};
use crate::font::FontVariation;
use crate::itemize::TextItem;
use crate::registry::FontHandle;
use crate::shaping::{
    canonical_features, canonical_variations, shape, FontFace, FontFeature, GlyphId, ShapeError,
    ShapeRequest, ShapedRun,
};

/// style 하나의 OpenType feature·variation 설정 (X.9.6). 문서 style run(VI.7.2)의 feature·variation을
/// 옮긴 값이며 canonical 형식(tag 오름차순, tag당 하나, 마지막 값 우선)입니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ShapingStyle {
    features: Vec<FontFeature>,
    variations: Vec<FontVariation>,
}

impl ShapingStyle {
    /// canonical 형식으로 만듭니다.
    pub fn new(features: Vec<FontFeature>, variations: Vec<FontVariation>) -> ShapingStyle {
        ShapingStyle {
            features: canonical_features(features),
            variations: canonical_variations(variations),
        }
    }

    /// feature(정렬됨).
    pub fn features(&self) -> &[FontFeature] {
        &self.features
    }

    /// variation(정렬됨).
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }
}

/// registry handle에 대응하는 로드된 face를 주는 저장소.
pub trait FaceSource {
    /// handle의 face. 로드되지 않았으면 `None`.
    fn face(&self, handle: FontHandle) -> Option<&FontFace>;
}

/// 결정론적 메모리 face 저장소(handle 오름차순).
#[derive(Debug, Clone, Default)]
pub struct LoadedFaces {
    faces: BTreeMap<FontHandle, FontFace>,
}

impl LoadedFaces {
    /// 빈 저장소.
    pub fn new() -> LoadedFaces {
        LoadedFaces::default()
    }

    /// handle에 face를 연결합니다. 이미 있으면 바꿉니다.
    pub fn insert(&mut self, handle: FontHandle, face: FontFace) {
        self.faces.insert(handle, face);
    }

    /// 연결된 face 수.
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }
}

impl FaceSource for LoadedFaces {
    fn face(&self, handle: FontHandle) -> Option<&FontFace> {
        self.faces.get(&handle)
    }
}

/// font run이 쓰는 face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunFace {
    /// fallback chain `chain_index`번 항목.
    Face {
        /// chain 안 위치(variation axes는 이 항목의 것).
        chain_index: usize,
        /// 등록 handle.
        handle: FontHandle,
    },
    /// 사용 가능한 face가 없습니다(shaping하지 않음, 폭 0).
    None,
}

/// 같은 item 안에서 같은 face를 쓰는 연속 구간.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontRun {
    /// paragraph 안 byte 범위.
    pub range: Range<usize>,
    /// itemization item index.
    pub item: usize,
    /// face.
    pub face: RunFace,
}

/// glyph가 없는 cluster의 원인.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MissingReason {
    /// 체인 안 어느 face도 cluster의 문자를 모두 갖지 않습니다.
    NoCoveringFace,
    /// shaping 결과가 `.notdef` glyph입니다.
    NotdefAfterShaping,
}

/// glyph가 없는 cluster(missing glyph cluster) 하나.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MissingGlyphCluster {
    /// paragraph 안 byte 범위.
    pub range: Range<usize>,
    /// itemization item index.
    pub item: usize,
    /// 원인.
    pub reason: MissingReason,
}

/// font 할당 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontAssignment {
    /// 논리 순서 font run.
    pub runs: Vec<FontRun>,
    /// missing glyph cluster(시작 offset 오름차순).
    pub missing: Vec<MissingGlyphCluster>,
}

/// font 할당·shaping 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontRunError {
    /// item의 style index에 해당하는 fallback chain이 없습니다.
    UnknownStyle {
        /// item index.
        item: usize,
        /// style index.
        style: u32,
    },
    /// item 범위가 텍스트와 맞지 않습니다.
    InvalidItem {
        /// item index.
        item: usize,
    },
    /// shaping 실패.
    Shape(ShapeError),
}

impl fmt::Display for FontRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontRunError::UnknownStyle { item, style } => {
                write!(f, "item {item}의 style {style}에 fallback chain이 없습니다")
            }
            FontRunError::InvalidItem { item } => {
                write!(f, "item {item}의 범위가 텍스트 문자 경계와 맞지 않습니다")
            }
            FontRunError::Shape(err) => write!(f, "font run shaping 실패: {err}"),
        }
    }
}

impl std::error::Error for FontRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FontRunError::Shape(err) => Some(err),
            _ => None,
        }
    }
}

impl From<ShapeError> for FontRunError {
    fn from(err: ShapeError) -> FontRunError {
        FontRunError::Shape(err)
    }
}

/// Default_Ignorable_Code_Point(Unicode DerivedCoreProperties)인지. coverage 판단에서 제외합니다.
pub fn is_default_ignorable(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x034F
            | 0x061C
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF0..=0xFFF8
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

/// face가 cluster의 모든 비무시 문자를 cmap에 갖는지.
fn covers(face: &FontFace, cluster: &str) -> bool {
    cluster
        .chars()
        .filter(|c| !is_default_ignorable(*c))
        .all(|c| face.nominal_glyph(c).is_some())
}

/// item마다 grapheme 단위로 fallback face를 고릅니다 (X.9.5).
///
/// `chains[style]`이 style의 해석된 fallback chain입니다.
pub fn assign_fonts<S: FaceSource + ?Sized>(
    text: &str,
    items: &[TextItem],
    chains: &[ResolvedFallbackChain<'_>],
    faces: &S,
) -> Result<FontAssignment, FontRunError> {
    let mut runs: Vec<FontRun> = Vec::new();
    let mut missing = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        let chain = usize::try_from(item.style)
            .ok()
            .and_then(|s| chains.get(s))
            .ok_or(FontRunError::UnknownStyle {
                item: item_index,
                style: item.style,
            })?;
        let segment = text
            .get(item.range.clone())
            .ok_or(FontRunError::InvalidItem { item: item_index })?;
        // 체인 순서상 첫 로드된 face(아무도 덮지 못할 때 .notdef 표시용).
        let fallback = chain
            .available()
            .find(|(_, h)| faces.face(*h).is_some())
            .map_or(RunFace::None, |(chain_index, handle)| RunFace::Face {
                chain_index,
                handle,
            });
        let mut previous: Option<RunFace> = None;
        for (offset, cluster) in segment.grapheme_indices(true) {
            let range = item.range.start + offset..item.range.start + offset + cluster.len();
            let face = if cluster.chars().all(is_default_ignorable) {
                // 무시 문자만으로 된 cluster는 coverage를 판단할 문자가 없어 앞 cluster face를 따릅니다.
                previous.unwrap_or(fallback)
            } else {
                match chain.select(|h| faces.face(h).is_some_and(|f| covers(f, cluster))) {
                    FallbackSelection::Face {
                        chain_index,
                        handle,
                    } => RunFace::Face {
                        chain_index,
                        handle,
                    },
                    FallbackSelection::NoFace => {
                        missing.push(MissingGlyphCluster {
                            range: range.clone(),
                            item: item_index,
                            reason: MissingReason::NoCoveringFace,
                        });
                        fallback
                    }
                }
            };
            previous = Some(face);
            match runs.last_mut() {
                Some(run)
                    if run.item == item_index
                        && run.face == face
                        && run.range.end == range.start =>
                {
                    run.range.end = range.end;
                }
                _ => runs.push(FontRun {
                    range,
                    item: item_index,
                    face,
                }),
            }
        }
    }
    Ok(FontAssignment { runs, missing })
}

/// shaping된 font run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedFontRun {
    /// font run.
    pub run: FontRun,
    /// shaping 결과. face가 없으면 `None`.
    pub shaped: Option<ShapedRun>,
}

/// font run마다 item의 script·방향·언어와 style feature·variation으로 shaping하고, `.notdef` cluster를
/// missing에 더합니다.
///
/// `chains[style]`·`styles[style]`이 item style의 fallback chain과 shaping 설정입니다. 반환하는 missing
/// 목록은 할당 단계 missing과 shaping 단계 missing을 합쳐 시작 offset 오름차순으로 정렬하고, 같은
/// 범위는 할당 단계 원인 하나만 남깁니다.
pub fn shape_font_runs<S: FaceSource + ?Sized>(
    text: &str,
    items: &[TextItem],
    assignment: &FontAssignment,
    chains: &[ResolvedFallbackChain<'_>],
    styles: &[ShapingStyle],
    faces: &S,
) -> Result<(Vec<ShapedFontRun>, Vec<MissingGlyphCluster>), FontRunError> {
    let mut missing = assignment.missing.clone();
    let mut shaped_runs = Vec::with_capacity(assignment.runs.len());
    // 같은 (face, axes) instance는 한 번만 만듭니다.
    let mut instances: BTreeMap<(FontHandle, Vec<FontVariation>), FontFace> = BTreeMap::new();
    for run in &assignment.runs {
        let item = items
            .get(run.item)
            .ok_or(FontRunError::InvalidItem { item: run.item })?;
        let style_index = usize::try_from(item.style).ok();
        let unknown_style = FontRunError::UnknownStyle {
            item: run.item,
            style: item.style,
        };
        let style = style_index
            .and_then(|s| styles.get(s))
            .ok_or(unknown_style.clone())?;
        let chain = style_index
            .and_then(|s| chains.get(s))
            .ok_or(unknown_style)?;
        let RunFace::Face {
            chain_index,
            handle,
        } = run.face
        else {
            shaped_runs.push(ShapedFontRun {
                run: run.clone(),
                shaped: None,
            });
            continue;
        };
        let Some(base) = faces.face(handle) else {
            shaped_runs.push(ShapedFontRun {
                run: run.clone(),
                shaped: None,
            });
            continue;
        };
        let entry_axes = chain
            .chain()
            .faces()
            .get(chain_index)
            .map(|f| f.variations().to_vec())
            .unwrap_or_default();
        let axes = canonical_variations(
            entry_axes
                .into_iter()
                .chain(style.variations().iter().copied()),
        );
        let face = instances
            .entry((handle, axes))
            .or_insert_with_key(|(_, axes)| {
                if axes.is_empty() {
                    base.clone()
                } else {
                    base.with_variations(axes)
                }
            });
        let shaped = shape(
            face,
            &ShapeRequest {
                text,
                range: run.range.clone(),
                direction: item.direction(),
                script: item.script,
                language: item.language.as_ref(),
                features: style.features(),
            },
        )?;
        for range in notdef_clusters(&shaped) {
            if !missing.iter().any(|m| m.range == range) {
                missing.push(MissingGlyphCluster {
                    range,
                    item: run.item,
                    reason: MissingReason::NotdefAfterShaping,
                });
            }
        }
        shaped_runs.push(ShapedFontRun {
            run: run.clone(),
            shaped: Some(shaped),
        });
    }
    missing.sort_by_key(|m| (m.range.start, m.reason));
    Ok((shaped_runs, missing))
}

/// run 안에서 `.notdef` glyph를 가진 shaping cluster 범위(오름차순).
pub fn notdef_clusters(run: &ShapedRun) -> Vec<Range<usize>> {
    let mut starts: Vec<usize> = run.glyphs.iter().map(|g| g.cluster as usize).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut out: Vec<Range<usize>> = Vec::new();
    for glyph in run.glyphs.iter().filter(|g| g.glyph == GlyphId::NOTDEF) {
        let start = glyph.cluster as usize;
        let next = starts.partition_point(|s| *s <= start);
        let end = starts.get(next).copied().unwrap_or(run.range.end);
        if !out.iter().any(|r| r.start == start) {
            out.push(start..end);
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::bidi::BaseDirection;
    use crate::fallback::FontFallbackChain;
    use crate::font::tests::{axis, tag};
    use crate::font::{FontFaceRef, FontSource};
    use crate::itemize::{itemize, StyleSpan};
    use crate::registry::{FontFaceKey, FontRegistry, InMemoryFontRegistry};
    use crate::test_fonts::{
        cjk_spec, emoji_spec, latin_liga_spec, latin_rtl_spec, latin_spec, variable_latin_spec,
        FontSpec,
    };

    /// 테스트 font 집합: 이름별 (registry handle, face ref).
    pub(crate) struct FontSet {
        pub registry: InMemoryFontRegistry,
        pub faces: LoadedFaces,
        pub refs: Vec<FontFaceRef>,
    }

    impl FontSet {
        /// spec마다 `hash(fill = i + 1)`로 등록하고 로드합니다.
        pub(crate) fn new(specs: &[FontSpec]) -> FontSet {
            let mut registry = InMemoryFontRegistry::new();
            let mut faces = LoadedFaces::new();
            let mut refs = Vec::new();
            for (i, spec) in specs.iter().enumerate() {
                let hash = crate::font::tests::hash(u8::try_from(i + 1).expect("작은 수"));
                let handle = registry
                    .register(FontSource::Project, FontFaceKey::new(hash, 0))
                    .expect("등록");
                faces.insert(handle, FontFace::from_bytes(spec.build(), 0).expect("로드"));
                refs.push(FontFaceRef::new(FontSource::Project, hash, 0, vec![]));
            }
            FontSet {
                registry,
                faces,
                refs,
            }
        }

        pub(crate) fn chain(&self, order: &[usize]) -> FontFallbackChain {
            FontFallbackChain::new(order.iter().map(|i| self.refs[*i].clone()).collect())
                .expect("chain")
        }
    }

    fn span(text: &str) -> Vec<StyleSpan> {
        vec![StyleSpan {
            range: 0..text.len(),
            style: 0,
            language: None,
        }]
    }

    /// (텍스트 조각, chain index) 목록.
    fn run_summary(text: &str, assignment: &FontAssignment) -> Vec<(String, Option<usize>)> {
        assignment
            .runs
            .iter()
            .map(|r| {
                (
                    text[r.range.clone()].to_string(),
                    match r.face {
                        RunFace::Face { chain_index, .. } => Some(chain_index),
                        RunFace::None => None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn emoji_fallback_fixture() {
        // chain: Latin(0) → CJK(1) → emoji(2).
        let set = FontSet::new(&[latin_spec(), cjk_spec(), emoji_spec()]);
        let chain = set.chain(&[0, 1, 2]);
        let resolved = [chain.resolve(&set.registry)];
        let text = "Hi 😀 你好 👩\u{200D}💻!";
        let items = itemize(text, &span(text), BaseDirection::LeftToRight).expect("items");
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        assert_eq!(
            run_summary(text, &assignment),
            vec![
                ("Hi ".to_string(), Some(0)),
                ("😀".to_string(), Some(2)),
                (" ".to_string(), Some(0)),
                ("你好".to_string(), Some(1)),
                (" ".to_string(), Some(0)),
                // ZWJ emoji 시퀀스는 grapheme 하나로 한 face(emoji)를 씁니다.
                ("👩\u{200D}💻".to_string(), Some(2)),
                ("!".to_string(), Some(0)),
            ]
        );
        assert!(assignment.missing.is_empty());
        let (shaped, missing) = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &[ShapingStyle::default()],
            &set.faces,
        )
        .expect("shape");
        assert!(missing.is_empty());
        // emoji run의 glyph advance는 emoji 폰트 값(1200)입니다.
        let emoji = shaped[1].shaped.as_ref().expect("shaped");
        assert_eq!(emoji.glyphs.len(), 1);
        assert_eq!(emoji.glyphs[0].x_advance, 1200);
    }

    #[test]
    fn cjk_fallback_fixture() {
        let set = FontSet::new(&[latin_spec(), cjk_spec()]);
        let chain = set.chain(&[0, 1]);
        let resolved = [chain.resolve(&set.registry)];
        let text = "Hello 世界 안녕";
        let items = itemize(text, &span(text), BaseDirection::LeftToRight).expect("items");
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        assert_eq!(
            run_summary(text, &assignment),
            vec![
                ("Hello ".to_string(), Some(0)),
                ("世界".to_string(), Some(1)),
                (" ".to_string(), Some(0)),
                ("안녕".to_string(), Some(1)),
            ]
        );
        // chain 순서를 바꾸면(CJK 먼저) CJK 폰트가 공백까지 덮습니다(기록 순서만 따름). item 경계
        // ("Hello " Latn | "世界 " Hani | "안녕" Hang)는 넘지 않습니다.
        let reversed = set.chain(&[1, 0]);
        let resolved = [reversed.resolve(&set.registry)];
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        assert_eq!(
            run_summary(text, &assignment),
            vec![
                ("Hello".to_string(), Some(1)),
                (" ".to_string(), Some(0)),
                ("世界 ".to_string(), Some(0)),
                ("안녕".to_string(), Some(0)),
            ]
        );
    }

    #[test]
    fn missing_glyph_clusters_are_preserved() {
        // chain에 Latin만: 한자 cluster는 missing이고 Latin face(.notdef)로 shaping합니다.
        let set = FontSet::new(&[latin_spec()]);
        let chain = set.chain(&[0]);
        let resolved = [chain.resolve(&set.registry)];
        let text = "a世b";
        let items = itemize(text, &span(text), BaseDirection::LeftToRight).expect("items");
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        let missing: Vec<(Range<usize>, MissingReason)> = assignment
            .missing
            .iter()
            .map(|m| (m.range.clone(), m.reason))
            .collect();
        assert_eq!(missing, vec![(1..4, MissingReason::NoCoveringFace)]);
        let (shaped, missing) = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &[ShapingStyle::default()],
            &set.faces,
        )
        .expect("shape");
        // shaping 단계의 .notdef는 이미 기록된 cluster라 중복 기록하지 않습니다.
        assert_eq!(missing.len(), 1);
        let glyphs: Vec<u32> = shaped
            .iter()
            .flat_map(|r| {
                r.shaped
                    .as_ref()
                    .map(|s| s.glyphs.clone())
                    .unwrap_or_default()
            })
            .map(|g| g.glyph.raw())
            .collect();
        assert!(glyphs.contains(&GlyphId::NOTDEF.raw()));
    }

    #[test]
    fn unregistered_or_unloaded_faces_are_skipped_without_substitution() {
        let set = FontSet::new(&[latin_spec(), cjk_spec()]);
        // 체인 첫 항목은 registry에 없는 face입니다.
        let absent = FontFaceRef::new(
            FontSource::System,
            crate::font::tests::hash(0xEE),
            0,
            vec![],
        );
        let chain = FontFallbackChain::new(vec![absent, set.refs[1].clone()]).expect("chain");
        let resolved = [chain.resolve(&set.registry)];
        let text = "가a";
        let items = itemize(text, &span(text), BaseDirection::LeftToRight).expect("items");
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        // CJK(체인 1번)만 사용 가능: 'a'는 CJK에 없어 missing이지만 Latin(체인 밖)으로 바꾸지 않습니다.
        // 한글·Latin은 서로 다른 item이라 run도 나뉩니다.
        assert_eq!(
            run_summary(text, &assignment),
            vec![("가".to_string(), Some(1)), ("a".to_string(), Some(1))]
        );
        assert_eq!(assignment.missing.len(), 1);
        assert_eq!(assignment.missing[0].range, 3..4);

        // 체인 전체가 사용 불가하면 face 없는 run과 전 cluster missing.
        let only_absent = FontFallbackChain::new(vec![FontFaceRef::new(
            FontSource::System,
            crate::font::tests::hash(0xEF),
            0,
            vec![],
        )])
        .expect("chain");
        let resolved = [only_absent.resolve(&set.registry)];
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        assert_eq!(
            run_summary(text, &assignment),
            vec![("가".to_string(), None), ("a".to_string(), None)]
        );
        assert_eq!(assignment.missing.len(), 2);
        let (shaped, _) = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &[ShapingStyle::default()],
            &set.faces,
        )
        .expect("shape");
        assert!(shaped[0].shaped.is_none());
    }

    #[test]
    fn rtl_items_and_ignorable_clusters() {
        let set = FontSet::new(&[latin_rtl_spec(), emoji_spec()]);
        let chain = set.chain(&[0, 1]);
        let resolved = [chain.resolve(&set.registry)];
        // 히브리어 + RLM(무시 문자 단독 cluster) + Latin.
        let text = "שלום\u{200F}ok";
        let items = itemize(text, &span(text), BaseDirection::Auto).expect("items");
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        assert!(assignment.missing.is_empty());
        assert!(assignment
            .runs
            .iter()
            .all(|r| matches!(r.face, RunFace::Face { chain_index: 0, .. })));
        assert!(is_default_ignorable('\u{200D}'));
        assert!(is_default_ignorable('\u{FE0F}'));
        assert!(!is_default_ignorable('a'));
    }

    fn advances(shaped: &[ShapedFontRun]) -> Vec<i32> {
        shaped
            .iter()
            .flat_map(|r| {
                r.shaped
                    .as_ref()
                    .map(|s| s.glyphs.clone())
                    .unwrap_or_default()
            })
            .map(|g| g.x_advance)
            .collect()
    }

    #[test]
    fn style_features_and_chain_or_style_variations_reach_shaping() {
        let set = FontSet::new(&[variable_latin_spec(), latin_liga_spec()]);
        let text = "ab";
        let spans = vec![
            StyleSpan {
                range: 0..1,
                style: 0,
                language: None,
            },
            StyleSpan {
                range: 1..2,
                style: 1,
                language: None,
            },
        ];
        let items = itemize(text, &spans, BaseDirection::LeftToRight).expect("items");
        // style 0: chain 항목이 wght 900. style 1: 같은 항목 + style이 wght 650으로 덮어씀.
        let bold_ref = FontFaceRef::new(
            FontSource::Project,
            set.refs[0].file_hash(),
            0,
            vec![axis("wght", 900.0)],
        );
        let chain = FontFallbackChain::new(vec![bold_ref]).expect("chain");
        let resolved = [chain.resolve(&set.registry), chain.resolve(&set.registry)];
        let styles = [
            ShapingStyle::default(),
            ShapingStyle::new(vec![], vec![axis("wght", 650.0)]),
        ];
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        let (shaped, _) = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &styles,
            &set.faces,
        )
        .expect("shape");
        assert_eq!(advances(&shaped), vec![600, 550]);

        // style feature: liga=0이면 "fi" ligature가 풀립니다.
        let text = "fi";
        let items = itemize(
            text,
            &[StyleSpan {
                range: 0..2,
                style: 0,
                language: None,
            }],
            BaseDirection::LeftToRight,
        )
        .expect("items");
        let liga_chain = set.chain(&[1]);
        let resolved = [liga_chain.resolve(&set.registry)];
        let assignment = assign_fonts(text, &items.items, &resolved, &set.faces).expect("assign");
        let on = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &[ShapingStyle::default()],
            &set.faces,
        )
        .expect("on");
        let off_style = ShapingStyle::new(
            vec![FontFeature {
                tag: tag("liga"),
                value: 0,
            }],
            vec![],
        );
        let off = shape_font_runs(
            text,
            &items.items,
            &assignment,
            &resolved,
            &[off_style],
            &set.faces,
        )
        .expect("off");
        assert_eq!(advances(&on.0), vec![550]);
        assert_eq!(advances(&off.0), vec![300, 300]);
        // style 표가 짧으면 typed error.
        assert_eq!(
            shape_font_runs(text, &items.items, &assignment, &resolved, &[], &set.faces)
                .map(|_| ()),
            Err(FontRunError::UnknownStyle { item: 0, style: 0 })
        );
    }

    #[test]
    fn unknown_style_is_an_error() {
        let set = FontSet::new(&[latin_spec()]);
        let text = "ab";
        let spans = vec![StyleSpan {
            range: 0..2,
            style: 4,
            language: None,
        }];
        let items = itemize(text, &spans, BaseDirection::LeftToRight).expect("items");
        let chain = set.chain(&[0]);
        let resolved = [chain.resolve(&set.registry)];
        assert_eq!(
            assign_fonts(text, &items.items, &resolved, &set.faces).map(|_| ()),
            Err(FontRunError::UnknownStyle { item: 0, style: 4 })
        );
        assert!(FontRunError::UnknownStyle { item: 0, style: 4 }
            .to_string()
            .contains('4'));
    }
}
