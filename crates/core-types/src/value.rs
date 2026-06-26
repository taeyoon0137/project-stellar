//! 타입드 값과 기하 (III.3).
//!
//! 모든 float 값은 [`FiniteF32`]를 거쳐 NaN·±Infinity를 생성·직렬화 단계에서 거부하고,
//! `-0.0`을 `0.0`으로 정규화합니다. 덕분에 값 타입은 전순서(`Ord`)와 `Eq`/`Hash`를 가질 수 있습니다.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// 유한 float 생성 실패(비유한 값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonFiniteError;

impl fmt::Display for NonFiniteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("값이 유한하지 않습니다(NaN 또는 ±무한대)")
    }
}

impl std::error::Error for NonFiniteError {}

/// 유한한 `f32`. NaN·±Infinity는 거부하고 `-0.0`은 `0.0`으로 정규화합니다.
#[derive(Debug, Clone, Copy)]
pub struct FiniteF32(f32);

impl FiniteF32 {
    /// 0 값(정규화된 양의 0).
    pub const ZERO: FiniteF32 = FiniteF32(0.0);

    /// 유한 값이면 `FiniteF32`를, 아니면 오류를 반환합니다. `-0.0`은 `0.0`으로 정규화합니다.
    pub fn new(value: f32) -> Result<FiniteF32, NonFiniteError> {
        if !value.is_finite() {
            return Err(NonFiniteError);
        }
        // value == 0.0 은 +0.0 과 -0.0 모두에 참 → 항상 +0.0 으로 정규화.
        let normalized = if value == 0.0 { 0.0 } else { value };
        Ok(FiniteF32(normalized))
    }

    /// 내부 `f32` 값.
    pub const fn get(&self) -> f32 {
        self.0
    }

    /// little-endian 4바이트 직렬화.
    pub fn to_le_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }

    /// little-endian 4바이트에서 복원합니다. 비유한 값은 거부합니다(-0.0 정규화).
    pub fn from_le_bytes(bytes: [u8; 4]) -> Result<FiniteF32, NonFiniteError> {
        FiniteF32::new(f32::from_le_bytes(bytes))
    }
}

impl PartialEq for FiniteF32 {
    fn eq(&self, other: &Self) -> bool {
        // NaN이 없고 -0.0이 정규화되어 있어 일반 == 비교가 reflexive.
        self.0 == other.0
    }
}

impl Eq for FiniteF32 {}

impl PartialOrd for FiniteF32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FiniteF32 {
    fn cmp(&self, other: &Self) -> Ordering {
        // NaN이 없으므로 partial_cmp는 항상 Some.
        self.0
            .partial_cmp(&other.0)
            .expect("FiniteF32는 NaN이 아님")
    }
}

impl Hash for FiniteF32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // -0.0이 +0.0으로 정규화되어 Eq와 일관된 해시.
        self.0.to_bits().hash(state);
    }
}

/// 유한 산술 결과를 유한 범위로 saturate합니다. finite±finite는 NaN이 될 수 없고
/// 오버플로 시 ±무한대만 가능하므로, 그 경우 `f32::MAX`/`f32::MIN`으로 고정합니다.
fn saturating_finite(value: f32) -> FiniteF32 {
    if value.is_finite() {
        FiniteF32::new(value).expect("finite")
    } else if value > 0.0 {
        FiniteF32::new(f32::MAX).expect("finite")
    } else {
        FiniteF32::new(f32::MIN).expect("finite")
    }
}

/// 단위 newtype을 정의합니다. 같은 단위끼리만 `Add`/`Sub`가 가능합니다(III.3.2).
macro_rules! define_unit {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(FiniteF32);

        impl $name {
            /// 유한 값으로부터 생성합니다(비유한 값은 거부, `-0.0`은 `0.0`으로 정규화).
            pub fn new(value: f32) -> Result<$name, NonFiniteError> {
                Ok($name(FiniteF32::new(value)?))
            }

            /// 내부 `f32` 값.
            pub const fn get(&self) -> f32 {
                self.0.get()
            }

            /// 내부 `FiniteF32`.
            pub const fn raw(&self) -> FiniteF32 {
                self.0
            }
        }

        impl std::ops::Add for $name {
            type Output = $name;

            /// 같은 단위끼리 더합니다. 오버플로(±무한대)는 유한 범위로 saturate합니다.
            fn add(self, rhs: $name) -> $name {
                $name(saturating_finite(self.0.get() + rhs.0.get()))
            }
        }

        impl std::ops::Sub for $name {
            type Output = $name;

            /// 같은 단위끼리 뺍니다. 오버플로(±무한대)는 유한 범위로 saturate합니다.
            fn sub(self, rhs: $name) -> $name {
                $name(saturating_finite(self.0.get() - rhs.0.get()))
            }
        }
    };
}

define_unit!(Length, "길이(논리 픽셀 등). 같은 단위끼리만 산술 가능.");
define_unit!(Percentage, "백분율. 같은 단위끼리만 산술 가능.");
define_unit!(Angle, "각도. 같은 단위끼리만 산술 가능.");
define_unit!(Duration, "지속 시간. 같은 단위끼리만 산술 가능.");
define_unit!(Scalar, "무차원 스칼라. 같은 단위끼리만 산술 가능.");

/// 서로 다른 단위는 더할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::value::{Length, Percentage};
/// let l = Length::new(1.0).unwrap();
/// let p = Percentage::new(1.0).unwrap();
/// let _ = l + p;
/// ```
///
/// 서로 다른 단위는 대입할 수 없습니다(컴파일 실패).
///
/// ```compile_fail
/// use stellar_core_types::value::{Length, Angle};
/// let l = Length::new(1.0).unwrap();
/// let _a: Angle = l;
/// ```
#[cfg(doctest)]
struct UnitCompileFailDocs;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of(value: FiniteF32) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn rejects_nan_and_infinities() {
        assert_eq!(FiniteF32::new(f32::NAN), Err(NonFiniteError));
        assert_eq!(FiniteF32::new(f32::INFINITY), Err(NonFiniteError));
        assert_eq!(FiniteF32::new(f32::NEG_INFINITY), Err(NonFiniteError));
    }

    #[test]
    fn normalizes_negative_zero() {
        let neg = FiniteF32::new(-0.0).expect("finite");
        assert_eq!(neg.get(), 0.0);
        // 비트까지 +0.0 으로 정규화.
        assert_eq!(neg.get().to_bits(), 0u32);
        // +0.0 과 동일(Eq, Hash).
        let pos = FiniteF32::new(0.0).expect("finite");
        assert_eq!(neg, pos);
        assert_eq!(hash_of(neg), hash_of(pos));
    }

    #[test]
    fn preserves_normal_values() {
        assert_eq!(FiniteF32::new(1.5).unwrap().get(), 1.5);
        assert_eq!(FiniteF32::new(-2.5).unwrap().get(), -2.5);
    }

    #[test]
    fn orders_total() {
        let a = FiniteF32::new(-1.0).unwrap();
        let b = FiniteF32::new(0.0).unwrap();
        let c = FiniteF32::new(1.0).unwrap();
        let mut v = [c, a, b];
        v.sort();
        assert_eq!(v, [a, b, c]);
    }

    #[test]
    fn units_same_unit_arithmetic() {
        let a = Length::new(10.0).unwrap();
        let b = Length::new(2.5).unwrap();
        assert_eq!((a + b).get(), 12.5);
        assert_eq!((a - b).get(), 7.5);
        assert_eq!(Angle::new(90.0).unwrap().get(), 90.0);
        assert_eq!(
            Percentage::new(50.0).unwrap().raw(),
            FiniteF32::new(50.0).unwrap()
        );
    }

    #[test]
    fn units_reject_nonfinite() {
        assert_eq!(Length::new(f32::NAN), Err(NonFiniteError));
        assert_eq!(Duration::new(f32::INFINITY), Err(NonFiniteError));
        assert_eq!(Scalar::new(-0.0).unwrap().get(), 0.0);
    }

    #[test]
    fn units_saturate_on_overflow() {
        let big = Length::new(f32::MAX).unwrap();
        // MAX + MAX → +무한대를 유한 범위로 saturate.
        let sum = big + big;
        assert!(sum.get().is_finite());
        assert_eq!(sum.get(), f32::MAX);
    }

    #[test]
    fn serialization_round_trip_and_rejects_nonfinite_bytes() {
        let value = FiniteF32::new(3.25).unwrap();
        let bytes = value.to_le_bytes();
        assert_eq!(FiniteF32::from_le_bytes(bytes).unwrap(), value);

        // NaN 비트 패턴은 직렬화 복원 단계에서 거부.
        let nan_bytes = f32::NAN.to_le_bytes();
        assert_eq!(FiniteF32::from_le_bytes(nan_bytes), Err(NonFiniteError));
        // +Inf 비트 패턴 거부.
        let inf_bytes = f32::INFINITY.to_le_bytes();
        assert_eq!(FiniteF32::from_le_bytes(inf_bytes), Err(NonFiniteError));
    }
}
