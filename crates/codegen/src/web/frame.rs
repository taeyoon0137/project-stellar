//! 저장된 Frame의 CSS 값 출력. 문서·속성 key·layout record를 정의하지 않습니다. 고정 Free Frame 출력이
//! 아니면 layout longhand 선언은 상위(ops)가 [`super::layout`]으로 내고, 이 모듈은 나머지 선언만 냅니다.
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

    /// 브라우저가 크기를 LayoutUnit으로 맞출 때 화면의 모서리·bounds가 1/64px 안에 남는지 검사합니다.
    pub fn size_rounding_fits(&self, world: &Transform2D) -> bool {
        let error = |size: f32| {
            let units = f64::from(size) * 64.0;
            (units.ceil() - units).max(units - units.floor())
        };
        let w = error(self.size.width());
        let h = error(self.size.height());
        let (a, b, c, d, _, _) = world.components();
        f64::from(a.abs()) * w + f64::from(c.abs()) * h <= 1.0
            && f64::from(b.abs()) * w + f64::from(d.abs()) * h <= 1.0
    }

    /// 정수 위치는 layout에 두고 소수 이동은 같은 부모 좌표계의 matrix에 두어 확대 전 반올림을 피합니다.
    ///
    /// 고정 Free Frame 출력(layout record가 없고 활성 세트가 `css`인 출력, ADR-0089 결정 2)의 전체 선언입니다.
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
        if self.parent_border.is_some() {
            for (name, value) in self.free_position() {
                declaration(name, value);
            }
        }
        let translation = if self.parent_border.is_some() {
            self.free_translation()
        } else {
            (0.0, 0.0)
        };
        self.paint(&mut out, translation);
        out
    }

    /// Free 배치(부모 기준 `position: absolute`)의 `left`·`top`: 이동의 정수부에서 가장 가까운 Frame 부모의
    /// border 두께를 뺍니다. 부모가 없으면 border 0으로 봅니다.
    pub fn free_position(&self) -> [(&'static str, String); 2] {
        let (_, _, _, _, x, y) = self.transform.components();
        let border = self.parent_border.map_or(0.0, |b| b.get());
        [("left", x), ("top", y)].map(|(name, value)| {
            (
                name,
                format!("calc({}px - {}px)", num(value.trunc()), num(border)),
            )
        })
    }

    /// Free 배치에서 matrix에 남기는 이동의 소수부.
    pub fn free_translation(&self) -> (f32, f32) {
        let (_, _, _, _, x, y) = self.transform.components();
        (x.fract(), y.fract())
    }

    /// 저장 affine의 이동 성분 전체.
    pub fn translation(&self) -> (f32, f32) {
        let (_, _, _, _, x, y) = self.transform.components();
        (x, y)
    }

    /// 선형 부분이 단위 행렬인지.
    pub fn linear_is_identity(&self) -> bool {
        let (a, b, c, d, _, _) = self.transform.components();
        a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
    }

    /// layout longhand를 뺀 선언: stacking(`z-index`), transform, clip, 배경, 테두리, 반경. layout 출력
    /// (ADR-0089 결정 3)이 layout longhand 선언 뒤에 붙입니다. `translation`은 matrix의 이동 값입니다.
    pub fn paint_declarations(&self, translation: (f32, f32)) -> String {
        let mut out = String::from("  z-index: 0;\n");
        self.paint(&mut out, translation);
        out
    }

    fn paint(&self, out: &mut String, (tx, ty): (f32, f32)) {
        let mut declaration = |name: &str, value: String| {
            let _ = writeln!(out, "  {name}: {value};");
        };
        let (a, b, c, d, _, _) = self.transform.components();
        declaration("transform-origin", "0px 0px".into());
        declaration(
            "transform",
            format!(
                "matrix({}, {}, {}, {}, {}, {})",
                num(a),
                num(b),
                num(c),
                num(d),
                num(tx),
                num(ty)
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
            // 테두리가 반경 이상이면 Chromium의 overflow clip edge에서 안쪽 반경이 0이 됩니다.
            // border-box 자체의 둥근 윤곽을 적용해 모서리 밖의 자식도 같은 경계에서 자릅니다.
            if self.radii.iter().any(|r| r.get() > 0.0) {
                declaration("clip-path", "border-box".into());
            }
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
