//! semantic snapshot 속성 값 (XIV.3.1).
//!
//! 속성 값은 canonical 비교가 가능한 **닫힌 집합**입니다. float은 [`FiniteF32`]로만 담으므로
//! NaN·±무한대가 없고 `-0.0`은 `0.0`으로 정규화되어 있습니다. 값 비교는
//! [`SemanticValue::canonical_eq`]로 하며 float은 **bit 단위**로 비교합니다(epsilon 없음,
//! XIV.3.4).
//!
//! canonical bytes 형식(모든 정수는 little-endian):
//!
//! | 종류 | tag | payload |
//! |---|---|---|
//! | Bool | `0x01` | `u8` 0/1 |
//! | Uint | `0x02` | `u64` |
//! | Text | `0x03` | `u32` 길이 + UTF-8 |
//! | Id | `0x04` | 16바이트 StableId |
//! | Ids | `0x05` | `u32` 개수 + 16바이트 × n |
//! | Floats | `0x06` | `u32` 개수 + `f32` × n |
//! | Typed | `0x07` | `u32` 길이 + [`TypedValue::to_bytes`] |
//! | Bytes | `0x08` | `u32` 길이 + bytes |
//!
//! 길이·개수는 `u32` 범위(4 GiB 미만)를 전제로 합니다. 값은 재귀 구조를 갖지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::FiniteF32;

/// semantic snapshot 속성 값.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticValue {
    /// 불리언(예: visible·locked·modifier enabled).
    Bool(bool),
    /// 부호 없는 정수(예: enum tag, 다각형 변 수).
    Uint(u64),
    /// 문자열(예: 표시 이름, 프로젝트 metadata 값).
    Text(String),
    /// 단일 StableId 참조.
    Id(StableId),
    /// 순서 있는 StableId 목록(예: 페이지·View·Slot 목록).
    Ids(Vec<StableId>),
    /// 순서 있는 유한 float 목록(예: size, transform, radius).
    Floats(Vec<FiniteF32>),
    /// core-types의 타입드 값(예: 색상, override 값).
    Typed(TypedValue),
    /// 문서 crate canonical 인코딩 등 불투명 canonical bytes(예: path command, text payload).
    Bytes(Vec<u8>),
}

/// 값의 타입: 값 종류 tag와(Typed이면) [`TypedValue`] type tag.
///
/// type-change 분류(XIV.10.5)에 씁니다. 같은 종류의 다른 값은 같은 타입입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueType {
    /// [`SemanticValue::tag`].
    pub tag: u8,
    /// Typed 값의 [`TypedValue::type_tag`]. 다른 종류는 `None`.
    pub typed: Option<u8>,
}

impl SemanticValue {
    /// Bool tag.
    pub const TAG_BOOL: u8 = 0x01;
    /// Uint tag.
    pub const TAG_UINT: u8 = 0x02;
    /// Text tag.
    pub const TAG_TEXT: u8 = 0x03;
    /// Id tag.
    pub const TAG_ID: u8 = 0x04;
    /// Ids tag.
    pub const TAG_IDS: u8 = 0x05;
    /// Floats tag.
    pub const TAG_FLOATS: u8 = 0x06;
    /// Typed tag.
    pub const TAG_TYPED: u8 = 0x07;
    /// Bytes tag.
    pub const TAG_BYTES: u8 = 0x08;

    /// 값 종류 tag(canonical bytes의 첫 바이트).
    pub const fn tag(&self) -> u8 {
        match self {
            SemanticValue::Bool(_) => SemanticValue::TAG_BOOL,
            SemanticValue::Uint(_) => SemanticValue::TAG_UINT,
            SemanticValue::Text(_) => SemanticValue::TAG_TEXT,
            SemanticValue::Id(_) => SemanticValue::TAG_ID,
            SemanticValue::Ids(_) => SemanticValue::TAG_IDS,
            SemanticValue::Floats(_) => SemanticValue::TAG_FLOATS,
            SemanticValue::Typed(_) => SemanticValue::TAG_TYPED,
            SemanticValue::Bytes(_) => SemanticValue::TAG_BYTES,
        }
    }

    /// 값의 타입.
    pub fn value_type(&self) -> ValueType {
        ValueType {
            tag: self.tag(),
            typed: match self {
                SemanticValue::Typed(value) => Some(value.type_tag()),
                _ => None,
            },
        }
    }

    /// canonical 의미로 같은 값인지. float은 bit 단위로 비교하고 epsilon을 쓰지 않습니다.
    ///
    /// [`SemanticValue::to_canonical_bytes`]가 같은 것과 정확히 같은 결과입니다(테스트로
    /// 고정). 대부분의 종류는 할당 없이 비교합니다.
    pub fn canonical_eq(&self, other: &SemanticValue) -> bool {
        match (self, other) {
            (SemanticValue::Bool(a), SemanticValue::Bool(b)) => a == b,
            (SemanticValue::Uint(a), SemanticValue::Uint(b)) => a == b,
            (SemanticValue::Text(a), SemanticValue::Text(b)) => a == b,
            (SemanticValue::Id(a), SemanticValue::Id(b)) => a == b,
            (SemanticValue::Ids(a), SemanticValue::Ids(b)) => a == b,
            (SemanticValue::Floats(a), SemanticValue::Floats(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| x.to_le_bytes() == y.to_le_bytes())
            }
            (SemanticValue::Typed(a), SemanticValue::Typed(b)) => a.to_bytes() == b.to_bytes(),
            (SemanticValue::Bytes(a), SemanticValue::Bytes(b)) => a == b,
            _ => false,
        }
    }

    /// canonical bytes를 `out`에 덧붙입니다(모듈 문서 형식).
    pub fn write_canonical(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        match self {
            SemanticValue::Bool(value) => out.push(u8::from(*value)),
            SemanticValue::Uint(value) => out.extend_from_slice(&value.to_le_bytes()),
            SemanticValue::Text(value) => put_blob(out, value.as_bytes()),
            SemanticValue::Id(value) => out.extend_from_slice(&value.to_bytes()),
            SemanticValue::Ids(values) => {
                put_len(out, values.len());
                for value in values {
                    out.extend_from_slice(&value.to_bytes());
                }
            }
            SemanticValue::Floats(values) => {
                put_len(out, values.len());
                for value in values {
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
            SemanticValue::Typed(value) => put_blob(out, &value.to_bytes()),
            SemanticValue::Bytes(value) => put_blob(out, value),
        }
    }

    /// canonical bytes.
    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_canonical(&mut out);
        out
    }
}

/// `u32` little-endian 길이·개수.
pub(crate) fn put_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as u32).to_le_bytes());
}

/// `u32` 길이 프리픽스 + bytes.
pub(crate) fn put_blob(out: &mut Vec<u8>, bytes: &[u8]) {
    put_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::value::Length;

    fn finite(v: f32) -> FiniteF32 {
        FiniteF32::new(v).unwrap()
    }

    fn samples() -> Vec<SemanticValue> {
        let one = finite(1.0);
        let next = finite(f32::from_bits(1.0f32.to_bits() + 1));
        vec![
            SemanticValue::Bool(false),
            SemanticValue::Bool(true),
            SemanticValue::Uint(0),
            SemanticValue::Uint(7),
            SemanticValue::Text(String::new()),
            SemanticValue::Text("이름".into()),
            SemanticValue::Id(StableId::new_v7(1, 0, 1)),
            SemanticValue::Id(StableId::new_v7(2, 0, 2)),
            SemanticValue::Ids(vec![]),
            SemanticValue::Ids(vec![StableId::new_v7(1, 0, 1)]),
            SemanticValue::Floats(vec![one]),
            SemanticValue::Floats(vec![next]),
            SemanticValue::Floats(vec![one, one]),
            SemanticValue::Typed(TypedValue::Number(one)),
            SemanticValue::Typed(TypedValue::Number(next)),
            SemanticValue::Typed(TypedValue::Length(Length::from_raw(one))),
            SemanticValue::Typed(TypedValue::Color(
                Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap(),
            )),
            SemanticValue::Bytes(vec![]),
            SemanticValue::Bytes(vec![1, 2, 3]),
        ]
    }

    #[test]
    fn canonical_eq_matches_canonical_bytes() {
        let values = samples();
        for a in &values {
            for b in &values {
                assert_eq!(
                    a.canonical_eq(b),
                    a.to_canonical_bytes() == b.to_canonical_bytes(),
                    "{a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn bit_different_finite_floats_are_different() {
        let one = finite(1.0);
        let next = finite(f32::from_bits(1.0f32.to_bits() + 1));
        assert!(
            (one.get() - next.get()).abs() < 1e-6,
            "epsilon 비교라면 같다고 볼 값"
        );
        let a = SemanticValue::Floats(vec![one]);
        let b = SemanticValue::Floats(vec![next]);
        assert!(!a.canonical_eq(&b));
        // -0.0은 생성 단계에서 0.0으로 정규화되어 같은 값입니다.
        let zero = SemanticValue::Floats(vec![finite(0.0)]);
        let negative_zero = SemanticValue::Floats(vec![finite(-0.0)]);
        assert!(zero.canonical_eq(&negative_zero));
    }

    #[test]
    fn value_type_distinguishes_typed_tags() {
        let number = SemanticValue::Typed(TypedValue::Number(finite(1.0)));
        let other_number = SemanticValue::Typed(TypedValue::Number(finite(2.0)));
        let length = SemanticValue::Typed(TypedValue::Length(Length::from_raw(finite(1.0))));
        assert_eq!(number.value_type(), other_number.value_type());
        assert_ne!(number.value_type(), length.value_type());
        assert_ne!(
            SemanticValue::Uint(1).value_type(),
            SemanticValue::Bool(true).value_type()
        );
    }

    #[test]
    fn canonical_bytes_golden() {
        assert_eq!(
            SemanticValue::Uint(0x0102).to_canonical_bytes(),
            vec![0x02, 0x02, 0x01, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            SemanticValue::Text("ab".into()).to_canonical_bytes(),
            vec![0x03, 2, 0, 0, 0, b'a', b'b']
        );
        assert_eq!(
            SemanticValue::Floats(vec![finite(1.0)]).to_canonical_bytes(),
            vec![0x06, 1, 0, 0, 0, 0x00, 0x00, 0x80, 0x3F]
        );
        assert_eq!(
            SemanticValue::Bool(true).to_canonical_bytes(),
            vec![0x01, 1]
        );
    }
}
