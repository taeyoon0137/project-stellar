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

            /// 이미 검증된 `FiniteF32`로부터 만듭니다(재검증 없음).
            pub const fn from_raw(raw: FiniteF32) -> $name {
                $name(raw)
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

/// 2D 벡터(finite 전용). (III.3.3)
///
/// 성분이 [`FiniteF32`](전순서)라 `(x, y)` 사전식 전순서를 파생합니다(canonical 비교용).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Vec2 {
    x: FiniteF32,
    y: FiniteF32,
}

impl Vec2 {
    /// 원점.
    pub const ZERO: Vec2 = Vec2 {
        x: FiniteF32::ZERO,
        y: FiniteF32::ZERO,
    };

    /// finite 성분으로 벡터를 만듭니다.
    pub fn new(x: f32, y: f32) -> Result<Vec2, NonFiniteError> {
        Ok(Vec2 {
            x: FiniteF32::new(x)?,
            y: FiniteF32::new(y)?,
        })
    }

    /// x 성분.
    pub const fn x(&self) -> f32 {
        self.x.get()
    }

    /// y 성분.
    pub const fn y(&self) -> f32 {
        self.y.get()
    }
}

/// 크기 생성 오류(III.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeError {
    /// 비유한 값.
    NonFinite,
    /// 음수 크기(정책상 불허).
    Negative,
}

impl fmt::Display for SizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeError::NonFinite => f.write_str("크기 값이 유한하지 않습니다"),
            SizeError::Negative => f.write_str("크기는 음수일 수 없습니다"),
        }
    }
}

impl std::error::Error for SizeError {}

/// 2D 크기(finite, 비음수 전용). 음수 크기는 정책상 거부합니다. (III.3.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size2 {
    width: FiniteF32,
    height: FiniteF32,
}

impl Size2 {
    /// 0 크기.
    pub const ZERO: Size2 = Size2 {
        width: FiniteF32::ZERO,
        height: FiniteF32::ZERO,
    };

    /// finite·비음수 크기를 만듭니다. 비유한은 `NonFinite`, 음수는 `Negative`로 거부합니다.
    pub fn new(width: f32, height: f32) -> Result<Size2, SizeError> {
        let width = FiniteF32::new(width).map_err(|_| SizeError::NonFinite)?;
        let height = FiniteF32::new(height).map_err(|_| SizeError::NonFinite)?;
        if width.get() < 0.0 || height.get() < 0.0 {
            return Err(SizeError::Negative);
        }
        Ok(Size2 { width, height })
    }

    /// 너비.
    pub const fn width(&self) -> f32 {
        self.width.get()
    }

    /// 높이.
    pub const fn height(&self) -> f32 {
        self.height.get()
    }

    /// 면적이 0이면(너비 또는 높이가 0) 빈 크기입니다.
    pub fn is_empty(&self) -> bool {
        self.width.get() == 0.0 || self.height.get() == 0.0
    }
}

/// 축 정렬 사각형(origin + size). (III.3.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    origin: Vec2,
    size: Size2,
}

impl Rect {
    /// origin과 size로 사각형을 만듭니다.
    pub const fn new(origin: Vec2, size: Size2) -> Rect {
        Rect { origin, size }
    }

    /// origin.
    pub const fn origin(&self) -> Vec2 {
        self.origin
    }

    /// size.
    pub const fn size(&self) -> Size2 {
        self.size
    }

    /// 면적이 0이면 빈 사각형입니다.
    pub fn is_empty(&self) -> bool {
        self.size.is_empty()
    }
}

/// 사방 inset(finite 전용). (III.3.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Insets {
    top: FiniteF32,
    right: FiniteF32,
    bottom: FiniteF32,
    left: FiniteF32,
}

impl Insets {
    /// finite 성분으로 inset을 만듭니다.
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Result<Insets, NonFiniteError> {
        Ok(Insets {
            top: FiniteF32::new(top)?,
            right: FiniteF32::new(right)?,
            bottom: FiniteF32::new(bottom)?,
            left: FiniteF32::new(left)?,
        })
    }

    /// (top, right, bottom, left).
    pub const fn components(&self) -> (f32, f32, f32, f32) {
        (
            self.top.get(),
            self.right.get(),
            self.bottom.get(),
            self.left.get(),
        )
    }
}

/// 2D 아핀 변환(finite 전용). 점 (x,y)를 (a·x + c·y + tx, b·x + d·y + ty)로 보냅니다. (III.3.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Transform2D {
    a: FiniteF32,
    b: FiniteF32,
    c: FiniteF32,
    d: FiniteF32,
    tx: FiniteF32,
    ty: FiniteF32,
}

impl Transform2D {
    /// 항등 변환.
    pub fn identity() -> Transform2D {
        Transform2D::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).expect("identity는 finite")
    }

    /// finite 성분으로 변환을 만듭니다.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        tx: f32,
        ty: f32,
    ) -> Result<Transform2D, NonFiniteError> {
        Ok(Transform2D {
            a: FiniteF32::new(a)?,
            b: FiniteF32::new(b)?,
            c: FiniteF32::new(c)?,
            d: FiniteF32::new(d)?,
            tx: FiniteF32::new(tx)?,
            ty: FiniteF32::new(ty)?,
        })
    }

    /// 선형부 행렬식 a·d − b·c.
    pub fn determinant(&self) -> f32 {
        self.a.get() * self.d.get() - self.b.get() * self.c.get()
    }

    /// 점을 변환합니다.
    pub fn apply(&self, point: Vec2) -> Result<Vec2, NonFiniteError> {
        let x = self.a.get() * point.x() + self.c.get() * point.y() + self.tx.get();
        let y = self.b.get() * point.x() + self.d.get() * point.y() + self.ty.get();
        Vec2::new(x, y)
    }

    /// 역변환을 반환합니다. 행렬식이 0이거나 결과가 비유한이면 `None`(역행렬 불가).
    pub fn inverse(&self) -> Option<Transform2D> {
        let det = self.determinant();
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        let ia = self.d.get() * inv_det;
        let ic = -self.c.get() * inv_det;
        let ib = -self.b.get() * inv_det;
        let id = self.a.get() * inv_det;
        let itx = -(ia * self.tx.get() + ic * self.ty.get());
        let ity = -(ib * self.tx.get() + id * self.ty.get());
        // 결과가 비유한이면(오버플로) None.
        Transform2D::new(ia, ib, ic, id, itx, ity).ok()
    }
}

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
    fn vec2_and_insets_reject_nonfinite() {
        assert_eq!(Vec2::new(f32::NAN, 0.0), Err(NonFiniteError));
        assert_eq!(Vec2::new(1.0, f32::INFINITY), Err(NonFiniteError));
        let v = Vec2::new(3.0, -4.0).unwrap();
        assert_eq!((v.x(), v.y()), (3.0, -4.0));
        assert_eq!(Insets::new(1.0, 2.0, f32::NAN, 4.0), Err(NonFiniteError));
        assert_eq!(
            Insets::new(1.0, 2.0, 3.0, 4.0).unwrap().components(),
            (1.0, 2.0, 3.0, 4.0)
        );
    }

    #[test]
    fn size_negative_policy() {
        assert_eq!(Size2::new(-1.0, 5.0), Err(SizeError::Negative));
        assert_eq!(Size2::new(5.0, -0.001), Err(SizeError::Negative));
        assert_eq!(Size2::new(f32::INFINITY, 1.0), Err(SizeError::NonFinite));
        let s = Size2::new(4.0, 3.0).unwrap();
        assert_eq!((s.width(), s.height()), (4.0, 3.0));
    }

    #[test]
    fn rect_empty_when_zero_area() {
        let origin = Vec2::new(1.0, 2.0).unwrap();
        let empty = Rect::new(origin, Size2::new(0.0, 10.0).unwrap());
        assert!(empty.is_empty());
        let nonempty = Rect::new(origin, Size2::new(5.0, 10.0).unwrap());
        assert!(!nonempty.is_empty());
        assert_eq!(nonempty.origin(), origin);
    }

    #[test]
    fn transform_invertible_and_singular() {
        // 2배 스케일 → 역행렬 가능. 역변환 후 forward로 round-trip.
        let scale = Transform2D::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0).unwrap();
        let inv = scale.inverse().expect("invertible");
        let point = Vec2::new(8.0, 6.0).unwrap();
        let back = inv.apply(point).unwrap();
        assert_eq!((back.x(), back.y()), (4.0, 3.0));
        let again = scale.apply(back).unwrap();
        assert_eq!((again.x(), again.y()), (8.0, 6.0));

        // 항등은 역행렬 가능.
        assert!(Transform2D::identity().inverse().is_some());

        // det == 0 → 역행렬 불가.
        let singular = Transform2D::new(1.0, 2.0, 2.0, 4.0, 0.0, 0.0).unwrap();
        assert_eq!(singular.determinant(), 0.0);
        assert!(singular.inverse().is_none());
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
