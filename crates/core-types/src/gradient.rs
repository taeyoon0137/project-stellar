//! 그라디언트와 stop (III.4.3).
//!
//! [`Gradient`]는 [`GradientStop`]의 **저장 순서**(`stops()`)를 그대로 보존합니다. 렌더링에
//! 쓰는 **시각 순서**(`visual_order()`)는 저장 순서와 분리되어 `(position, 안정 ID)` 기준으로
//! 결정됩니다. 따라서 동일 position을 가진 stop들도 안정 ID로 순서가 고정되며(결정론적),
//! 저장 순서를 바꿔도 시각 순서는 변하지 않습니다.
//!
//! [`Gradient::to_bytes`] ↔ [`Gradient::from_bytes`]로 정확히 round-trip 합니다.

use std::fmt;

use crate::color::{Color, ColorError};
use crate::ids::StableId;
use crate::value::FiniteF32;

/// 그라디언트 stop 생성·디코드 오류(III.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientError {
    /// position이 비유한(NaN·±무한대).
    NonFinitePosition,
    /// stop 색상 디코드 실패.
    Color(ColorError),
    /// 입력이 stop 도중에 끊김.
    Truncated,
    /// stop을 모두 읽은 뒤 잉여 bytes가 남음.
    TrailingBytes,
}

impl fmt::Display for GradientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GradientError::NonFinitePosition => f.write_str("stop position이 유한하지 않습니다"),
            GradientError::Color(err) => write!(f, "stop 색상 디코드 실패: {err}"),
            GradientError::Truncated => f.write_str("stop 도중에 입력이 끊김"),
            GradientError::TrailingBytes => f.write_str("stop을 모두 읽은 뒤 잉여 bytes가 남음"),
        }
    }
}

impl std::error::Error for GradientError {}

/// 그라디언트 stop: 안정 ID + position + 색상. (III.4.3)
///
/// 시각 순서 비교의 tie-break로 안정 ID를 사용하므로, 동일 position stop의 순서가 고정됩니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GradientStop {
    id: StableId,
    position: FiniteF32,
    color: Color,
}

impl GradientStop {
    /// 직렬화 길이(바이트): id 16 + position 4 + color 17.
    pub const BYTE_LEN: usize = 16 + 4 + Color::BYTE_LEN;

    /// 안정 ID·position·색상으로 stop을 만듭니다. position은 유한해야 합니다.
    pub fn new(id: StableId, position: f32, color: Color) -> Result<GradientStop, GradientError> {
        let position = FiniteF32::new(position).map_err(|_| GradientError::NonFinitePosition)?;
        Ok(GradientStop {
            id,
            position,
            color,
        })
    }

    /// 안정 ID.
    pub const fn id(&self) -> StableId {
        self.id
    }

    /// position.
    pub const fn position(&self) -> f32 {
        self.position.get()
    }

    /// 색상.
    pub const fn color(&self) -> Color {
        self.color
    }

    /// 고정 37바이트 직렬화: id(16) ++ position(LE 4) ++ color(17).
    pub fn to_bytes(&self) -> [u8; GradientStop::BYTE_LEN] {
        let mut out = [0u8; GradientStop::BYTE_LEN];
        out[0..16].copy_from_slice(&self.id.to_bytes());
        out[16..20].copy_from_slice(&self.position.to_le_bytes());
        out[20..37].copy_from_slice(&self.color.to_bytes());
        out
    }

    /// [`GradientStop::to_bytes`]의 역.
    pub fn from_bytes(bytes: [u8; GradientStop::BYTE_LEN]) -> Result<GradientStop, GradientError> {
        let mut id_bytes = [0u8; 16];
        id_bytes.copy_from_slice(&bytes[0..16]);
        let id = StableId::from_bytes(id_bytes);

        let position = FiniteF32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]])
            .map_err(|_| GradientError::NonFinitePosition)?;

        let mut color_bytes = [0u8; Color::BYTE_LEN];
        color_bytes.copy_from_slice(&bytes[20..37]);
        let color = Color::from_bytes(color_bytes).map_err(GradientError::Color)?;

        Ok(GradientStop {
            id,
            position,
            color,
        })
    }
}

/// 그라디언트: stop의 저장 순서를 보존하는 컬렉션. (III.4.3)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Gradient {
    stops: Vec<GradientStop>,
}

impl Gradient {
    /// 저장 순서 그대로 stop들을 담은 그라디언트를 만듭니다.
    pub fn new(stops: Vec<GradientStop>) -> Gradient {
        Gradient { stops }
    }

    /// 저장 순서의 stop 슬라이스.
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops
    }

    /// 시각 순서(`(position, 안정 ID)` 기준)로 정렬한 stop 목록.
    ///
    /// 저장 순서와 독립이며, 동일 position stop은 안정 ID로 tie-break 되어 순서가 고정됩니다.
    pub fn visual_order(&self) -> Vec<GradientStop> {
        let mut ordered = self.stops.clone();
        ordered.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
        ordered
    }

    /// 직렬화: stop 개수(u32 BE) ++ 각 stop(37바이트).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.stops.len() * GradientStop::BYTE_LEN);
        out.extend_from_slice(&(self.stops.len() as u32).to_be_bytes());
        for stop in &self.stops {
            out.extend_from_slice(&stop.to_bytes());
        }
        out
    }

    /// [`Gradient::to_bytes`]의 역. 잉여·부족 bytes를 거부합니다.
    pub fn from_bytes(bytes: &[u8]) -> Result<Gradient, GradientError> {
        if bytes.len() < 4 {
            return Err(GradientError::Truncated);
        }
        let count = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let mut pos: usize = 4;
        // count는 입력 길이로 검증됩니다(bogus 큰 count는 곧 Truncated). 미리 할당하지 않습니다.
        let mut stops = Vec::new();
        for _ in 0..count {
            let end = pos
                .checked_add(GradientStop::BYTE_LEN)
                .ok_or(GradientError::Truncated)?;
            if end > bytes.len() {
                return Err(GradientError::Truncated);
            }
            let mut stop_bytes = [0u8; GradientStop::BYTE_LEN];
            stop_bytes.copy_from_slice(&bytes[pos..end]);
            stops.push(GradientStop::from_bytes(stop_bytes)?);
            pos = end;
        }
        if pos != bytes.len() {
            return Err(GradientError::TrailingBytes);
        }
        Ok(Gradient { stops })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorSpace;

    fn stop(id: u64, position: f32) -> GradientStop {
        let color = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();
        GradientStop::new(StableId::new_v7(id, 0, id), position, color).unwrap()
    }

    #[test]
    fn stop_round_trips() {
        let color = Color::new(ColorSpace::DisplayP3, 0.1, 0.2, 0.3, 0.5).unwrap();
        let s = GradientStop::new(StableId::new_v7(7, 1, 2), 0.25, color).unwrap();
        assert_eq!(GradientStop::from_bytes(s.to_bytes()).unwrap(), s);
    }

    #[test]
    fn stop_rejects_non_finite_position() {
        let color = Color::new(ColorSpace::Srgb, 0.0, 0.0, 0.0, 1.0).unwrap();
        assert_eq!(
            GradientStop::new(StableId::NIL, f32::NAN, color),
            Err(GradientError::NonFinitePosition)
        );
    }

    #[test]
    fn gradient_round_trips_preserving_storage_order() {
        // 저장 순서: 0.5, 0.0, 1.0 (정렬되지 않은 순서).
        let g = Gradient::new(vec![stop(2, 0.5), stop(1, 0.0), stop(3, 1.0)]);
        let restored = Gradient::from_bytes(&g.to_bytes()).unwrap();
        assert_eq!(restored, g);
        // 저장 순서는 입력 순서 그대로.
        let positions: Vec<f32> = restored
            .stops()
            .iter()
            .map(GradientStop::position)
            .collect();
        assert_eq!(positions, vec![0.5, 0.0, 1.0]);
    }

    #[test]
    fn empty_gradient_round_trips() {
        let g = Gradient::default();
        assert_eq!(g.to_bytes(), vec![0, 0, 0, 0]);
        assert_eq!(Gradient::from_bytes(&g.to_bytes()).unwrap(), g);
    }

    #[test]
    fn visual_order_is_independent_of_storage_order() {
        let a = stop(1, 0.5);
        let b = stop(2, 0.0);
        let c = stop(3, 1.0);
        // 서로 다른 저장 순서의 두 그라디언트.
        let g1 = Gradient::new(vec![a, b, c]);
        let g2 = Gradient::new(vec![c, a, b]);
        // 시각 순서는 저장 순서와 무관하게 position 기준으로 동일.
        let v1: Vec<f32> = g1
            .visual_order()
            .iter()
            .map(GradientStop::position)
            .collect();
        let v2: Vec<f32> = g2
            .visual_order()
            .iter()
            .map(GradientStop::position)
            .collect();
        assert_eq!(v1, vec![0.0, 0.5, 1.0]);
        assert_eq!(v1, v2);
    }

    #[test]
    fn equal_position_stops_have_stable_order_by_id() {
        // 동일 position(0.5)인 stop 3개, ID는 3 < 7 < 9.
        let s_hi = stop(9, 0.5);
        let s_lo = stop(3, 0.5);
        let s_mid = stop(7, 0.5);
        // 저장 순서를 뒤섞어도 시각 순서는 안정 ID 오름차순으로 고정.
        let g = Gradient::new(vec![s_hi, s_lo, s_mid]);
        let ids: Vec<StableId> = g.visual_order().iter().map(GradientStop::id).collect();
        assert_eq!(ids, vec![s_lo.id(), s_mid.id(), s_hi.id()]);

        // 저장 순서가 달라도 동일 시각 순서(결정론).
        let g2 = Gradient::new(vec![s_mid, s_hi, s_lo]);
        assert_eq!(g.visual_order(), g2.visual_order());
    }

    #[test]
    fn from_bytes_rejects_truncated_and_trailing() {
        // count=1인데 stop bytes 부족.
        let mut truncated = vec![0, 0, 0, 1];
        truncated.extend_from_slice(&[0u8; 10]);
        assert_eq!(
            Gradient::from_bytes(&truncated),
            Err(GradientError::Truncated)
        );

        // 정상 그라디언트 뒤에 잉여 바이트.
        let g = Gradient::new(vec![stop(1, 0.0)]);
        let mut trailing = g.to_bytes();
        trailing.push(0xFF);
        assert_eq!(
            Gradient::from_bytes(&trailing),
            Err(GradientError::TrailingBytes)
        );
    }

    #[test]
    fn from_bytes_rejects_non_finite_position_bytes() {
        let mut bytes = vec![0, 0, 0, 1]; // count=1
        let mut stop_bytes = stop(1, 0.0).to_bytes();
        stop_bytes[16..20].copy_from_slice(&f32::INFINITY.to_le_bytes());
        bytes.extend_from_slice(&stop_bytes);
        assert_eq!(
            Gradient::from_bytes(&bytes),
            Err(GradientError::NonFinitePosition)
        );
    }
}
