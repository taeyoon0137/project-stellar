//! 그래프 node 희소 배치 hint (XXX.1.5, ADR-0025 결정 2.6, ADR-0094 결정 5).
//!
//! 기본 배치는 결정론적 자동 배치(XXX.2.2)이므로 저장하지 않습니다. 사용자가 고정한 node 위치만 StableId→좌표로
//! 저장합니다. hint가 없으면 binary·텍스트 모두 0 bytes입니다. hint 변경은 Semantic Diff에서 표현 전용 변경입니다
//! ([`diff_hints`], [`ChangeClass::Presentation`]).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;

/// 배치 좌표(그래프 면 단위, 유한).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HintPoint {
    x: f32,
    y: f32,
}

/// 좌표 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonFiniteHint;

impl fmt::Display for NonFiniteHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("배치 hint 좌표가 유한하지 않습니다")
    }
}

impl std::error::Error for NonFiniteHint {}

/// 좌표 절댓값 상한.
pub const MAX_HINT_COORDINATE: f32 = 1.0e7;

impl HintPoint {
    /// 좌표를 만듭니다. `-0`은 `0`으로 정규화합니다.
    pub fn new(x: f32, y: f32) -> Result<HintPoint, NonFiniteHint> {
        let ok = |v: f32| v.is_finite() && v.abs() <= MAX_HINT_COORDINATE;
        if !ok(x) || !ok(y) {
            return Err(NonFiniteHint);
        }
        Ok(HintPoint {
            x: x + 0.0,
            y: y + 0.0,
        })
    }

    /// x.
    pub const fn x(&self) -> f32 {
        self.x
    }

    /// y.
    pub const fn y(&self) -> f32 {
        self.y
    }
}

/// 소유자 하나의 배치 hint 모음(StableId 순).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutHints {
    hints: BTreeMap<StableId, HintPoint>,
}

impl LayoutHints {
    /// 빈 모음.
    pub fn new() -> LayoutHints {
        LayoutHints::default()
    }

    /// hint 수.
    pub fn len(&self) -> usize {
        self.hints.len()
    }

    /// 비었는지.
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }

    /// node의 hint.
    pub fn get(&self, node: StableId) -> Option<HintPoint> {
        self.hints.get(&node).copied()
    }

    /// 고정합니다. 이전 hint를 돌려줍니다.
    pub fn pin(&mut self, node: StableId, point: HintPoint) -> Option<HintPoint> {
        self.hints.insert(node, point)
    }

    /// 고정을 풉니다. 이전 hint를 돌려줍니다.
    pub fn unpin(&mut self, node: StableId) -> Option<HintPoint> {
        self.hints.remove(&node)
    }

    /// 모든 hint(StableId 순).
    pub fn iter(&self) -> impl Iterator<Item = (StableId, HintPoint)> + '_ {
        self.hints.iter().map(|(id, p)| (*id, *p))
    }

    /// canonical binary: 빈 모음은 0 bytes, 그 밖은 count u32 LE ++ (id[16] ++ x f32 LE ++ y f32 LE)*.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.hints.is_empty() {
            return out;
        }
        out.extend_from_slice(&(self.hints.len() as u32).to_le_bytes());
        for (id, point) in &self.hints {
            out.extend_from_slice(&id.to_bytes());
            out.extend_from_slice(&point.x.to_le_bytes());
            out.extend_from_slice(&point.y.to_le_bytes());
        }
        out
    }

    /// canonical binary를 읽습니다.
    pub fn decode(bytes: &[u8]) -> Result<LayoutHints, HintDecodeError> {
        let mut hints = LayoutHints::new();
        if bytes.is_empty() {
            return Ok(hints);
        }
        if bytes.len() < 4 {
            return Err(HintDecodeError("길이"));
        }
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if count == 0 || bytes.len() != 4 + count * 24 {
            return Err(HintDecodeError("길이"));
        }
        let mut previous: Option<StableId> = None;
        for chunk in bytes[4..].chunks_exact(24) {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&chunk[..16]);
            let id = StableId::from_bytes(raw);
            if previous.is_some_and(|p| p >= id) {
                return Err(HintDecodeError("id 오름차순이 아님"));
            }
            previous = Some(id);
            let x = f32::from_le_bytes([chunk[16], chunk[17], chunk[18], chunk[19]]);
            let y = f32::from_le_bytes([chunk[20], chunk[21], chunk[22], chunk[23]]);
            if x.to_bits() == (-0.0f32).to_bits() || y.to_bits() == (-0.0f32).to_bits() {
                return Err(HintDecodeError("-0 좌표"));
            }
            let point = HintPoint::new(x, y).map_err(|_| HintDecodeError("좌표"))?;
            hints.pin(id, point);
        }
        Ok(hints)
    }
}

/// hint binary 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintDecodeError(pub &'static str);

impl fmt::Display for HintDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "배치 hint 형식 오류: {}", self.0)
    }
}

impl std::error::Error for HintDecodeError {}

/// 변경 분류(Semantic Diff의 stage 단위).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChangeClass {
    /// 문서 의미가 바뀌는 변경.
    Semantic,
    /// 표현 전용 변경(배치 hint). 의미 변경과 따로 stage할 수 있습니다.
    Presentation,
}

/// hint 변경 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HintChange {
    /// node.
    pub node: StableId,
    /// 이전(없으면 새로 고정).
    pub before: Option<HintPoint>,
    /// 이후(없으면 고정 해제).
    pub after: Option<HintPoint>,
}

impl HintChange {
    /// 언제나 [`ChangeClass::Presentation`]입니다.
    pub const fn class(&self) -> ChangeClass {
        ChangeClass::Presentation
    }
}

/// 두 hint 모음의 변경(node StableId 순).
pub fn diff_hints(before: &LayoutHints, after: &LayoutHints) -> Vec<HintChange> {
    let mut ids: Vec<StableId> = before
        .hints
        .keys()
        .chain(after.hints.keys())
        .copied()
        .collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .filter_map(|node| {
            let (b, a) = (before.get(node), after.get(node));
            (b != a).then_some(HintChange {
                node,
                before: b,
                after: a,
            })
        })
        .collect()
}

/// `base`에 `changes` 중 고른 것만 적용한 모음(표현 변경만 따로 stage하는 경로).
pub fn apply_hint_changes(base: &LayoutHints, changes: &[HintChange]) -> LayoutHints {
    let mut out = base.clone();
    for change in changes {
        match change.after {
            Some(point) => {
                out.pin(change.node, point);
            }
            None => {
                out.unpin(change.node);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn p(x: f32, y: f32) -> HintPoint {
        HintPoint::new(x, y).expect("좌표")
    }

    #[test]
    fn graph_without_hints_saves_zero_bytes() {
        // 검증 기준(XXX.1.5): hint 없는 그래프의 save bytes에 배치 정보 0.
        assert!(LayoutHints::new().encode().is_empty());
        assert_eq!(LayoutHints::decode(&[]), Ok(LayoutHints::new()));
    }

    #[test]
    fn hints_round_trip() {
        let mut hints = LayoutHints::new();
        hints.pin(sid(2), p(10.5, -4.0));
        hints.pin(sid(1), p(-0.0, 3.0));
        let bytes = hints.encode();
        assert_eq!(LayoutHints::decode(&bytes), Ok(hints.clone()));
        assert_eq!(bytes.len(), 4 + 2 * 24);
        assert!(HintPoint::new(f32::NAN, 0.0).is_err());
    }

    #[test]
    fn hint_change_is_presentation_and_stages_separately() {
        // 검증 기준(XXX.1.5): hint 변경의 diff kind 정확.
        let mut before = LayoutHints::new();
        before.pin(sid(1), p(0.0, 0.0));
        before.pin(sid(2), p(5.0, 5.0));
        let mut after = before.clone();
        after.pin(sid(1), p(20.0, 0.0));
        after.unpin(sid(2));
        after.pin(sid(3), p(1.0, 1.0));
        let changes = diff_hints(&before, &after);
        assert_eq!(changes.len(), 3);
        assert!(changes
            .iter()
            .all(|c| c.class() == ChangeClass::Presentation));
        assert_eq!(apply_hint_changes(&before, &changes), after);
        // 하나만 stage.
        let staged = apply_hint_changes(&before, &changes[..1]);
        assert_eq!(staged.get(sid(1)), Some(p(20.0, 0.0)));
        assert_eq!(staged.get(sid(2)), Some(p(5.0, 5.0)));
    }
}
