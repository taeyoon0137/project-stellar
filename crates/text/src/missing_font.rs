//! missing font 진단 (X.8.4).
//!
//! [`collect_missing_fonts`]는 Text node별 fallback chain([`TextFontUsage`])을 registry에 대해
//! 해석하고, 등록되지 않은 face마다 [`MissingFontDiagnostic`] 하나를 만듭니다. 진단은 요청 face
//! (file hash, face index, variation axes), 출처, 영향받는 Text node `StableId`만 담습니다.
//!
//! **텍스트 내용 미포함**: 입력과 진단 어디에도 Text 내용 필드가 없습니다. 진단 문자열은 hash,
//! index, axis tag·값, 출처 이름, StableId로만 구성되므로 비민감 진단(II.3.2)으로 다룰 수 있습니다.
//! 사용자 텍스트·경로가 필요한 detail은 상위 계층이 민감 타입으로 따로 붙입니다.
//!
//! 결과는 결정론적입니다: 진단은 (face key, 출처, axes) 오름차순, 영향 Text ID는 오름차순·중복
//! 없음입니다. 입력 순서와 무관하게 같은 결과를 냅니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::fallback::FontFallbackChain;
use crate::font::{FontFaceRef, FontSource};
use crate::registry::{FontFaceKey, FontRegistry};

/// Text node 하나가 사용하는 fallback chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFontUsage<'a> {
    /// Text node 영속 ID.
    pub text_id: StableId,
    /// 그 Text가 쓰는 fallback chain.
    pub chain: &'a FontFallbackChain,
}

/// 등록되지 않은 face 하나에 대한 진단 (X.8.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFontDiagnostic {
    requested: FontFaceRef,
    affected_text_ids: Vec<StableId>,
}

impl MissingFontDiagnostic {
    /// 안정 진단 코드.
    pub const CODE: &'static str = "missing-font";

    /// 요청된 face(file hash, face index, variation axes, 출처).
    pub fn requested(&self) -> &FontFaceRef {
        &self.requested
    }

    /// 요청 face의 출처.
    pub fn source(&self) -> FontSource {
        self.requested.source()
    }

    /// 영향받는 Text node ID(오름차순, 중복 없음).
    pub fn affected_text_ids(&self) -> &[StableId] {
        &self.affected_text_ids
    }
}

impl fmt::Display for MissingFontDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: face {} (출처 {}",
            MissingFontDiagnostic::CODE,
            FontFaceKey::from(&self.requested),
            self.requested.source()
        )?;
        if !self.requested.variations().is_empty() {
            f.write_str(", axes")?;
            for variation in self.requested.variations() {
                write!(f, " {}={}", variation.tag, variation.value.get())?;
            }
        }
        write!(f, ") 누락, 영향 Text {}개:", self.affected_text_ids.len())?;
        for id in &self.affected_text_ids {
            write!(f, " {id}")?;
        }
        Ok(())
    }
}

/// 각 Text의 fallback chain에서 registry에 없는 face를 찾아 face별 진단으로 모읍니다.
///
/// primary뿐 아니라 fallback 항목의 누락도 보고합니다(누락된 fallback은 줄바꿈·glyph 결과를
/// 바꿀 수 있기 때문입니다). 누락이 없으면 빈 목록입니다.
pub fn collect_missing_fonts<R: FontRegistry + ?Sized>(
    usages: &[TextFontUsage<'_>],
    registry: &R,
) -> Vec<MissingFontDiagnostic> {
    type GroupKey<'a> = (FontFaceKey, FontSource, &'a FontFaceRef);
    let mut groups: BTreeMap<GroupKey<'_>, BTreeSet<StableId>> = BTreeMap::new();
    for usage in usages {
        for (_, face) in usage.chain.resolve(registry).missing() {
            groups
                .entry((FontFaceKey::from(face), face.source(), face))
                .or_default()
                .insert(usage.text_id);
        }
    }
    groups
        .into_iter()
        .map(|((_, _, face), ids)| MissingFontDiagnostic {
            requested: face.clone(),
            affected_text_ids: ids.into_iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::tests::{axis, hash};
    use crate::registry::InMemoryFontRegistry;

    fn id(n: u8) -> StableId {
        let mut bytes = [0u8; 16];
        bytes[15] = n;
        StableId::from_bytes(bytes)
    }

    fn face(source: FontSource, fill: u8) -> FontFaceRef {
        FontFaceRef::new(source, hash(fill), 0, vec![])
    }

    fn chain(faces: Vec<FontFaceRef>) -> FontFallbackChain {
        FontFallbackChain::new(faces).expect("chain")
    }

    /// Text node fixture 레코드(id와 chain). 진단 입력 타입에는 Text 내용 필드가 없으므로 fixture도
    /// 내용을 갖지 않습니다.
    struct TextNodeFixture {
        id: StableId,
        chain: FontFallbackChain,
    }

    fn fixture_nodes() -> Vec<TextNodeFixture> {
        let a_project = face(FontSource::Project, 0x0A);
        let b_system = face(FontSource::System, 0x0B);
        let c_bold = FontFaceRef::new(FontSource::System, hash(0x0C), 0, vec![axis("wght", 700.0)]);
        vec![
            TextNodeFixture {
                id: id(4),
                chain: chain(vec![a_project.clone()]),
            },
            TextNodeFixture {
                id: id(1),
                chain: chain(vec![a_project.clone(), b_system.clone()]),
            },
            TextNodeFixture {
                id: id(2),
                chain: chain(vec![c_bold, a_project]),
            },
            TextNodeFixture {
                id: id(3),
                chain: chain(vec![b_system]),
            },
            TextNodeFixture {
                id: id(5),
                chain: chain(vec![face(FontSource::System, 0x0A)]),
            },
        ]
    }

    fn registry() -> InMemoryFontRegistry {
        let mut registry = InMemoryFontRegistry::new();
        registry
            .register(FontSource::System, FontFaceKey::new(hash(0x0B), 0))
            .expect("b");
        registry
    }

    fn usages(nodes: &[TextNodeFixture]) -> Vec<TextFontUsage<'_>> {
        nodes
            .iter()
            .map(|node| TextFontUsage {
                text_id: node.id,
                chain: &node.chain,
            })
            .collect()
    }

    #[test]
    fn missing_font_fixture_is_exact() {
        let nodes = fixture_nodes();
        let mut input = usages(&nodes);
        // 같은 Text가 두 번 들어와도 ID는 한 번만 기록됩니다.
        input.push(input[1]);
        let diagnostics = collect_missing_fonts(&input, &registry());

        let summary: Vec<(u8, FontSource, usize, Vec<StableId>)> = diagnostics
            .iter()
            .map(|d| {
                (
                    d.requested().file_hash().sha256_digest()[0],
                    d.source(),
                    d.requested().variations().len(),
                    d.affected_text_ids().to_vec(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                // A(Project): 1, 2, 4번 Text(2번은 fallback 항목으로 누락).
                (0x0A, FontSource::Project, 0, vec![id(1), id(2), id(4)]),
                // 같은 파일이라도 System으로 요청한 경우는 별도 진단.
                (0x0A, FontSource::System, 0, vec![id(5)]),
                // C(System, wght 700): 2번 Text.
                (0x0C, FontSource::System, 1, vec![id(2)]),
            ]
        );
        assert!(diagnostics.iter().all(|d| d.requested().face_index() == 0));
        assert_eq!(
            diagnostics[2].requested().variations(),
            &[axis("wght", 700.0)]
        );
    }

    #[test]
    fn result_is_independent_of_input_order() {
        let nodes = fixture_nodes();
        let forward = collect_missing_fonts(&usages(&nodes), &registry());
        let mut reversed = usages(&nodes);
        reversed.reverse();
        assert_eq!(collect_missing_fonts(&reversed, &registry()), forward);
    }

    /// "텍스트 내용 미포함"은 **구조적 보장**입니다. 진단 입력([`TextFontUsage`])과 진단
    /// ([`MissingFontDiagnostic`])에 Text 내용 필드가 없으므로 내용이 진단에 들어올 경로가 없습니다.
    /// 아래 exhaustive 비구조화는 두 타입에 필드가 추가되면 컴파일이 실패하므로, 내용 필드를 추가하는
    /// 변경은 이 테스트를 함께 고쳐야 합니다. Display는 code·face key·출처·axes·StableId만으로
    /// 재구성되는 문자열과 정확히 같아야 합니다(다른 구성 요소 없음).
    #[test]
    fn diagnostics_have_no_text_content_field() {
        let nodes = fixture_nodes();
        let input = usages(&nodes);
        for usage in &input {
            let TextFontUsage {
                text_id: _,
                chain: _,
            } = *usage;
        }
        let diagnostics = collect_missing_fonts(&input, &registry());
        assert!(!diagnostics.is_empty());
        for diagnostic in &diagnostics {
            let MissingFontDiagnostic {
                requested,
                affected_text_ids,
            } = diagnostic;
            let mut expected = format!(
                "missing-font: face {} (출처 {}",
                FontFaceKey::from(requested),
                requested.source()
            );
            if !requested.variations().is_empty() {
                expected.push_str(", axes");
                for variation in requested.variations() {
                    expected.push_str(&format!(" {}={}", variation.tag, variation.value.get()));
                }
            }
            expected.push_str(&format!(") 누락, 영향 Text {}개:", affected_text_ids.len()));
            for id in affected_text_ids {
                expected.push_str(&format!(" {id}"));
            }
            assert_eq!(diagnostic.to_string(), expected);
        }
    }

    #[test]
    fn display_lists_face_source_axes_and_ids() {
        let nodes = fixture_nodes();
        let diagnostics = collect_missing_fonts(&usages(&nodes), &registry());
        assert_eq!(
            diagnostics[2].to_string(),
            format!(
                "missing-font: face sha256:{}#0 (출처 system, axes wght=700) 누락, 영향 Text 1개: {}",
                "0c".repeat(32),
                id(2)
            )
        );
        assert_eq!(
            diagnostics[0].to_string(),
            format!(
                "missing-font: face sha256:{}#0 (출처 project) 누락, 영향 Text 3개: {} {} {}",
                "0a".repeat(32),
                id(1),
                id(2),
                id(4)
            )
        );
    }

    #[test]
    fn no_diagnostics_when_every_face_is_registered() {
        let nodes = fixture_nodes();
        let mut registry = registry();
        for key in [
            FontFaceKey::new(hash(0x0A), 0),
            FontFaceKey::new(hash(0x0C), 0),
        ] {
            registry.register(FontSource::Project, key).expect("등록");
        }
        assert!(collect_missing_fonts(&usages(&nodes), &registry).is_empty());
        assert!(collect_missing_fonts(&[], &registry).is_empty());
    }
}
