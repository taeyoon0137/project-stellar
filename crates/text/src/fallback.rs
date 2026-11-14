//! 명시적 font fallback chain (X.8.3).
//!
//! fallback은 사용자가 기록한 순서만 따릅니다(아키텍처 11.1 "명시적 fallback").
//!
//! - [`FontFallbackChain`]: 비어 있지 않은 [`FontFaceRef`] 목록입니다. 첫 항목이 primary이고 나머지는
//!   기록 순서대로 시도합니다. 같은 face instance(file hash + face index + variation axes가 같음,
//!   출처는 무관)를 두 번 넣을 수 없습니다 — registry가 출처와 무관하게 dedup하므로 두 번째 항목은
//!   항상 첫 항목과 같은 결과가 되어 의미가 없습니다.
//! - [`FontFallbackChain::resolve`]: registry에 등록된 항목만 사용 가능으로 표시합니다. **누락된
//!   face를 체인 밖의 임의 시스템 font로 바꾸지 않습니다.** registry에 다른 face가 있어도 체인에
//!   없으면 결과에 나타나지 않습니다.
//! - [`ResolvedFallbackChain::select`]: 사용 가능한 항목을 체인 순서로 검사해 처음으로 조건(예:
//!   glyph coverage — X.9.5)을 만족하는 face를 고릅니다. 아무것도 없으면
//!   [`FallbackSelection::NoFace`]이며, 호출자는 missing glyph로 처리합니다.
//!
//! canonical binary: `u32 LE 항목 수` + 항목마다 [`FontFaceRef`] canonical bytes(X.8.1).

use std::collections::BTreeMap;
use std::fmt;

use crate::font::{ByteReader, FontFaceRef, FontRecordError, FontVariation};
use crate::registry::{FontFaceKey, FontHandle, FontRegistry};

/// fallback chain 생성·decode 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackChainError {
    /// 항목이 하나도 없습니다(primary가 필요).
    Empty,
    /// 같은 face instance가 두 번 이상 나옵니다.
    DuplicateFace {
        /// 처음 나온 위치.
        first: usize,
        /// 중복된 위치.
        duplicate: usize,
    },
    /// 항목 레코드 decode 실패.
    Record(FontRecordError),
}

impl fmt::Display for FallbackChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FallbackChainError::Empty => f.write_str("font fallback chain이 비어 있습니다"),
            FallbackChainError::DuplicateFace { first, duplicate } => write!(
                f,
                "font fallback chain의 {duplicate}번 항목이 {first}번 항목과 같은 face입니다"
            ),
            FallbackChainError::Record(err) => write!(f, "font fallback chain 항목 오류: {err}"),
        }
    }
}

impl std::error::Error for FallbackChainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FallbackChainError::Record(err) => Some(err),
            _ => None,
        }
    }
}

impl From<FontRecordError> for FallbackChainError {
    fn from(err: FontRecordError) -> FallbackChainError {
        FallbackChainError::Record(err)
    }
}

/// 명시적 font fallback chain (X.8.3). 첫 항목이 primary입니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontFallbackChain {
    faces: Vec<FontFaceRef>,
}

impl FontFallbackChain {
    /// 기록 순서 그대로 chain을 만듭니다. 비어 있거나 같은 face instance가 중복되면 거부합니다.
    pub fn new(faces: Vec<FontFaceRef>) -> Result<FontFallbackChain, FallbackChainError> {
        if faces.is_empty() {
            return Err(FallbackChainError::Empty);
        }
        let mut seen: BTreeMap<(FontFaceKey, &[FontVariation]), usize> = BTreeMap::new();
        for (i, face) in faces.iter().enumerate() {
            let identity = (FontFaceKey::from(face), face.variations());
            if let Some(&first) = seen.get(&identity) {
                return Err(FallbackChainError::DuplicateFace {
                    first,
                    duplicate: i,
                });
            }
            seen.insert(identity, i);
        }
        Ok(FontFallbackChain { faces })
    }

    /// primary face(첫 항목).
    pub fn primary(&self) -> &FontFaceRef {
        // 생성 경로가 빈 chain을 거부하므로 첫 항목이 항상 있습니다.
        &self.faces[0]
    }

    /// 모든 항목(기록 순서).
    pub fn faces(&self) -> &[FontFaceRef] {
        &self.faces
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // 항목은 서로 다른 face instance이며 실제 문서 크기상 u32 범위를 넘지 않습니다.
        buf.extend_from_slice(&(self.faces.len() as u32).to_le_bytes());
        for face in &self.faces {
            face.encode_into(&mut buf);
        }
        buf
    }

    /// canonical bytes에서 복원하고 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<FontFallbackChain, FallbackChainError> {
        let mut reader = ByteReader::new(bytes);
        let count = reader.take_u32()? as usize;
        // 항목 하나는 최소 41 bytes입니다. 악성 개수로 큰 할당을 만들지 않습니다.
        let mut faces = Vec::with_capacity(count.min(reader.remaining() / 41));
        for _ in 0..count {
            faces.push(FontFaceRef::decode_from(&mut reader)?);
        }
        reader.finish()?;
        FontFallbackChain::new(faces)
    }

    /// registry 기준으로 각 항목의 사용 가능 여부를 계산합니다. 체인 밖 face는 고려하지 않습니다.
    pub fn resolve<R: FontRegistry + ?Sized>(&self, registry: &R) -> ResolvedFallbackChain<'_> {
        ResolvedFallbackChain {
            chain: self,
            handles: self.faces.iter().map(|f| registry.resolve(f)).collect(),
        }
    }
}

/// fallback 선택 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackSelection {
    /// 체인 `chain_index`번 항목이 선택되었습니다.
    Face {
        /// 체인 안 위치(variation axes는 이 항목의 것을 씁니다).
        chain_index: usize,
        /// 등록 face handle.
        handle: FontHandle,
    },
    /// 체인 안 어느 사용 가능 face도 조건을 만족하지 않습니다. 임의 시스템 font로 대체하지
    /// 않으며 호출자는 missing glyph로 처리합니다.
    NoFace,
}

/// registry에 대해 해석한 fallback chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFallbackChain<'a> {
    chain: &'a FontFallbackChain,
    handles: Vec<Option<FontHandle>>,
}

impl<'a> ResolvedFallbackChain<'a> {
    /// 원본 chain.
    pub fn chain(&self) -> &'a FontFallbackChain {
        self.chain
    }

    pub(crate) fn face_for_handle(&self, index: usize, handle: FontHandle) -> Option<&FontFaceRef> {
        if self.handles.get(index) != Some(&Some(handle)) {
            return None;
        }
        self.chain.faces().get(index)
    }

    /// 사용 가능한 항목을 체인 순서로 (위치, handle) 반환합니다.
    pub fn available(&self) -> impl Iterator<Item = (usize, FontHandle)> + '_ {
        self.handles
            .iter()
            .enumerate()
            .filter_map(|(i, handle)| handle.map(|h| (i, h)))
    }

    /// registry에 없는 항목을 체인 순서로 (위치, 요청 face) 반환합니다.
    pub fn missing(&self) -> impl Iterator<Item = (usize, &'a FontFaceRef)> + '_ {
        let faces = self.chain.faces();
        self.handles
            .iter()
            .enumerate()
            .filter(|(_, handle)| handle.is_none())
            .map(move |(i, _)| (i, &faces[i]))
    }

    /// 모든 항목이 등록되어 있는지.
    pub fn is_fully_available(&self) -> bool {
        self.handles.iter().all(Option::is_some)
    }

    /// 사용 가능한 항목을 체인 순서로 검사해 `accepts`를 처음 만족하는 face를 고릅니다.
    pub fn select(&self, mut accepts: impl FnMut(FontHandle) -> bool) -> FallbackSelection {
        self.available()
            .find(|(_, handle)| accepts(*handle))
            .map_or(FallbackSelection::NoFace, |(chain_index, handle)| {
                FallbackSelection::Face {
                    chain_index,
                    handle,
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::tests::{axis, hash};
    use crate::font::FontSource;
    use crate::registry::InMemoryFontRegistry;

    fn face(source: FontSource, fill: u8) -> FontFaceRef {
        FontFaceRef::new(source, hash(fill), 0, vec![])
    }

    /// fixture: A(누락) → B → C → D(누락) → E. registry에는 체인 밖 Z도 있습니다.
    struct Fixture {
        registry: InMemoryFontRegistry,
        chain: FontFallbackChain,
        b: FontHandle,
        c: FontHandle,
        e: FontHandle,
        z: FontHandle,
    }

    fn fixture() -> Fixture {
        let mut registry = InMemoryFontRegistry::new();
        let z = registry
            .register(FontSource::System, FontFaceKey::new(hash(0x2A), 0))
            .expect("z");
        let b = registry
            .register(FontSource::System, FontFaceKey::new(hash(0x0B), 0))
            .expect("b");
        let c = registry
            .register(FontSource::Project, FontFaceKey::new(hash(0x0C), 0))
            .expect("c");
        let e = registry
            .register(FontSource::External, FontFaceKey::new(hash(0x0E), 0))
            .expect("e");
        let chain = FontFallbackChain::new(vec![
            face(FontSource::Project, 0x0A),
            face(FontSource::System, 0x0B),
            face(FontSource::Project, 0x0C),
            face(FontSource::External, 0x0D),
            face(FontSource::External, 0x0E),
        ])
        .expect("chain");
        Fixture {
            registry,
            chain,
            b,
            c,
            e,
            z,
        }
    }

    #[test]
    fn fallback_order_fixture() {
        let fx = fixture();
        let resolved = fx.chain.resolve(&fx.registry);
        assert_eq!(
            resolved.available().collect::<Vec<_>>(),
            vec![(1, fx.b), (2, fx.c), (4, fx.e)]
        );
        assert_eq!(
            resolved
                .missing()
                .map(|(i, f)| (i, f.file_hash()))
                .collect::<Vec<_>>(),
            vec![(0, hash(0x0A)), (3, hash(0x0D))]
        );
        assert!(!resolved.is_fully_available());
        // primary가 누락되면 다음 사용 가능 항목(B)이 먼저 선택됩니다.
        assert_eq!(
            resolved.select(|_| true),
            FallbackSelection::Face {
                chain_index: 1,
                handle: fx.b
            }
        );
        // 조건을 만족하는 첫 항목은 체인 순서상 C(2)이며 E(4)·Z보다 앞섭니다.
        let covers = [fx.c, fx.e, fx.z];
        assert_eq!(
            resolved.select(|h| covers.contains(&h)),
            FallbackSelection::Face {
                chain_index: 2,
                handle: fx.c
            }
        );
        assert_eq!(
            resolved.select(|h| h == fx.e),
            FallbackSelection::Face {
                chain_index: 4,
                handle: fx.e
            }
        );
    }

    #[test]
    fn missing_face_is_never_replaced_by_unlisted_system_font() {
        let fx = fixture();
        let resolved = fx.chain.resolve(&fx.registry);
        // Z는 registry에 있고 조건도 만족하지만 체인에 없으므로 선택되지 않습니다.
        assert_eq!(resolved.select(|h| h == fx.z), FallbackSelection::NoFace);
        assert!(resolved.available().all(|(_, h)| h != fx.z));

        // 체인 전부가 누락이면 사용 가능 항목이 없고 어떤 조건에도 NoFace입니다.
        let all_missing = FontFallbackChain::new(vec![
            face(FontSource::Project, 0x51),
            face(FontSource::System, 0x52),
        ])
        .expect("chain");
        let resolved = all_missing.resolve(&fx.registry);
        assert_eq!(resolved.available().count(), 0);
        assert_eq!(resolved.missing().count(), 2);
        assert_eq!(resolved.select(|_| true), FallbackSelection::NoFace);
    }

    #[test]
    fn fully_available_chain_selects_primary() {
        let fx = fixture();
        let chain = FontFallbackChain::new(vec![
            face(FontSource::Project, 0x0C),
            face(FontSource::System, 0x0B),
        ])
        .expect("chain");
        let resolved = chain.resolve(&fx.registry);
        assert!(resolved.is_fully_available());
        assert_eq!(resolved.chain(), &chain);
        assert_eq!(chain.primary().file_hash(), hash(0x0C));
        assert_eq!(
            resolved.select(|_| true),
            FallbackSelection::Face {
                chain_index: 0,
                handle: fx.c
            }
        );
    }

    #[test]
    fn chain_order_round_trips_and_golden_bytes() {
        let fx = fixture();
        let bytes = fx.chain.encode();
        let decoded = FontFallbackChain::decode(&bytes).expect("decode");
        assert_eq!(decoded, fx.chain);
        assert_eq!(decoded.encode(), bytes);
        let order: Vec<u8> = decoded
            .faces()
            .iter()
            .map(|f| f.file_hash().sha256_digest()[0])
            .collect();
        assert_eq!(order, vec![0x0A, 0x0B, 0x0C, 0x0D, 0x0E]);

        let single = FontFallbackChain::new(vec![face(FontSource::System, 0x01)]).expect("chain");
        let mut expected = vec![0x01, 0x00, 0x00, 0x00]; // 항목 1개
        expected.extend_from_slice(&face(FontSource::System, 0x01).encode());
        assert_eq!(single.encode(), expected);
    }

    #[test]
    fn empty_and_duplicate_chains_are_rejected() {
        assert_eq!(
            FontFallbackChain::new(vec![]),
            Err(FallbackChainError::Empty)
        );
        // 같은 hash+index+axes는 출처가 달라도 같은 face instance입니다.
        assert_eq!(
            FontFallbackChain::new(vec![
                face(FontSource::Project, 1),
                face(FontSource::System, 2),
                face(FontSource::System, 1),
            ]),
            Err(FallbackChainError::DuplicateFace {
                first: 0,
                duplicate: 2
            })
        );
        // 같은 face라도 variation axes가 다르면 다른 instance로 허용합니다.
        let regular = FontFaceRef::new(FontSource::Project, hash(1), 0, vec![axis("wght", 400.0)]);
        let bold = FontFaceRef::new(FontSource::Project, hash(1), 0, vec![axis("wght", 700.0)]);
        let chain = FontFallbackChain::new(vec![bold, regular]).expect("chain");
        // 두 항목은 같은 handle로 해석되고, 선택은 체인 순서상 첫 항목(bold)입니다.
        let mut registry = InMemoryFontRegistry::new();
        let handle = registry
            .register(FontSource::Project, FontFaceKey::new(hash(1), 0))
            .expect("등록");
        let resolved = chain.resolve(&registry);
        assert_eq!(
            resolved.available().collect::<Vec<_>>(),
            vec![(0, handle), (1, handle)]
        );
        assert_eq!(
            resolved.select(|_| true),
            FallbackSelection::Face {
                chain_index: 0,
                handle
            }
        );
    }

    #[test]
    fn decode_rejects_invalid_chains() {
        assert_eq!(
            FontFallbackChain::decode(&[0, 0, 0, 0]),
            Err(FallbackChainError::Empty)
        );
        assert_eq!(
            FontFallbackChain::decode(&[1, 0]),
            Err(FallbackChainError::Record(FontRecordError::Truncated))
        );
        let fx = fixture();
        let bytes = fx.chain.encode();
        assert_eq!(
            FontFallbackChain::decode(&bytes[..bytes.len() - 1]),
            Err(FallbackChainError::Record(FontRecordError::Truncated))
        );
        let mut trailing = bytes.clone();
        trailing.push(0xFF);
        assert_eq!(
            FontFallbackChain::decode(&trailing),
            Err(FallbackChainError::Record(FontRecordError::TrailingBytes {
                remaining: 1
            }))
        );
        // 개수 u32::MAX: 큰 할당 없이 첫 항목에서 Truncated.
        assert_eq!(
            FontFallbackChain::decode(&u32::MAX.to_le_bytes()),
            Err(FallbackChainError::Record(FontRecordError::Truncated))
        );
        // 중복 항목을 가진 bytes도 거부합니다.
        let mut duplicated = vec![2, 0, 0, 0];
        duplicated.extend_from_slice(&face(FontSource::Project, 3).encode());
        duplicated.extend_from_slice(&face(FontSource::Project, 3).encode());
        assert_eq!(
            FontFallbackChain::decode(&duplicated),
            Err(FallbackChainError::DuplicateFace {
                first: 0,
                duplicate: 1
            })
        );
        // 항목 안의 -0.0 axis 값도 non-canonical로 거부합니다(X.8.1 decode 규칙 상속).
        let entry = FontFaceRef::new(FontSource::Project, hash(4), 0, vec![axis("slnt", 2.0)]);
        let mut negative_zero = vec![1, 0, 0, 0];
        negative_zero.extend_from_slice(&entry.encode());
        // chain 개수 4 bytes + 항목 안 첫 axis 값 offset 45.
        negative_zero[4 + 45..4 + 49].copy_from_slice(&(-0.0f32).to_le_bytes());
        assert_eq!(
            FontFallbackChain::decode(&negative_zero),
            Err(FallbackChainError::Record(
                FontRecordError::NonCanonicalAxisValue {
                    tag: crate::font::tests::tag("slnt")
                }
            ))
        );
        let err = FallbackChainError::Record(FontRecordError::Truncated);
        assert!(std::error::Error::source(&err).is_some());
        assert!(err.to_string().contains("부족"));
    }
}
