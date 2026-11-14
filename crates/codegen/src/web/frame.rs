//! 저장된 고정 Free Frame의 CSS 값 출력. 문서·속성 key·layout record를 정의하지 않습니다.
//! 트리는 기존 IrTree → HTML/ARIA mapper가 담당하고, 이 모듈은 요소 하나의 값만 받습니다.

use std::fmt::Write as _;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::{FiniteF32, Size2, Transform2D};

use crate::text::num;

/// 고정 Frame 하나의 출력 값. 저장 스키마나 범용 코드 생성 IR이 아닙니다.
pub struct FrameStyle {
    /// border box 크기.
    pub size: Size2,
    /// 원점이 border box 좌상단인 저장 affine.
    pub transform: Transform2D,
    /// 단색 배경.
    pub background: Option<Color>,
    /// 네 변이 같은 solid 테두리(색, 두께).
    pub border: Option<(Color, FiniteF32)>,
    /// 좌상·우상·우하·좌하의 원형 반경.
    pub radii: [FiniteF32; 4],
    /// border box와 바깥 radius에서 자식을 clip.
    pub clip: bool,
    /// Fragment 조상까지 반영한 표시 여부.
    pub visible: bool,
    /// None이면 View root, Some이면 가장 가까운 Frame 부모의 border 두께.
    pub parent_border: Option<FiniteF32>,
}

impl FrameStyle {
    /// CSS가 값을 조용히 바꾸는 경우를 거부합니다. 기준은 Chromium, DPR 1, zoom 100%입니다.
    pub fn validate(&self) -> Result<(), &'static str> {
        // Chromium LayoutUnit 포화보다 작은 명시적 출력 범위. root canvas 이동은 출력하지 않습니다.
        let (a, b, c, d, x, y) = self.transform.components();
        if [a, b, c, d].iter().any(|v| v.abs() > 1024.0) {
            return Err("affine 선형 성분은 절댓값 1024 이하만 지원합니다");
        }
        let lengths = [self.size.width(), self.size.height()]
            .into_iter()
            .chain(self.radii.iter().map(|v| v.get()))
            .chain(self.parent_border.into_iter().flat_map(|_| [x, y]));
        if lengths.into_iter().any(|v| v.abs() > 1_000_000.0) {
            return Err("고정 Frame의 크기·위치·반경은 절댓값 1,000,000px 이하만 지원합니다");
        }
        if self.radii.iter().any(|v| v.get() < 0.0) {
            return Err("음수 반경은 지원하지 않습니다");
        }
        if let Some((_, width)) = self.border {
            let width = width.get();
            if width < 0.0 || width.fract() != 0.0 {
                return Err("브라우저 border 폭의 device-pixel 반올림을 피하기 위해 비음수 정수 px 테두리만 지원합니다");
            }
            if 2.0 * width > self.size.width().min(self.size.height()) {
                return Err("양쪽 border 합이 저장 크기를 넘으면 CSS border box가 커지므로 지원하지 않습니다");
            }
        }
        Ok(())
    }

    /// 검증된 값의 선언 블록. 위치 보정은 calc로 출력해 Rust f32 뺄셈 손실을 피합니다.
    pub fn declarations(&self) -> String {
        let mut out = String::new();
        let mut declaration = |name: &str, value: String| {
            let _ = writeln!(out, "  {name}: {value};");
        };
        declaration("box-sizing", "border-box".into());
        declaration(
            "display",
            if self.visible { "block" } else { "none" }.into(),
        );
        declaration(
            "position",
            if self.parent_border.is_some() {
                "absolute"
            } else {
                "relative"
            }
            .into(),
        );
        declaration("z-index", "0".into());
        declaration("width", format!("{}px", num(self.size.width())));
        declaration("height", format!("{}px", num(self.size.height())));
        let (a, b, c, d, x, y) = self.transform.components();
        if let Some(border) = self.parent_border {
            for (name, value) in [("left", x), ("top", y)] {
                declaration(
                    name,
                    format!("calc({}px - {}px)", num(value), num(border.get())),
                );
            }
        }
        declaration("transform-origin", "0px 0px".into());
        declaration(
            "transform",
            format!(
                "matrix({}, {}, {}, {}, 0, 0)",
                num(a),
                num(b),
                num(c),
                num(d)
            ),
        );
        declaration(
            "overflow-x",
            if self.clip { "clip" } else { "visible" }.into(),
        );
        declaration(
            "overflow-y",
            if self.clip { "clip" } else { "visible" }.into(),
        );
        if self.clip {
            declaration("overflow-clip-margin", "border-box".into());
        }
        declaration(
            "background-color",
            self.background.map_or("transparent".into(), color),
        );
        for side in ["top", "right", "bottom", "left"] {
            declaration(
                &format!("border-{side}-width"),
                format!("{}px", num(self.border.map_or(0.0, |(_, w)| w.get()))),
            );
            declaration(
                &format!("border-{side}-style"),
                if self.border.is_some() {
                    "solid"
                } else {
                    "none"
                }
                .into(),
            );
            declaration(
                &format!("border-{side}-color"),
                self.border.map_or("transparent".into(), |(c, _)| color(c)),
            );
        }
        for (corner, radius) in ["top-left", "top-right", "bottom-right", "bottom-left"]
            .iter()
            .zip(self.radii)
        {
            declaration(
                &format!("border-{corner}-radius"),
                format!("{}px", num(radius.get())),
            );
        }
        out
    }
}

/// 속성 모델 8.2의 정규 CSS 색. 8-bit 왕복이 정확할 때만 hex를 사용합니다.
fn color(color: Color) -> String {
    let channels = [color.r(), color.g(), color.b(), color.alpha()];
    if color.space() == ColorSpace::Srgb
        && channels
            .iter()
            .all(|v| (0.0..=1.0).contains(v) && (v * 255.0).round() / 255.0 == *v)
    {
        let [r, g, b, a] = channels.map(|v| (v * 255.0).round() as u8);
        return if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        };
    }
    let space = match color.space() {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    };
    let alpha = if color.alpha() == 1.0 {
        String::new()
    } else {
        format!(" / {}", num(color.alpha()))
    };
    format!(
        "color({space} {} {} {}{alpha})",
        num(color.r()),
        num(color.g()),
        num(color.b())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_preserve_space_extended_channels_and_non_byte_alpha() {
        let c = |space, r, g, b, a| Color::new(space, r, g, b, a).unwrap();
        assert_eq!(color(c(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0)), "#ff0000");
        assert_eq!(
            color(c(ColorSpace::Srgb, 1.0, 0.0, 0.0, 128.0 / 255.0)),
            "#ff000080"
        );
        assert_eq!(
            color(c(ColorSpace::Srgb, 0.1, 0.2, 0.3, 0.5)),
            "color(srgb 0.1 0.2 0.3 / 0.5)"
        );
        assert_eq!(
            color(c(ColorSpace::DisplayP3, 1.2, -0.1, 0.3, 0.25)),
            "color(display-p3 1.2 -0.1 0.3 / 0.25)"
        );
    }
}
