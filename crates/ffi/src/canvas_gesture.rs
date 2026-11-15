//! 캔버스 직접 조작 C ABI(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(두 점, delta, local 상자)와 앱 설정 값만 넘기고, 새 도형의 modifier 구성과
//! 선택·편집 의미는 코어가 정합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{FiniteF32, Vec2};
use stellar_runtime::app::{ShapeKind, ShapePaint, WindowSessionHandle};

use crate::canvas::StellarCanvasColor;
use crate::{ffi_guard, registry, StellarStatus};

/// 새 도형의 칠(D7). 셸이 앱 설정(Settings ▸ 캔버스)에서 읽어 넘깁니다. 색공간 0은 없음입니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarShapeStyle {
    fill: StellarCanvasColor,
    stroke: StellarCanvasColor,
    stroke_width: f32,
}

impl StellarShapeStyle {
    fn decode(self) -> Result<ShapePaint, StellarStatus> {
        let fill = self.fill.decode()?;
        let stroke = match self.stroke.decode()? {
            Some(color) => {
                let width = FiniteF32::new(self.stroke_width)
                    .ok()
                    .filter(|w| w.get() > 0.0)
                    .ok_or(StellarStatus::InvalidArgument)?;
                Some((color, width))
            }
            None => None,
        };
        Ok(ShapePaint { fill, stroke })
    }
}

/// 부모 local 공간의 시작·끝 두 점과 셸이 넘긴 칠로 Shape를 만들고 선택까지 한 history 항목에 기록합니다.
///
/// `shape`는 ShapeKind 저장 tag(1 Rectangle, 2 Ellipse, 3 Line, 4 Polygon)입니다.
///
/// # Safety
/// `view`·`parent`는 16 byte, `style`은 `StellarShapeStyle` 하나를 읽을 수 있어야 하고 `out_request`는 null 또는
/// 쓰기 가능해야 합니다.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn stellar_window_session_insert_shape(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    shape: u8,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    style: *const StellarShapeStyle,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || parent.is_null() || style.is_null() {
            return StellarStatus::NullPointer;
        }
        let paint = match unsafe { *style }.decode() {
            Ok(paint) => paint,
            Err(status) => return status,
        };
        let (Ok(kind), Ok(start), Ok(end)) = (
            ShapeKind::from_tag(shape),
            Vec2::new(x0, y0),
            Vec2::new(x1, y1),
        ) else {
            return StellarStatus::InvalidArgument;
        };
        let view = StableId::from_bytes(unsafe { *view });
        let parent = StableId::from_bytes(unsafe { *parent });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.insert_canvas_shape(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                parent,
                kind,
                [start, end],
                paint,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};

    #[test]
    fn shape_style_layout_is_fixed() {
        assert_eq!(std::mem::size_of::<StellarShapeStyle>(), 44);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke), 20);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke_width), 40);
    }

    #[test]
    fn shape_style_keeps_absent_paint_and_rejects_invalid_stroke() {
        let gray = Color::new(ColorSpace::Srgb, 0.85, 0.85, 0.85, 1.0).unwrap();
        let style = StellarShapeStyle {
            fill: Some(gray).into(),
            stroke: StellarCanvasColor::default(),
            stroke_width: 0.0,
        };
        assert_eq!(
            style.decode().unwrap(),
            ShapePaint {
                fill: Some(gray),
                stroke: None
            }
        );
        for width in [0.0, -1.0, f32::NAN] {
            let line = StellarShapeStyle {
                fill: StellarCanvasColor::default(),
                stroke: Some(gray).into(),
                stroke_width: width,
            };
            assert_eq!(line.decode(), Err(StellarStatus::InvalidArgument));
        }
    }
}
