//! missing font의 명시 fallback 적용과 줄바꿈 변경 진단 (XIII.6.7).
//!
//! Text node마다 기록된 **명시 fallback chain**(X.8.3)만 registry에 대해 해석합니다
//! ([`resolve_text_fonts`]).
//!
//! - chain 첫 항목(primary)이 registry에 있으면 그 face를 씁니다([`TextFaceResolution::Primary`]).
//! - primary가 없으면 chain **안에서** 기록 순서상 다음으로 등록된 항목을 씁니다
//!   ([`TextFaceResolution::Fallback`]). chain 밖의 face는 registry에 있어도(같은 family 이름의 시스템
//!   font 포함) 쓰지 않습니다. face 정체성은 이름이 아니라 file hash + face index(X.8.2)이기 때문입니다.
//! - chain 안에 등록된 항목이 하나도 없으면 [`TextFaceResolution::NoFace`]입니다. 임의 font로 대체하지
//!   않으며 렌더링은 missing glyph 경로(X.9.5)를 탑니다.
//!
//! 누락된 face 자체는 X.8.4 [`MissingFontDiagnostic`](요청 face·출처·영향 Text)으로 함께 보고합니다.
//!
//! # 줄바꿈 변경 진단
//!
//! primary가 아닌 face로 배치하면 advance·metric이 달라 줄바꿈이 바뀔 수 있습니다. 대체가 일어난 Text마다
//! [`LineBreakChangeDiagnostic`]을 만듭니다.
//!
//! - 호출자가 [`LineBreakEvidence`]로 의도 face로 마지막에 계산한 줄바꿈(기준)과 대체 결과로 다시 계산한
//!   줄바꿈을 주면 둘을 비교합니다. 같으면 진단을 만들지 않고, 다르면 [`LineBreakChange::Changed`]입니다.
//! - 어느 한쪽이라도 없으면 대체가 일어났다는 사실만으로 [`LineBreakChange::MayChange`]입니다.
//!
//! 진단에는 Text 내용이 없습니다. Text StableId, face key·출처, chain 위치, 줄 수와 줄 경계(byte offset)의
//! digest만 담습니다([`LineBreakSignature`]). 결과는 Text id 오름차순으로 결정론적입니다.
//!
//! # Text id 중복
//!
//! Text node 하나는 chain 하나를 가지므로 입력의 Text id는 고유해야 합니다. 같은 id가 **같은 chain**으로
//! 여러 번 오면 하나로 합칩니다. **다른 chain**으로 오면 어느 쪽을 쓸지가 입력 순서에 달리므로
//! [`ConflictingTextUsage`]로 거부합니다(해석과 누락 진단이 서로 다른 chain에서 나오지 않음). 따라서 같은
//! 입력 집합은 순서와 무관하게 같은 결과입니다.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use stellar_core_types::ids::StableId;
use stellar_storage::hash::ContentHash;
use stellar_text::fallback::FallbackSelection;
use stellar_text::font::FontFaceRef;
use stellar_text::line_layout::LineBreak;
use stellar_text::missing_font::{collect_missing_fonts, MissingFontDiagnostic, TextFontUsage};
use stellar_text::registry::{FontFaceKey, FontHandle, FontRegistry};

/// Text 하나의 face 해석 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFaceResolution {
    /// chain primary(0번)가 등록되어 있음.
    Primary {
        /// 등록 face handle.
        handle: FontHandle,
    },
    /// primary가 없어 chain 안의 다음 등록 항목을 씀.
    Fallback {
        /// chain 안 위치(1 이상).
        chain_index: usize,
        /// 등록 face handle.
        handle: FontHandle,
    },
    /// chain 안에 등록된 face가 없음(임의 font로 대체하지 않음).
    NoFace,
}

impl TextFaceResolution {
    /// primary가 아닌 결과(대체 또는 face 없음)인지.
    pub fn is_substituted(&self) -> bool {
        !matches!(self, TextFaceResolution::Primary { .. })
    }
}

/// Text 하나의 해석.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFontResolution {
    /// Text node 영속 ID.
    pub text_id: StableId,
    /// 요청한 primary face.
    pub requested: FontFaceRef,
    /// 해석 결과.
    pub resolution: TextFaceResolution,
}

/// Text 하나의 줄 경계 요약(내용 없음). 같은 텍스트·폭에서 줄 경계가 같으면 같은 값입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineBreakSignature {
    line_count: u64,
    digest: ContentHash,
}

impl LineBreakSignature {
    /// 줄 범위(paragraph byte 범위)와 강제 줄바꿈 여부 목록에서 만듭니다.
    pub fn from_ranges(
        lines: impl IntoIterator<Item = (Range<usize>, bool)>,
    ) -> LineBreakSignature {
        let mut bytes = Vec::new();
        let mut line_count: u64 = 0;
        for (range, hard_break) in lines {
            line_count = line_count.saturating_add(1);
            bytes.extend_from_slice(&(range.start as u64).to_le_bytes());
            bytes.extend_from_slice(&(range.end as u64).to_le_bytes());
            bytes.push(u8::from(hard_break));
        }
        LineBreakSignature {
            line_count,
            digest: ContentHash::compute(&bytes),
        }
    }

    /// 텍스트 엔진(X.10.1) 줄바꿈 결과에서 만듭니다. 줄 폭은 넣지 않습니다(face마다 달라도 줄 경계가 같으면
    /// 줄바꿈은 같음).
    pub fn from_line_breaks(lines: &[LineBreak]) -> LineBreakSignature {
        LineBreakSignature::from_ranges(
            lines
                .iter()
                .map(|line| (line.range.clone(), line.hard_break)),
        )
    }

    /// 줄 수.
    pub fn line_count(&self) -> u64 {
        self.line_count
    }
}

/// 줄바꿈 변경 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineBreakChange {
    /// 기준과 대체 결과의 줄 경계가 다름.
    Changed {
        /// 기준(의도 face) 줄 수.
        before_lines: u64,
        /// 대체 결과 줄 수.
        after_lines: u64,
    },
    /// 비교할 기준·대체 결과가 없어 대체 사실만으로 변경 가능성을 알림.
    MayChange,
}

/// 줄바꿈 변경 진단.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineBreakChangeDiagnostic {
    /// 영향받는 Text node ID.
    pub text_id: StableId,
    /// 요청한 primary face.
    pub requested: FontFaceRef,
    /// 실제 해석 결과(대체 또는 face 없음).
    pub resolution: TextFaceResolution,
    /// 판정.
    pub change: LineBreakChange,
}

impl LineBreakChangeDiagnostic {
    /// 안정 진단 코드.
    pub const CODE: &'static str = "font-fallback-line-break";
}

impl fmt::Display for LineBreakChangeDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: Text {} 요청 face {} (출처 {}) ",
            LineBreakChangeDiagnostic::CODE,
            self.text_id,
            FontFaceKey::from(&self.requested),
            self.requested.source()
        )?;
        match self.resolution {
            TextFaceResolution::Fallback { chain_index, .. } => {
                write!(f, "누락, fallback chain {chain_index}번 face로 대체")?;
            }
            TextFaceResolution::NoFace => f.write_str("누락, chain 안에 사용 가능한 face 없음")?,
            TextFaceResolution::Primary { .. } => f.write_str("사용")?,
        }
        match self.change {
            LineBreakChange::Changed {
                before_lines,
                after_lines,
            } => write!(f, ", 줄바꿈 변경({before_lines}줄 → {after_lines}줄)"),
            LineBreakChange::MayChange => f.write_str(", 줄바꿈이 바뀔 수 있음"),
        }
    }
}

/// 줄바꿈 비교 입력. Text 내용은 이 trait 밖(텍스트 엔진)에 있고 결과 요약만 넘깁니다.
pub trait LineBreakEvidence {
    /// `text_id`를 의도 face로 마지막에 계산한 줄바꿈. 없으면 `None`.
    fn baseline(&self, text_id: StableId) -> Option<LineBreakSignature>;

    /// `text_id`를 `resolution`(대체 또는 face 없음)으로 다시 계산한 줄바꿈. 계산할 수 없으면 `None`.
    fn relaid(
        &self,
        text_id: StableId,
        resolution: TextFaceResolution,
    ) -> Option<LineBreakSignature>;
}

/// 비교 입력이 없음(대체가 일어나면 항상 [`LineBreakChange::MayChange`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLineBreakEvidence;

impl LineBreakEvidence for NoLineBreakEvidence {
    fn baseline(&self, _text_id: StableId) -> Option<LineBreakSignature> {
        None
    }

    fn relaid(
        &self,
        _text_id: StableId,
        _resolution: TextFaceResolution,
    ) -> Option<LineBreakSignature> {
        None
    }
}

/// 같은 Text id가 서로 다른 chain으로 두 번 이상 들어옴(모듈 문서 "Text id 중복").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictingTextUsage {
    /// 충돌한 Text id(오름차순, 중복 없음, 1개 이상).
    pub text_ids: Vec<StableId>,
}

impl fmt::Display for ConflictingTextUsage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("같은 Text에 서로 다른 fallback chain이 들어왔습니다:")?;
        for text_id in &self.text_ids {
            write!(f, " {text_id}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConflictingTextUsage {}

/// [`resolve_text_fonts`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFontReport {
    /// Text별 해석(Text id 오름차순, Text마다 하나).
    pub resolutions: Vec<TextFontResolution>,
    /// 누락 face 진단(X.8.4, face key 순).
    pub missing: Vec<MissingFontDiagnostic>,
    /// 줄바꿈 변경 진단(Text id 오름차순).
    pub line_breaks: Vec<LineBreakChangeDiagnostic>,
}

impl MissingFontReport {
    /// `text_id`의 해석.
    pub fn resolution(&self, text_id: StableId) -> Option<&TextFontResolution> {
        self.resolutions
            .binary_search_by_key(&text_id, |r| r.text_id)
            .ok()
            .map(|index| &self.resolutions[index])
    }
}

/// Text별 명시 fallback chain을 registry에 대해 해석하고 누락·줄바꿈 변경 진단을 만듭니다.
///
/// 같은 Text id가 서로 다른 chain으로 오면 [`ConflictingTextUsage`]입니다(같은 chain 중복은 하나로 합침).
pub fn resolve_text_fonts<R: FontRegistry + ?Sized, E: LineBreakEvidence + ?Sized>(
    usages: &[TextFontUsage<'_>],
    registry: &R,
    evidence: &E,
) -> Result<MissingFontReport, ConflictingTextUsage> {
    let usages = unique_usages(usages)?;
    let mut by_text: BTreeMap<StableId, TextFontResolution> = BTreeMap::new();
    for usage in &usages {
        let resolved = usage.chain.resolve(registry);
        let resolution = match resolved.select(|_| true) {
            FallbackSelection::Face {
                chain_index: 0,
                handle,
            } => TextFaceResolution::Primary { handle },
            FallbackSelection::Face {
                chain_index,
                handle,
            } => TextFaceResolution::Fallback {
                chain_index,
                handle,
            },
            FallbackSelection::NoFace => TextFaceResolution::NoFace,
        };
        by_text.insert(
            usage.text_id,
            TextFontResolution {
                text_id: usage.text_id,
                requested: usage.chain.primary().clone(),
                resolution,
            },
        );
    }
    let line_breaks = by_text
        .values()
        .filter(|r| r.resolution.is_substituted())
        .filter_map(|r| {
            let change = match (
                evidence.baseline(r.text_id),
                evidence.relaid(r.text_id, r.resolution),
            ) {
                (Some(before), Some(after)) if before == after => return None,
                (Some(before), Some(after)) => LineBreakChange::Changed {
                    before_lines: before.line_count(),
                    after_lines: after.line_count(),
                },
                _ => LineBreakChange::MayChange,
            };
            Some(LineBreakChangeDiagnostic {
                text_id: r.text_id,
                requested: r.requested.clone(),
                resolution: r.resolution,
                change,
            })
        })
        .collect();
    Ok(MissingFontReport {
        resolutions: by_text.into_values().collect(),
        missing: collect_missing_fonts(&usages, registry),
        line_breaks,
    })
}

/// Text id마다 usage 하나(id 오름차순). 같은 chain 중복은 합치고, 다른 chain이면 충돌 id를 모두 보고합니다.
fn unique_usages<'a>(
    usages: &[TextFontUsage<'a>],
) -> Result<Vec<TextFontUsage<'a>>, ConflictingTextUsage> {
    let mut by_text: BTreeMap<StableId, TextFontUsage<'a>> = BTreeMap::new();
    let mut conflicts: BTreeSet<StableId> = BTreeSet::new();
    for usage in usages {
        match by_text.entry(usage.text_id) {
            Entry::Vacant(entry) => {
                entry.insert(*usage);
            }
            Entry::Occupied(entry) => {
                if entry.get().chain != usage.chain {
                    conflicts.insert(usage.text_id);
                }
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(ConflictingTextUsage {
            text_ids: conflicts.into_iter().collect(),
        });
    }
    Ok(by_text.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::system::{SystemFontDiscovery, SystemFontLimits};
    use crate::fonts::test_fixtures::{family_faces, sfnt_face, TempDir, SIG_TRUETYPE};
    use std::collections::BTreeMap;
    use stellar_text::fallback::FontFallbackChain;
    use stellar_text::font::{FontFileHash, FontSource};
    use stellar_text::registry::InMemoryFontRegistry;

    fn id(n: u8) -> StableId {
        let mut bytes = [0u8; 16];
        bytes[15] = n;
        StableId::from_bytes(bytes)
    }

    fn hash_of(bytes: &[u8]) -> FontFileHash {
        FontFileHash::from_sha256_digest(*ContentHash::compute(bytes).digest())
    }

    fn face(source: FontSource, hash: FontFileHash) -> FontFaceRef {
        FontFaceRef::new(source, hash, 0, vec![])
    }

    fn chain(faces: Vec<FontFaceRef>) -> FontFallbackChain {
        FontFallbackChain::new(faces).unwrap()
    }

    /// Text별 기준·대체 줄바꿈(Text 내용 없음).
    #[derive(Default)]
    struct Evidence {
        baseline: BTreeMap<StableId, LineBreakSignature>,
        relaid: BTreeMap<StableId, LineBreakSignature>,
    }

    impl LineBreakEvidence for Evidence {
        fn baseline(&self, text_id: StableId) -> Option<LineBreakSignature> {
            self.baseline.get(&text_id).copied()
        }

        fn relaid(
            &self,
            text_id: StableId,
            resolution: TextFaceResolution,
        ) -> Option<LineBreakSignature> {
            assert!(resolution.is_substituted(), "대체된 Text만 다시 배치");
            self.relaid.get(&text_id).copied()
        }
    }

    /// missing face fixture.
    ///
    /// - 시스템 discovery(XIII.6.2)가 실제 파일 두 개(B: "Fixture Sans", Z: 체인 밖, 같은 family 이름)를
    ///   registry에 등록합니다.
    /// - Project face A(파일 없음 → 미등록)와 External face D(미등록)는 누락입니다.
    struct Fixture {
        _dir: TempDir,
        registry: InMemoryFontRegistry,
        a: FontFaceRef,
        b: FontFaceRef,
        c: FontFaceRef,
        d: FontFaceRef,
        b_handle: FontHandle,
        c_handle: FontHandle,
        z_handle: FontHandle,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new("missing-font");
        let b_bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[0]);
        let z_bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Sans")[1]);
        let c_bytes = sfnt_face(SIG_TRUETYPE, &family_faces("Fixture Serif")[0]);
        let paths: Vec<_> = [
            ("b.ttf", &b_bytes),
            ("z.ttf", &z_bytes),
            ("c.ttf", &c_bytes),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            let path = dir.path.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        })
        .collect();
        let mut registry = InMemoryFontRegistry::new();
        let catalog = SystemFontDiscovery::new(SystemFontLimits::default())
            .register_files(paths, &mut registry);
        assert!(catalog.skipped().is_empty(), "{:?}", catalog.skipped());
        let handle_of = |hash: FontFileHash| {
            catalog
                .faces()
                .iter()
                .find(|face| face.key().file_hash() == hash)
                .map(|face| face.handle())
                .unwrap()
        };
        let b = face(FontSource::System, hash_of(&b_bytes));
        let c = face(FontSource::System, hash_of(&c_bytes));
        Fixture {
            registry,
            a: face(FontSource::Project, hash_of(b"project font not imported")),
            d: face(FontSource::External, hash_of(b"external package missing")),
            b_handle: handle_of(b.file_hash()),
            c_handle: handle_of(c.file_hash()),
            z_handle: handle_of(hash_of(&z_bytes)),
            b,
            c,
            _dir: dir,
        }
    }

    #[test]
    fn missing_primary_uses_only_the_explicit_chain() {
        let fx = fixture();
        let fallback = chain(vec![fx.a.clone(), fx.b.clone(), fx.c.clone()]);
        let skip_missing = chain(vec![fx.a.clone(), fx.d.clone(), fx.c.clone()]);
        let primary = chain(vec![fx.b.clone(), fx.a.clone()]);
        let nothing = chain(vec![fx.a.clone(), fx.d.clone()]);
        let usages = [
            TextFontUsage {
                text_id: id(4),
                chain: &nothing,
            },
            TextFontUsage {
                text_id: id(1),
                chain: &fallback,
            },
            TextFontUsage {
                text_id: id(3),
                chain: &primary,
            },
            TextFontUsage {
                text_id: id(2),
                chain: &skip_missing,
            },
        ];
        let report = resolve_text_fonts(&usages, &fx.registry, &NoLineBreakEvidence).unwrap();

        // expected fallback: chain 기록 순서상 다음 등록 face. 체인 밖 같은 family(Z)는 쓰지 않음.
        let expected = [
            (
                id(1),
                TextFaceResolution::Fallback {
                    chain_index: 1,
                    handle: fx.b_handle,
                },
            ),
            (
                id(2),
                TextFaceResolution::Fallback {
                    chain_index: 2,
                    handle: fx.c_handle,
                },
            ),
            (
                id(3),
                TextFaceResolution::Primary {
                    handle: fx.b_handle,
                },
            ),
            (id(4), TextFaceResolution::NoFace),
        ];
        let actual: Vec<_> = report
            .resolutions
            .iter()
            .map(|r| (r.text_id, r.resolution))
            .collect();
        assert_eq!(actual, expected);
        for r in &report.resolutions {
            assert!(
                !matches!(
                    r.resolution,
                    TextFaceResolution::Primary { handle } | TextFaceResolution::Fallback { handle, .. }
                        if handle == fx.z_handle
                ),
                "체인 밖 face 사용 금지"
            );
        }
        assert_eq!(report.resolution(id(1)).unwrap().requested, fx.a);

        // 누락 face 진단(X.8.4): A는 Text 1·2·3·4, D는 2·4.
        let missing: Vec<_> = report
            .missing
            .iter()
            .map(|d| (d.requested().clone(), d.affected_text_ids().to_vec()))
            .collect();
        let mut expected_missing = vec![
            (fx.a.clone(), vec![id(1), id(2), id(3), id(4)]),
            (fx.d.clone(), vec![id(2), id(4)]),
        ];
        expected_missing.sort_by_key(|(face, _)| (FontFaceKey::from(face), face.source()));
        assert_eq!(missing, expected_missing);

        // 줄바꿈 변경 진단: 대체가 일어난 Text 1·2·4만, 비교 입력이 없으면 MayChange.
        let line_breaks: Vec<_> = report
            .line_breaks
            .iter()
            .map(|d| (d.text_id, d.change))
            .collect();
        assert_eq!(
            line_breaks,
            vec![
                (id(1), LineBreakChange::MayChange),
                (id(2), LineBreakChange::MayChange),
                (id(4), LineBreakChange::MayChange),
            ]
        );
        let text = report.line_breaks[0].to_string();
        assert!(text.starts_with("font-fallback-line-break: "), "{text}");
        assert!(text.contains("fallback chain 1번"), "{text}");

        // 같은 입력은 같은 결과(입력 순서와 무관).
        let mut reversed = usages;
        reversed.reverse();
        assert_eq!(
            resolve_text_fonts(&reversed, &fx.registry, &NoLineBreakEvidence).unwrap(),
            report
        );
    }

    #[test]
    fn line_break_evidence_confirms_or_clears_the_change() {
        let fx = fixture();
        let fallback = chain(vec![fx.a.clone(), fx.b.clone()]);
        let usages: Vec<_> = (1..=4)
            .map(|n| TextFontUsage {
                text_id: id(n),
                chain: &fallback,
            })
            .collect();
        let two_lines = LineBreakSignature::from_ranges([(0..10, false), (10..18, false)]);
        let three_lines =
            LineBreakSignature::from_ranges([(0..7, false), (7..14, false), (14..18, false)]);
        let mut evidence = Evidence::default();
        // Text 1: 기준 2줄 → 대체 3줄(변경). Text 2: 같은 줄 경계(진단 없음). Text 3: 기준만. Text 4: 없음.
        evidence.baseline.insert(id(1), two_lines);
        evidence.relaid.insert(id(1), three_lines);
        evidence.baseline.insert(id(2), two_lines);
        evidence.relaid.insert(id(2), two_lines);
        evidence.baseline.insert(id(3), two_lines);
        let report = resolve_text_fonts(&usages, &fx.registry, &evidence).unwrap();
        let line_breaks: Vec<_> = report
            .line_breaks
            .iter()
            .map(|d| (d.text_id, d.change))
            .collect();
        assert_eq!(
            line_breaks,
            vec![
                (
                    id(1),
                    LineBreakChange::Changed {
                        before_lines: 2,
                        after_lines: 3
                    }
                ),
                (id(3), LineBreakChange::MayChange),
                (id(4), LineBreakChange::MayChange),
            ]
        );
        assert!(report.line_breaks[0].to_string().contains("2줄 → 3줄"));
    }

    #[test]
    fn duplicated_text_ids_do_not_depend_on_input_order() {
        let fx = fixture();
        let fallback = chain(vec![fx.a.clone(), fx.b.clone()]);
        let primary = chain(vec![fx.c.clone()]);
        let usage = |n: u8, chain| TextFontUsage {
            text_id: id(n),
            chain,
        };
        // 같은 Text id가 서로 다른 chain으로 옴(리뷰 재현): 순서와 무관하게 충돌 id를 모두 보고합니다.
        let usages = [
            usage(2, &primary),
            usage(1, &fallback),
            usage(3, &primary),
            usage(1, &primary),
            usage(2, &fallback),
        ];
        let expected = ConflictingTextUsage {
            text_ids: vec![id(1), id(2)],
        };
        let mut reversed = usages;
        reversed.reverse();
        for input in [&usages[..], &reversed[..]] {
            assert_eq!(
                resolve_text_fonts(input, &fx.registry, &NoLineBreakEvidence),
                Err(expected.clone())
            );
        }
        assert!(expected.to_string().contains(&id(1).to_string()));

        // 같은 chain 중복은 하나로 합쳐 고유 입력과 같은 결과입니다(해석·누락 진단이 같은 chain에서 나옴).
        let single =
            resolve_text_fonts(&[usage(1, &fallback)], &fx.registry, &NoLineBreakEvidence).unwrap();
        let repeated = resolve_text_fonts(
            &[usage(1, &fallback), usage(1, &fallback)],
            &fx.registry,
            &NoLineBreakEvidence,
        )
        .unwrap();
        assert_eq!(repeated, single);
        assert_eq!(repeated.resolutions.len(), 1);
        assert_eq!(repeated.missing[0].affected_text_ids(), &[id(1)]);
    }

    #[test]
    fn fully_available_chains_produce_no_diagnostics() {
        let fx = fixture();
        let full = chain(vec![fx.b.clone(), fx.c.clone()]);
        let usages = [TextFontUsage {
            text_id: id(9),
            chain: &full,
        }];
        let report = resolve_text_fonts(&usages, &fx.registry, &NoLineBreakEvidence).unwrap();
        assert_eq!(
            report.resolutions[0].resolution,
            TextFaceResolution::Primary {
                handle: fx.b_handle
            }
        );
        assert!(report.missing.is_empty());
        assert!(report.line_breaks.is_empty());
    }

    #[test]
    fn signature_depends_on_line_boundaries_only() {
        let a = LineBreakSignature::from_ranges([(0..5, false), (5..9, true)]);
        assert_eq!(a.line_count(), 2);
        assert_eq!(
            a,
            LineBreakSignature::from_ranges([(0..5, false), (5..9, true)])
        );
        assert_ne!(
            a,
            LineBreakSignature::from_ranges([(0..5, false), (5..9, false)])
        );
        assert_ne!(
            a,
            LineBreakSignature::from_ranges([(0..4, false), (4..9, true)])
        );
        assert_eq!(LineBreakSignature::from_ranges([]).line_count(), 0);
    }
}
