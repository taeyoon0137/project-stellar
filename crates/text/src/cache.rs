//! shaping·줄 레이아웃 cache key (X.10.5).
//!
//! - [`ShapingCacheKey`]: paragraph shaping 결과([`ShapedParagraph`](crate::paragraph::ShapedParagraph))의
//!   key. **text hash**(paragraph 텍스트), **style hash**(span 범위·style index·언어·기준 방향과 style별
//!   글꼴 크기·feature·variation), **font face hash**(style별 fallback chain 항목의 file hash·face index·axes·
//!   사용 가능 여부), **locale**, **target profile**을 담습니다.
//! - [`LineLayoutCacheKey`]: 줄 레이아웃 결과(X.10.2 `ParagraphLines`)의 key. shaping key에 **너비 제약**,
//!   paragraph 레이아웃 속성(정렬·줄 높이·wrap), 줄바꿈 옵션(word-break·엄격도)을 더합니다.
//!
//! hash는 [`StableHasher`](FNV-1a 128-bit, 필드 길이 접두)로 계산합니다. 실행·플랫폼과 무관하게 같은
//! 입력은 같은 값입니다. key는 저장하지 않는 런타임 값이며, cache([`LayoutCache`])는 hash가 같아도 원본
//! 입력을 다시 비교해 충돌로 잘못된 결과를 돌려주지 않게 합니다(X.10.6 incremental 엔진).
//!
//! font face hash는 chain 항목이 지금 **사용 가능한지**도 담으므로, 누락 폰트가 설치·등록되면 key가 바뀌어
//! 다시 shaping합니다.

use std::collections::BTreeMap;

use crate::bidi::BaseDirection;
use crate::font::{FontFaceRef, FontSource};
use crate::font_runs::FaceSource;
use crate::itemize::StyleSpan;
use crate::line_break::{LineBreakOptions, LineBreakStrictness, WordBreak};
use crate::line_layout::WrapMode;
use crate::paragraph::{ParagraphInput, TextStyleSpec};
use crate::shaping::LanguageTag;
use crate::text_layout::{LineHeight, ParagraphLayoutStyle, TextAlign};
use crate::units::TextUnit;

/// 결정론적 128-bit FNV-1a hasher. 가변 길이 필드는 길이를 먼저 씁니다.
#[derive(Debug, Clone)]
pub struct StableHasher(u128);

impl Default for StableHasher {
    fn default() -> StableHasher {
        StableHasher::new()
    }
}

impl StableHasher {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    /// 빈 상태.
    pub fn new() -> StableHasher {
        StableHasher(StableHasher::OFFSET)
    }

    /// bytes를 그대로 씁니다.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u128::from(*byte);
            self.0 = self.0.wrapping_mul(StableHasher::PRIME);
        }
    }

    /// 길이 접두 bytes.
    pub fn write_len_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.write_bytes(bytes);
    }

    /// little-endian `u64`.
    pub fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// little-endian `u32`.
    pub fn write_u32(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// little-endian `i32`.
    pub fn write_i32(&mut self, value: i32) {
        self.write_bytes(&value.to_le_bytes());
    }

    /// `u8`.
    pub fn write_u8(&mut self, value: u8) {
        self.write_bytes(&[value]);
    }

    /// 결과.
    pub fn finish(&self) -> u128 {
        self.0
    }
}

/// 128-bit 내용 hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash128(pub u128);

/// 선택된 target profile을 가리키는 호출자 제공 key(profile id와 고정 version의 digest 등, IX.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TargetProfileKey(pub u128);

/// paragraph shaping cache key (X.10.5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShapingCacheKey {
    /// paragraph 텍스트 hash.
    pub text: Hash128,
    /// span·style 설정 hash.
    pub style: Hash128,
    /// font face(chain 항목·사용 가능 여부) hash.
    pub font_faces: Hash128,
    /// locale.
    pub locale: LanguageTag,
    /// target profile.
    pub target_profile: TargetProfileKey,
}

/// 줄 레이아웃 cache key (X.10.5).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineLayoutCacheKey {
    /// shaping key.
    pub shaping: ShapingCacheKey,
    /// 너비 제약(없으면 `None`).
    pub width: Option<TextUnit>,
    /// paragraph 레이아웃 속성·줄바꿈 옵션 hash.
    pub layout: Hash128,
}

/// paragraph 텍스트 hash.
pub fn text_hash(text: &str) -> Hash128 {
    let mut h = StableHasher::new();
    h.write_len_bytes(text.as_bytes());
    Hash128(h.finish())
}

fn write_language(h: &mut StableHasher, language: Option<&LanguageTag>) {
    match language {
        Some(tag) => {
            h.write_u8(1);
            h.write_len_bytes(tag.as_str().as_bytes());
        }
        None => h.write_u8(0),
    }
}

fn write_span(h: &mut StableHasher, span: &StyleSpan) {
    h.write_u64(span.range.start as u64);
    h.write_u64(span.range.end as u64);
    h.write_u32(span.style);
    write_language(h, span.language.as_ref());
}

/// span·기준 방향·style별 글꼴 크기·feature·variation hash.
pub fn style_hash(input: &ParagraphInput<'_>, styles: &[TextStyleSpec<'_>]) -> Hash128 {
    let mut h = StableHasher::new();
    h.write_u8(match input.base_direction {
        BaseDirection::LeftToRight => 0,
        BaseDirection::RightToLeft => 1,
        BaseDirection::Auto => 2,
    });
    h.write_u64(input.spans.len() as u64);
    for span in input.spans {
        write_span(&mut h, span);
    }
    h.write_u64(styles.len() as u64);
    for style in styles {
        h.write_i32(style.font_size.raw());
        h.write_u64(style.shaping.features().len() as u64);
        for feature in style.shaping.features() {
            h.write_bytes(&feature.tag.bytes());
            h.write_u32(feature.value);
        }
        h.write_u64(style.shaping.variations().len() as u64);
        for variation in style.shaping.variations() {
            h.write_bytes(&variation.tag.bytes());
            h.write_bytes(&variation.value.to_le_bytes());
        }
    }
    Hash128(h.finish())
}

fn write_face_ref(h: &mut StableHasher, face: &FontFaceRef) {
    h.write_u8(match face.source() {
        FontSource::Project => 1,
        FontSource::System => 2,
        FontSource::External => 3,
    });
    h.write_bytes(face.file_hash().sha256_digest());
    h.write_u32(face.face_index());
    h.write_u64(face.variations().len() as u64);
    for variation in face.variations() {
        h.write_bytes(&variation.tag.bytes());
        h.write_bytes(&variation.value.to_le_bytes());
    }
}

/// style별 fallback chain 항목(file hash·face index·axes)과 항목별 사용 가능 여부 hash.
pub fn font_face_hash<S: FaceSource + ?Sized>(styles: &[TextStyleSpec<'_>], faces: &S) -> Hash128 {
    let mut h = StableHasher::new();
    h.write_u64(styles.len() as u64);
    for style in styles {
        let entries = style.chain.chain().faces();
        let available: Vec<bool> = {
            let mut flags = vec![false; entries.len()];
            for (index, handle) in style.chain.available() {
                if faces.face(handle).is_some() {
                    flags[index] = true;
                }
            }
            flags
        };
        h.write_u64(entries.len() as u64);
        for (entry, ok) in entries.iter().zip(available) {
            write_face_ref(&mut h, entry);
            h.write_u8(u8::from(ok));
        }
    }
    Hash128(h.finish())
}

impl ShapingCacheKey {
    /// paragraph 입력·style·face 저장소·locale·target profile로 key를 만듭니다.
    pub fn new<S: FaceSource + ?Sized>(
        input: &ParagraphInput<'_>,
        styles: &[TextStyleSpec<'_>],
        faces: &S,
        locale: &LanguageTag,
        target_profile: TargetProfileKey,
    ) -> ShapingCacheKey {
        ShapingCacheKey {
            text: text_hash(input.text),
            style: style_hash(input, styles),
            font_faces: font_face_hash(styles, faces),
            locale: locale.clone(),
            target_profile,
        }
    }
}

impl LineLayoutCacheKey {
    /// shaping key에 너비·paragraph 레이아웃 속성·줄바꿈 옵션을 더합니다. 줄바꿈 옵션의 locale은 shaping
    /// key의 locale과 같아야 하며 hash에도 넣습니다.
    pub fn new(
        shaping: ShapingCacheKey,
        style: &ParagraphLayoutStyle,
        breaks: &LineBreakOptions,
        width: Option<TextUnit>,
    ) -> LineLayoutCacheKey {
        let mut h = StableHasher::new();
        h.write_u8(match style.align {
            TextAlign::Start => 0,
            TextAlign::Center => 1,
            TextAlign::End => 2,
            TextAlign::Justify => 3,
        });
        match style.line_height {
            LineHeight::Auto => h.write_u8(0),
            LineHeight::Multiplier(ratio) => {
                h.write_u8(1);
                h.write_u32(ratio.raw());
            }
            LineHeight::Fixed(fixed) => {
                h.write_u8(2);
                h.write_i32(fixed.raw());
            }
        }
        h.write_u8(match style.wrap {
            WrapMode::Wrap => 0,
            WrapMode::NoWrap => 1,
        });
        h.write_u8(match breaks.word_break {
            WordBreak::Normal => 0,
            WordBreak::KeepAll => 1,
        });
        h.write_u8(match breaks.strictness {
            LineBreakStrictness::Strict => 0,
            LineBreakStrictness::Normal => 1,
        });
        h.write_len_bytes(breaks.locale.as_str().as_bytes());
        LineLayoutCacheKey {
            shaping,
            width,
            layout: Hash128(h.finish()),
        }
    }
}

/// hit·miss를 세는 결정론적 cache. 값과 함께 원본 검증 값(`check`)을 보관해, key가 같아도 검증 값이
/// 다르면 miss로 처리합니다(hash 충돌 방어).
#[derive(Debug, Clone)]
pub struct LayoutCache<K: Ord, C: PartialEq, V> {
    entries: BTreeMap<K, (C, V)>,
    hits: u64,
    misses: u64,
}

impl<K: Ord, C: PartialEq, V> Default for LayoutCache<K, C, V> {
    fn default() -> LayoutCache<K, C, V> {
        LayoutCache {
            entries: BTreeMap::new(),
            hits: 0,
            misses: 0,
        }
    }
}

impl<K: Ord, C: PartialEq, V> LayoutCache<K, C, V> {
    /// 빈 cache.
    pub fn new() -> LayoutCache<K, C, V> {
        LayoutCache::default()
    }

    /// key·검증 값이 모두 같은 항목을 찾습니다. hit·miss를 셉니다.
    pub fn get(&mut self, key: &K, check: &C) -> Option<&V> {
        match self.entries.get(key) {
            Some((stored, value)) if stored == check => {
                self.hits += 1;
                Some(value)
            }
            _ => {
                self.misses += 1;
                None
            }
        }
    }

    /// hit·miss를 세지 않고 key의 값을 봅니다.
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(_, value)| value)
    }

    /// 항목을 넣습니다(같은 key는 바꿈).
    pub fn insert(&mut self, key: K, check: C, value: V) {
        self.entries.insert(key, (check, value));
    }

    /// hit 수.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// miss 수.
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// 항목 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 조건을 만족하지 않는 항목을 지웁니다(메모리 회수용).
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.entries.retain(|k, _| keep(k));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::tests::{axis, hash, tag};
    use crate::font::FontSource;
    use crate::font_runs::ShapingStyle;
    use crate::paragraph::tests::Fixture;
    use crate::shaping::FontFeature;
    use crate::text_layout::LineHeightRatio;
    use crate::units::FontSize;

    struct Inputs {
        text: String,
        spans: Vec<StyleSpan>,
        base: BaseDirection,
        size: i32,
        features: Vec<FontFeature>,
        locale: &'static str,
        target: u128,
        width: Option<i32>,
        align: TextAlign,
        word_break: WordBreak,
        extra_chain_entry: bool,
    }

    impl Inputs {
        fn base() -> Inputs {
            Inputs {
                text: "Hello 世界".to_string(),
                spans: vec![StyleSpan {
                    range: 0..12,
                    style: 0,
                    language: None,
                }],
                base: BaseDirection::LeftToRight,
                size: 16,
                features: vec![],
                locale: "ko",
                target: 7,
                width: Some(100),
                align: TextAlign::Start,
                word_break: WordBreak::Normal,
                extra_chain_entry: false,
            }
        }
    }

    fn keys(fx: &Fixture, i: &Inputs) -> (ShapingCacheKey, LineLayoutCacheKey) {
        let mut entries = fx.set.refs.clone();
        if i.extra_chain_entry {
            entries.push(crate::font::FontFaceRef::new(
                FontSource::System,
                hash(0x77),
                0,
                vec![axis("wght", 700.0)],
            ));
        }
        let chain = crate::fallback::FontFallbackChain::new(entries).expect("chain");
        let styles = vec![TextStyleSpec {
            chain: chain.resolve(&fx.set.registry),
            shaping: ShapingStyle::new(i.features.clone(), vec![]),
            font_size: FontSize::from_px(i.size).expect("크기"),
        }];
        let input = ParagraphInput {
            text: &i.text,
            spans: &i.spans,
            base_direction: i.base,
        };
        let locale = LanguageTag::parse(i.locale).expect("locale");
        let shaping = ShapingCacheKey::new(
            &input,
            &styles,
            &fx.set.faces,
            &locale,
            TargetProfileKey(i.target),
        );
        let breaks = LineBreakOptions {
            word_break: i.word_break,
            ..LineBreakOptions::new(locale)
        };
        let line = LineLayoutCacheKey::new(
            shaping.clone(),
            &ParagraphLayoutStyle {
                align: i.align,
                ..ParagraphLayoutStyle::default()
            },
            &breaks,
            i.width.map(|w| TextUnit::from_px(w).expect("폭")),
        );
        (shaping, line)
    }

    #[test]
    fn each_input_change_causes_expected_cache_miss() {
        let fx = Fixture::new();
        let (base_shaping, base_line) = keys(&fx, &Inputs::base());
        let mut shaping_cache: LayoutCache<ShapingCacheKey, (), ()> = LayoutCache::new();
        let mut line_cache: LayoutCache<LineLayoutCacheKey, (), ()> = LayoutCache::new();
        shaping_cache.insert(base_shaping.clone(), (), ());
        line_cache.insert(base_line.clone(), (), ());

        // 같은 입력: 둘 다 hit.
        let (s, l) = keys(&fx, &Inputs::base());
        assert!(shaping_cache.get(&s, &()).is_some());
        assert!(line_cache.get(&l, &()).is_some());

        // (변경, shaping miss 기대, line miss 기대)
        type Change = fn(&mut Inputs);
        let changes: [(&str, Change, bool, bool); 10] = [
            ("text", |i| i.text = "Hello 世間".to_string(), true, true),
            ("style: 글꼴 크기", |i| i.size = 17, true, true),
            (
                "style: feature",
                |i| {
                    i.features = vec![FontFeature {
                        tag: tag("liga"),
                        value: 0,
                    }]
                },
                true,
                true,
            ),
            (
                "style: span 언어",
                |i| i.spans[0].language = Some(LanguageTag::parse("ja").expect("ja")),
                true,
                true,
            ),
            (
                "style: 기준 방향",
                |i| i.base = BaseDirection::RightToLeft,
                true,
                true,
            ),
            ("font face", |i| i.extra_chain_entry = true, true, true),
            ("locale", |i| i.locale = "ja", true, true),
            ("target profile", |i| i.target = 8, true, true),
            ("width", |i| i.width = Some(101), false, true),
            ("정렬", |i| i.align = TextAlign::Center, false, true),
        ];
        for (name, change, shaping_miss, line_miss) in changes {
            let mut inputs = Inputs::base();
            change(&mut inputs);
            let (s, l) = keys(&fx, &inputs);
            assert_eq!(
                shaping_cache.get(&s, &()).is_none(),
                shaping_miss,
                "{name}: shaping"
            );
            assert_eq!(line_cache.get(&l, &()).is_none(), line_miss, "{name}: line");
        }
        // word-break도 줄 key만 바꿉니다.
        let mut inputs = Inputs::base();
        inputs.word_break = WordBreak::KeepAll;
        let (s, l) = keys(&fx, &inputs);
        assert_eq!(s, base_shaping);
        assert_ne!(l, base_line);
        // 집계: shaping hit = 같은 입력 1 + width·정렬 2, line hit = 같은 입력 1.
        assert_eq!(shaping_cache.hits(), 3);
        assert_eq!(line_cache.hits(), 1);
    }

    #[test]
    fn font_availability_change_changes_font_face_hash() {
        use crate::registry::{FontFaceKey, FontRegistry};
        // chain 첫 항목(0x99)은 처음에 registry에 없습니다.
        let mut fx = Fixture::new();
        let missing = crate::font::FontFaceRef::new(FontSource::System, hash(0x99), 0, vec![]);
        let chain = crate::fallback::FontFallbackChain::new(vec![missing, fx.set.refs[0].clone()])
            .expect("chain");
        let hash_now = |fx: &Fixture| {
            let spec = TextStyleSpec {
                chain: chain.resolve(&fx.set.registry),
                shaping: ShapingStyle::default(),
                font_size: FontSize::from_px(16).expect("크기"),
            };
            font_face_hash(&[spec], &fx.set.faces)
        };
        let before = hash_now(&fx);
        // 등록만 되고 로드되지 않은 face는 사용 불가라 hash가 같습니다.
        let handle = fx
            .set
            .registry
            .register(FontSource::System, FontFaceKey::new(hash(0x99), 0))
            .expect("등록");
        assert_eq!(hash_now(&fx), before);
        // 로드되어 사용 가능해지면 hash가 바뀝니다(다시 shaping).
        fx.set.faces.insert(
            handle,
            crate::shaping::FontFace::from_bytes(crate::test_fonts::latin_spec().build(), 0)
                .expect("로드"),
        );
        assert_ne!(hash_now(&fx), before);
    }

    #[test]
    fn stable_hash_golden_and_collision_guard() {
        // FNV-1a 128 golden: 빈 입력은 offset basis, "a"는 공개 test vector.
        assert_eq!(
            StableHasher::new().finish(),
            0x6c62_272e_07bb_0142_62b8_2175_6295_c58d
        );
        let mut h = StableHasher::new();
        h.write_bytes(b"a");
        assert_eq!(h.finish(), 0xd228_cb69_6f1a_8caf_7891_2b70_4e4a_8964);
        // 길이 접두로 경계가 다른 입력을 구분합니다.
        assert_ne!(text_hash("ab"), text_hash("a"));
        // 같은 key라도 검증 값이 다르면 miss.
        let mut cache: LayoutCache<u8, String, u32> = LayoutCache::new();
        cache.insert(1, "원본".to_string(), 10);
        assert_eq!(cache.get(&1, &"원본".to_string()), Some(&10));
        assert_eq!(cache.get(&1, &"다른 입력".to_string()), None);
        assert_eq!((cache.hits(), cache.misses()), (1, 1));
        assert_eq!(cache.len(), 1);
        cache.retain(|_| false);
        assert!(cache.is_empty());
    }

    #[test]
    fn line_height_mode_is_part_of_line_key() {
        let fx = Fixture::new();
        let (shaping, base) = keys(&fx, &Inputs::base());
        let locale = LanguageTag::parse("ko").expect("ko");
        let with_height = |line_height| {
            LineLayoutCacheKey::new(
                shaping.clone(),
                &ParagraphLayoutStyle {
                    line_height,
                    ..ParagraphLayoutStyle::default()
                },
                &LineBreakOptions::new(locale.clone()),
                Some(TextUnit::from_px(100).expect("폭")),
            )
        };
        assert_eq!(with_height(LineHeight::Auto), base);
        let ratio = LineHeightRatio::from_raw(1536).expect("1.5");
        assert_ne!(with_height(LineHeight::Multiplier(ratio)), base);
        assert_ne!(
            with_height(LineHeight::Fixed(TextUnit::from_px(20).expect("20px"))),
            base
        );
    }
}
