//! Stroke tessellation (XI.6.2).
//!
//! [`PathGeometry`]를 [`StrokeStyle`](폭·join·miter limit·cap·dash·평탄화 허용 오차)로 칠할 삼각형
//! mesh([`FillMesh`])로 바꿉니다. 결과는 path local 좌표이며 fill과 같은
//! [`TessellationCache`](crate::tessellation::TessellationCache)를 공유합니다
//! ([`TessellationKey::stroke`](crate::tessellation::TessellationKey::stroke)).
//!
//! ## 기하 규칙
//!
//! - **정렬**: mesh는 항상 path 중심선 양쪽으로 폭의 절반씩입니다. scene [`StrokeAlign`]의
//!   `Inside`/`Outside`는 폭을 두 배로 한 중심 stroke를 fill 영역 안/밖으로 clip해 그립니다
//!   ([`StrokeStyle::from_layer`]). clip은 pass 계획(XI.4.3) 소관입니다.
//! - **곡선**: CubicTo는 균일 매개변수 분할로 평탄화합니다. 분할 수
//!   `n = ceil(sqrt(0.75 · d / tolerance))`(`d`는 두 2차 차분 벡터 길이의 최대값)는 chord 오차가
//!   허용 오차 이하가 되는 상한식입니다. stroke는 chord 방향으로 offset하므로, 이웃 chord 사이 꺾임의
//!   반각 `α`(양 끝은 정확한 접선과 chord 사이 각)에 대해 `반폭 · sin α ≤ tolerance`가 되도록 분할을
//!   늘립니다. 그래서 cap·dash 끝·offset 외곽의 오차도 허용 오차 안입니다. 분할 수는 [`MAX_CUBIC_SEGMENTS`]로 자릅니다(cusp처럼 각이 줄지 않는 곡선은 상한에서
//!   멈춥니다).
//! - **join**: 꺾인 점의 바깥쪽 쐐기만 채웁니다. Miter는 miter 비율(miter 길이 ÷ 폭 =
//!   `1 / sin(θ/2)`, SVG `stroke-miterlimit`와 같은 정의)이 limit 이하일 때 뾰족한 끝을, 넘으면
//!   Bevel을 씁니다. 정확히 되돌아가는 점(180°)의 Miter는 항상 Bevel입니다. 일직선 연결은 join이
//!   없습니다.
//! - **cap**: 열린 subpath 양 끝에만 붙습니다. Butt는 끝점에서 자르고, Square는 반폭만큼 연장하며,
//!   Round는 반원입니다. 닫힌 subpath에는 cap이 없습니다.
//! - **길이 0 subpath**(점): 그리기 command가 있지만 모든 점이 같으면 document Line 규칙(VI.5.4)과
//!   같은 "점 cap"입니다 — Round는 지름 = 폭인 원, Square는 한 변 = 폭인 축 정렬 정사각형, Butt는
//!   아무것도 그리지 않습니다. MoveTo만 있는 subpath는 그리지 않습니다.
//! - **dash**: 간격 목록은 on, off, on, … 순서이고 홀수 개면 두 번 반복해 짝수로 만듭니다(SVG
//!   `stroke-dasharray`). offset만큼 pattern을 앞으로 진행한 위치에서 시작하며 subpath마다 다시
//!   시작합니다. 각 dash는 열린 polyline으로 cap을 갖고, dash 안의 꺾인 점에는 join이 붙습니다.
//!   닫힌 subpath의 시작점과 끝점이 모두 on이면 두 dash를 이어 시작점에도 join을 둡니다. 간격 합이
//!   0인 pattern은 실선과 같습니다. 길이 0인 on 간격은 그 위치의 진행 방향으로 점 cap을 그립니다.
//!   subpath 끝점에서 시작하는 dash는 그릴 길이가 없으므로 만들지 않습니다(시작점의 길이 0 dash는
//!   그립니다).
//! - **원호**: round join·cap·점은 단위 벡터 이등분(정규화한 합)으로 chord 높이가 허용 오차 이하가
//!   될 때까지 나눕니다(최대 깊이 [`MAX_ARC_DEPTH`]). 삼각함수 없이 사칙연산과 `sqrt`만 써서 결과가
//!   플랫폼과 무관하게 같습니다.
//!
//! mesh 삼각형은 서로 겹칠 수 있습니다(join 쐐기와 segment, 자기 교차 path). renderer는 stroke
//! coverage를 합집합(stencil 또는 coverage mask)으로 그려야 하며 삼각형마다 alpha blend하면 겹친 곳이
//! 진해집니다.
//!
//! ## 신뢰되지 않은 입력
//!
//! path는 사용자·import 입력입니다. 평탄화 점 수([`MAX_FLATTENED_POINTS`]), 정점 수
//! ([`MAX_STROKE_VERTICES`]), dash 수([`MAX_DASHES`])를 넘으면 [`TessellationError::TooComplex`]로
//! 멈춥니다. 모든 반복은 명시적 상한이 있고 재귀를 쓰지 않습니다. 정점 좌표가 f32 범위를 넘으면
//! 오류를 반환합니다(panic 없음).

use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::stack::{StrokeAlign, StrokeCap, StrokeLayer};
use stellar_scene::Vec2;

use crate::tessellation::{FillMesh, FlattenTolerance, TessellationError};

/// dash 간격 최대 개수(홀수 입력은 두 번 반복한 뒤 개수).
pub const MAX_DASH_INTERVALS: usize = 16;

/// CubicTo 하나의 최대 평탄화 segment 수.
pub const MAX_CUBIC_SEGMENTS: u32 = 1024;

/// 원호 이등분 최대 깊이(원호 하나당 최대 `2^깊이` segment).
pub const MAX_ARC_DEPTH: u32 = 10;

/// path 평탄화 결과 점 수 상한.
pub const MAX_FLATTENED_POINTS: usize = 1 << 21;

/// stroke mesh 정점 수 상한.
pub const MAX_STROKE_VERTICES: usize = 1 << 22;

/// path 하나에서 만들 수 있는 dash 수 상한.
pub const MAX_DASHES: usize = 1 << 16;

/// dash 진행 반복 상한(길이 0 간격이 섞여도 반드시 끝나도록).
const MAX_DASH_STEPS: usize = 1 << 22;

/// 꺾인 점 모양. tag는 document·scene과 같은 방식의 안정 numeric 값입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StrokeJoin {
    /// 뾰족한 끝(기본). miter limit을 넘으면 Bevel.
    #[default]
    Miter,
    /// 원호.
    Round,
    /// 바깥 두 모서리를 직선으로 잇습니다.
    Bevel,
}

impl StrokeJoin {
    /// 안정 numeric tag(Miter=0, Round=1, Bevel=2).
    pub const fn tag(self) -> u8 {
        match self {
            StrokeJoin::Miter => 0,
            StrokeJoin::Round => 1,
            StrokeJoin::Bevel => 2,
        }
    }

    /// tag에서 복원합니다. 미지 tag는 `None`.
    pub const fn from_tag(tag: u8) -> Option<StrokeJoin> {
        match tag {
            0 => Some(StrokeJoin::Miter),
            1 => Some(StrokeJoin::Round),
            2 => Some(StrokeJoin::Bevel),
            _ => None,
        }
    }
}

/// `-0.0`을 `0.0`으로 바꾼 bits(같은 값은 같은 bits).
fn canonical_bits(value: f32) -> u32 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

/// stroke 폭(유한·비음수). 0이면 빈 mesh입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrokeWidth {
    bits: u32,
}

impl StrokeWidth {
    /// 유한·비음수 폭을 만듭니다.
    pub fn new(value: f32) -> Result<StrokeWidth, TessellationError> {
        if value.is_finite() && value >= 0.0 {
            Ok(StrokeWidth {
                bits: canonical_bits(value),
            })
        } else {
            Err(TessellationError::InvalidStroke {
                reason: "폭은 유한한 비음수여야 합니다",
            })
        }
    }

    /// 폭.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits)
    }
}

/// miter limit(유한, 1 이상). miter 길이 ÷ 폭의 상한입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MiterLimit {
    bits: u32,
}

impl MiterLimit {
    /// SVG 기본값 4.
    pub const DEFAULT: MiterLimit = MiterLimit {
        bits: 4.0f32.to_bits(),
    };

    /// 유한하고 1 이상인 limit을 만듭니다.
    pub fn new(value: f32) -> Result<MiterLimit, TessellationError> {
        if value.is_finite() && value >= 1.0 {
            Ok(MiterLimit {
                bits: value.to_bits(),
            })
        } else {
            Err(TessellationError::InvalidStroke {
                reason: "miter limit은 1 이상의 유한한 값이어야 합니다",
            })
        }
    }

    /// limit 값.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits)
    }
}

impl Default for MiterLimit {
    fn default() -> MiterLimit {
        MiterLimit::DEFAULT
    }
}

/// dash pattern. 간격은 on, off, on, … 순서이며 개수는 항상 짝수입니다.
///
/// 고정 크기 배열이라 `Copy`이고 cache key에 그대로 들어갑니다. 쓰지 않는 칸은 0입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DashPattern {
    intervals: [u32; MAX_DASH_INTERVALS],
    len: u8,
    offset: u32,
}

impl DashPattern {
    /// 간격 목록과 시작 offset으로 만듭니다.
    ///
    /// 간격은 유한한 비음수여야 하고, 홀수 개면 두 번 반복해 짝수로 만듭니다. 반복 뒤 개수가
    /// [`MAX_DASH_INTERVALS`]를 넘거나 목록이 비면 거부합니다. offset은 유한한 값이면 음수도
    /// 허용합니다(pattern을 거꾸로 진행).
    pub fn new(intervals: &[f32], offset: f32) -> Result<DashPattern, TessellationError> {
        if intervals.is_empty() {
            return Err(TessellationError::InvalidStroke {
                reason: "dash 간격 목록이 비어 있습니다",
            });
        }
        let len = if !intervals.len().is_multiple_of(2) {
            intervals.len() * 2
        } else {
            intervals.len()
        };
        if len > MAX_DASH_INTERVALS {
            return Err(TessellationError::InvalidStroke {
                reason: "dash 간격이 너무 많습니다",
            });
        }
        if intervals.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(TessellationError::InvalidStroke {
                reason: "dash 간격은 유한한 비음수여야 합니다",
            });
        }
        if !offset.is_finite() {
            return Err(TessellationError::InvalidStroke {
                reason: "dash offset은 유한해야 합니다",
            });
        }
        let mut bits = [0u32; MAX_DASH_INTERVALS];
        for (slot, value) in bits.iter_mut().zip(intervals.iter().cycle().take(len)) {
            *slot = canonical_bits(*value);
        }
        Ok(DashPattern {
            intervals: bits,
            // len ≤ MAX_DASH_INTERVALS(16)이므로 u8에 들어갑니다.
            len: len as u8,
            offset: canonical_bits(offset),
        })
    }

    /// 간격 개수(짝수).
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// 간격이 없으면 참입니다(생성 규칙상 항상 거짓).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `index`번째 간격.
    pub fn interval(&self, index: usize) -> Option<f32> {
        (index < self.len()).then(|| f32::from_bits(self.intervals[index]))
    }

    /// 시작 offset.
    pub fn offset(&self) -> f32 {
        f32::from_bits(self.offset)
    }

    /// 간격 합(f64).
    fn period(&self) -> f64 {
        self.intervals[..self.len()]
            .iter()
            .map(|bits| f64::from(f32::from_bits(*bits)))
            .sum()
    }

    /// 간격 합이 0이면(실선과 같음) 참입니다.
    pub fn is_solid(&self) -> bool {
        self.period() <= 0.0
    }
}

/// mesh에 영향을 주는 stroke style.
///
/// paint·opacity·transform은 mesh를 바꾸지 않으므로 여기에 없습니다(fill과 같은 규칙).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrokeStyle {
    width: StrokeWidth,
    join: StrokeJoin,
    miter_limit: MiterLimit,
    cap: StrokeCap,
    dash: Option<DashPattern>,
    tolerance: FlattenTolerance,
}

impl StrokeStyle {
    /// 폭 `width`, Miter join(limit 4), Butt cap, 실선, 기본 허용 오차.
    pub fn new(width: f32) -> Result<StrokeStyle, TessellationError> {
        Ok(StrokeStyle {
            width: StrokeWidth::new(width)?,
            join: StrokeJoin::Miter,
            miter_limit: MiterLimit::DEFAULT,
            cap: StrokeCap::Butt,
            dash: None,
            tolerance: FlattenTolerance::DEFAULT,
        })
    }

    /// scene stroke layer의 폭·cap으로 만듭니다. join·dash는 기본값입니다.
    ///
    /// `Inside`/`Outside` 정렬은 폭을 두 배로 한 중심 stroke입니다. 호출자는 이 mesh를 fill 영역
    /// 안(`Inside`)이나 밖(`Outside`)으로 clip해 그려야 합니다.
    pub fn from_layer(layer: &StrokeLayer) -> Result<StrokeStyle, TessellationError> {
        let width = match layer.align() {
            StrokeAlign::Center => layer.width(),
            StrokeAlign::Inside | StrokeAlign::Outside => layer.width() * 2.0,
        };
        Ok(StrokeStyle::new(width)?.with_cap(layer.cap()))
    }

    /// join을 바꿉니다.
    pub fn with_join(mut self, join: StrokeJoin) -> StrokeStyle {
        self.join = join;
        self
    }

    /// miter limit을 바꿉니다.
    pub fn with_miter_limit(mut self, limit: MiterLimit) -> StrokeStyle {
        self.miter_limit = limit;
        self
    }

    /// cap을 바꿉니다.
    pub fn with_cap(mut self, cap: StrokeCap) -> StrokeStyle {
        self.cap = cap;
        self
    }

    /// dash pattern을 바꿉니다(`None`은 실선).
    pub fn with_dash(mut self, dash: Option<DashPattern>) -> StrokeStyle {
        self.dash = dash;
        self
    }

    /// 평탄화 허용 오차를 바꿉니다.
    pub fn with_tolerance(mut self, tolerance: FlattenTolerance) -> StrokeStyle {
        self.tolerance = tolerance;
        self
    }

    /// 폭.
    pub fn width(&self) -> f32 {
        self.width.get()
    }

    /// join.
    pub fn join(&self) -> StrokeJoin {
        self.join
    }

    /// miter limit.
    pub fn miter_limit(&self) -> MiterLimit {
        self.miter_limit
    }

    /// cap.
    pub fn cap(&self) -> StrokeCap {
        self.cap
    }

    /// dash pattern.
    pub fn dash(&self) -> Option<DashPattern> {
        self.dash
    }

    /// 평탄화 허용 오차.
    pub fn tolerance(&self) -> FlattenTolerance {
        self.tolerance
    }

    /// mesh가 같은 style을 하나로 모은 정규형(cache key용).
    ///
    /// - 폭 0: mesh가 항상 비므로 나머지 값을 모두 기본값으로 둡니다.
    /// - Miter가 아닌 join: miter limit을 기본값으로 둡니다.
    /// - 간격 합이 0인 dash: 실선(`None`)으로 둡니다.
    pub fn canonical(&self) -> StrokeStyle {
        if self.width.bits == 0 {
            return StrokeStyle {
                width: self.width,
                join: StrokeJoin::Miter,
                miter_limit: MiterLimit::DEFAULT,
                cap: StrokeCap::Butt,
                dash: None,
                tolerance: FlattenTolerance::DEFAULT,
            };
        }
        let mut out = *self;
        if out.join != StrokeJoin::Miter {
            out.miter_limit = MiterLimit::DEFAULT;
        }
        if out.dash.is_some_and(|dash| dash.is_solid()) {
            out.dash = None;
        }
        out
    }

    /// mesh 정점이 path 점에서 떨어질 수 있는 최대 거리(culling bounds 확장용, 보수적).
    ///
    /// 반폭 × max(1, Square cap의 √2, Miter join의 miter limit). 곡선 평탄화는 control point 볼록
    /// 껍질 안에 머물므로 control bounds를 이 값만큼 넓히면 mesh를 덮습니다.
    pub fn outset(&self) -> f32 {
        let half = self.width.get() / 2.0;
        let mut factor = 1.0f32;
        if self.cap == StrokeCap::Square {
            factor = factor.max(std::f32::consts::SQRT_2);
        }
        if self.join == StrokeJoin::Miter {
            factor = factor.max(self.miter_limit.get());
        }
        half * factor
    }
}

/// stroke tessellation interface.
///
/// 구현은 같은 입력에 같은 mesh를 반환해야 하며(결정론), worker thread에서 호출될 수 있도록
/// `Send + Sync`입니다. 입력이 잘못되어도 panic하지 않고 오류를 반환해야 합니다.
pub trait StrokeTessellator: Send + Sync {
    /// `path`를 `style`로 그리는 삼각형 mesh를 만듭니다.
    fn tessellate_stroke(
        &self,
        path: &PathGeometry,
        style: &StrokeStyle,
    ) -> Result<FillMesh, TessellationError>;
}

/// 모듈 문서 규칙을 구현한 기본 stroke tessellator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReferenceStrokeTessellator;

impl StrokeTessellator for ReferenceStrokeTessellator {
    fn tessellate_stroke(
        &self,
        path: &PathGeometry,
        style: &StrokeStyle,
    ) -> Result<FillMesh, TessellationError> {
        tessellate_stroke(path, style)
    }
}

/// `path`를 `style`로 그리는 삼각형 mesh(모듈 문서 규칙).
pub fn tessellate_stroke(
    path: &PathGeometry,
    style: &StrokeStyle,
) -> Result<FillMesh, TessellationError> {
    let half = f64::from(style.width()) / 2.0;
    if half <= 0.0 {
        return Ok(FillMesh::empty());
    }
    let tolerance = f64::from(style.tolerance().get());
    let resolved = Resolved {
        half,
        join: style.join(),
        miter_limit: f64::from(style.miter_limit().get()),
        cap: style.cap(),
        arc_tolerance: tolerance / half,
    };
    let lines = flatten(path, tolerance, Some(half))?;
    let mut mesh = MeshBuilder::new(MAX_STROKE_VERTICES);
    let dash = style
        .dash()
        .filter(|dash| !dash.is_solid())
        .map(ResolvedDash::new);
    match dash {
        None => {
            for line in &lines {
                stroke_polyline(&mut mesh, &line.points, line.closed, AXIS_X, &resolved)?;
            }
        }
        Some(dash) => {
            let mut budget = DashBudget::default();
            for line in &lines {
                for run in dash_polyline(line, &dash, &mut budget)? {
                    stroke_polyline(&mut mesh, &run.points, run.closed, run.dir, &resolved)?;
                }
            }
        }
    }
    mesh.finish()
}

// --- 내부 기하 ---

/// f64 점·벡터.
#[derive(Debug, Clone, Copy, PartialEq)]
struct P {
    x: f64,
    y: f64,
}

const AXIS_X: P = P { x: 1.0, y: 0.0 };

impl P {
    const fn new(x: f64, y: f64) -> P {
        P { x, y }
    }

    fn from_vec2(v: Vec2) -> P {
        P::new(f64::from(v.x()), f64::from(v.y()))
    }

    fn add(self, o: P) -> P {
        P::new(self.x + o.x, self.y + o.y)
    }

    fn sub(self, o: P) -> P {
        P::new(self.x - o.x, self.y - o.y)
    }

    fn scale(self, k: f64) -> P {
        P::new(self.x * k, self.y * k)
    }

    fn dot(self, o: P) -> f64 {
        self.x * o.x + self.y * o.y
    }

    fn cross(self, o: P) -> f64 {
        self.x * o.y - self.y * o.x
    }

    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }

    /// 왼쪽 법선 `(-y, x)`.
    fn left(self) -> P {
        P::new(-self.y, self.x)
    }

    /// 단위 벡터. 길이 0이면 `None`.
    fn unit(self) -> Option<P> {
        let len = self.len();
        (len > 0.0 && len.is_finite()).then(|| self.scale(1.0 / len))
    }
}

/// 평탄화한 subpath.
#[derive(Debug, Clone, PartialEq)]
struct Polyline {
    /// 연속 중복 없는 점(닫힌 경우 끝점 = 시작점 중복도 제거).
    points: Vec<P>,
    closed: bool,
}

/// 평탄화 점 수 계수기.
struct PointBudget {
    used: usize,
}

impl PointBudget {
    fn take(&mut self) -> Result<(), TessellationError> {
        self.used += 1;
        if self.used > MAX_FLATTENED_POINTS {
            Err(TessellationError::TooComplex {
                limit: MAX_FLATTENED_POINTS,
            })
        } else {
            Ok(())
        }
    }
}

/// subpath 작성 중 상태.
struct SubpathBuilder {
    points: Vec<P>,
    drawn: bool,
}

impl SubpathBuilder {
    fn start(at: P) -> SubpathBuilder {
        SubpathBuilder {
            points: vec![at],
            drawn: false,
        }
    }

    fn last(&self) -> P {
        // start가 항상 점 하나를 넣으므로 비지 않습니다.
        self.points.last().copied().unwrap_or(P::new(0.0, 0.0))
    }

    fn push(&mut self, point: P, budget: &mut PointBudget) -> Result<(), TessellationError> {
        self.drawn = true;
        if self.points.last() != Some(&point) {
            budget.take()?;
            self.points.push(point);
        }
        Ok(())
    }

    fn finish(mut self, closed: bool, out: &mut Vec<Polyline>) {
        if !self.drawn {
            // MoveTo만 있는 subpath는 그리지 않습니다.
            return;
        }
        if closed && self.points.len() > 1 && self.points.first() == self.points.last() {
            self.points.pop();
        }
        out.push(Polyline {
            points: self.points,
            closed,
        });
    }
}

/// cubic 평탄화 segment 수(곡선 chord 오차 기준).
fn cubic_segments(p0: P, c1: P, c2: P, p3: P, tolerance: f64) -> u32 {
    let d1 = p0.sub(c1.scale(2.0)).add(c2).len();
    let d2 = c1.sub(c2.scale(2.0)).add(p3).len();
    let d = d1.max(d2);
    if d <= 0.0 {
        return 1;
    }
    let n = (0.75 * d / tolerance).sqrt().ceil();
    if n.is_nan() || n < 1.0 {
        1
    } else if n >= f64::from(MAX_CUBIC_SEGMENTS) {
        MAX_CUBIC_SEGMENTS
    } else {
        n as u32
    }
}

/// 정확한 시작·끝 접선(단위). 퇴화한 control point는 건너뜁니다.
fn cubic_end_tangents(p0: P, c1: P, c2: P, p3: P) -> (Option<P>, Option<P>) {
    let start = [c1.sub(p0), c2.sub(p0), p3.sub(p0)]
        .into_iter()
        .find_map(P::unit);
    let end = [p3.sub(c2), p3.sub(c1), p3.sub(p0)]
        .into_iter()
        .find_map(P::unit);
    (start, end)
}

/// 정확한 접선 `tangent`와 chord 방향 `chord`(단위) 사이 각의 sin. 반대 방향이면 1.
fn tangent_error(tangent: Option<P>, chord: Option<P>) -> f64 {
    match (tangent, chord) {
        (Some(t), Some(c)) if t.dot(c) > 0.0 => t.cross(c).abs(),
        (Some(_), Some(_)) => 1.0,
        _ => 0.0,
    }
}

/// 이웃 chord 방향(단위) 사이 꺾임 반각의 sin = `sqrt((1 − cos φ) / 2)`.
fn half_turn_sin(d0: P, d1: P) -> f64 {
    ((1.0 - d0.dot(d1)).max(0.0) / 2.0).sqrt()
}

/// `n` 분할 sample 점(시작점 제외, 끝점은 정확히 `p3`).
fn cubic_samples(p0: P, c1: P, c2: P, p3: P, n: u32) -> Vec<P> {
    let mut out = Vec::with_capacity(n as usize);
    for i in 1..n {
        let t = f64::from(i) / f64::from(n);
        out.push(cubic_point(p0, c1, c2, p3, t));
    }
    out.push(p3);
    out
}

/// 분할 sample의 최악 offset 각 오차(sin): 양 끝 접선-chord 각과 이웃 chord 꺾임 반각의 최대.
fn worst_offset_error(p0: P, samples: &[P], start: Option<P>, end: Option<P>) -> f64 {
    let mut previous = p0;
    let mut first: Option<P> = None;
    let mut last: Option<P> = None;
    let mut worst = 0.0f64;
    for point in samples {
        if let Some(dir) = point.sub(previous).unit() {
            if let Some(prev) = last {
                worst = worst.max(half_turn_sin(prev, dir));
            }
            first = first.or(Some(dir));
            last = Some(dir);
        }
        previous = *point;
    }
    worst
        .max(tangent_error(start, first))
        .max(tangent_error(end, last))
}

/// 분할 보정 최대 반복 수.
const MAX_REFINE_PASSES: u32 = 4;

/// stroke offset 오차까지 고려한 cubic 분할 sample(모듈 문서 "곡선").
///
/// 각 `α`에 대해 `sin α ≥ 2α/π`(α ≤ π/2)이므로 분할을 `ceil(반폭 · (π/2) · sin α / tolerance)`배
/// 늘리면 새 각의 sin이 대략 허용 오차 비율 이하가 됩니다. 균일 매개변수 분할은 각을 정확히 비례로
/// 줄이지 않으므로 [`MAX_REFINE_PASSES`]번까지 다시 확인합니다.
fn stroke_cubic_samples(p0: P, c1: P, c2: P, p3: P, tolerance: f64, half: Option<f64>) -> Vec<P> {
    let mut n = cubic_segments(p0, c1, c2, p3, tolerance);
    let mut samples = cubic_samples(p0, c1, c2, p3, n);
    let Some(half) = half else {
        return samples;
    };
    let (start, end) = cubic_end_tangents(p0, c1, c2, p3);
    for _ in 0..MAX_REFINE_PASSES {
        let worst = worst_offset_error(p0, &samples, start, end);
        if half * worst <= tolerance || n >= MAX_CUBIC_SEGMENTS {
            break;
        }
        let factor = (half * std::f64::consts::FRAC_PI_2 * worst / tolerance)
            .ceil()
            .max(2.0);
        n = (f64::from(n) * factor).min(f64::from(MAX_CUBIC_SEGMENTS)) as u32;
        samples = cubic_samples(p0, c1, c2, p3, n);
    }
    samples
}

fn cubic_point(p0: P, c1: P, c2: P, p3: P, t: f64) -> P {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let c = 3.0 * mt * t * t;
    let d = t * t * t;
    P::new(
        a * p0.x + b * c1.x + c * c2.x + d * p3.x,
        a * p0.y + b * c1.y + c * c2.y + d * p3.y,
    )
}

/// path를 subpath polyline 목록으로 평탄화합니다. `half`는 stroke 반폭(offset 오차 보정용)입니다.
fn flatten(
    path: &PathGeometry,
    tolerance: f64,
    half: Option<f64>,
) -> Result<Vec<Polyline>, TessellationError> {
    let mut out = Vec::new();
    let mut budget = PointBudget { used: 0 };
    let mut current: Option<SubpathBuilder> = None;
    for command in path.commands() {
        match *command {
            PathCommand::MoveTo(v) => {
                if let Some(sub) = current.take() {
                    sub.finish(false, &mut out);
                }
                budget.take()?;
                current = Some(SubpathBuilder::start(P::from_vec2(v)));
            }
            PathCommand::LineTo(v) => {
                if let Some(sub) = current.as_mut() {
                    sub.push(P::from_vec2(v), &mut budget)?;
                }
            }
            PathCommand::CubicTo { c1, c2, to } => {
                if let Some(sub) = current.as_mut() {
                    let (p0, c1, c2, p3) = (
                        sub.last(),
                        P::from_vec2(c1),
                        P::from_vec2(c2),
                        P::from_vec2(to),
                    );
                    for point in stroke_cubic_samples(p0, c1, c2, p3, tolerance, half) {
                        sub.push(point, &mut budget)?;
                    }
                }
            }
            PathCommand::Close => {
                if let Some(mut sub) = current.take() {
                    // Close도 시작점으로 가는 선을 그리므로 "M p Z"는 길이 0 subpath(점)입니다.
                    sub.drawn = true;
                    sub.finish(true, &mut out);
                }
            }
        }
    }
    if let Some(sub) = current.take() {
        sub.finish(false, &mut out);
    }
    Ok(out)
}

/// 해석한 style.
struct Resolved {
    half: f64,
    join: StrokeJoin,
    miter_limit: f64,
    cap: StrokeCap,
    /// 반폭 대비 원호 chord 높이 허용 오차.
    arc_tolerance: f64,
}

/// 정점 수 상한이 있는 mesh 작성기.
struct MeshBuilder {
    vertices: Vec<Vec2>,
    indices: Vec<u32>,
    limit: usize,
}

impl MeshBuilder {
    fn new(limit: usize) -> MeshBuilder {
        MeshBuilder {
            vertices: Vec::new(),
            indices: Vec::new(),
            limit,
        }
    }

    fn vertex(&mut self, p: P) -> Result<u32, TessellationError> {
        if self.vertices.len() >= self.limit {
            return Err(TessellationError::TooComplex { limit: self.limit });
        }
        let v = Vec2::new(p.x as f32, p.y as f32).map_err(|_| TessellationError::Failed {
            reason: "stroke 정점 좌표가 f32 범위를 넘습니다",
        })?;
        let index = u32::try_from(self.vertices.len())
            .map_err(|_| TessellationError::TooComplex { limit: self.limit })?;
        self.vertices.push(v);
        Ok(index)
    }

    /// 면적이 0이 아닌 삼각형을 추가합니다.
    fn triangle(&mut self, a: P, b: P, c: P) -> Result<(), TessellationError> {
        if b.sub(a).cross(c.sub(a)) == 0.0 {
            return Ok(());
        }
        let ia = self.vertex(a)?;
        let ib = self.vertex(b)?;
        let ic = self.vertex(c)?;
        self.indices.extend_from_slice(&[ia, ib, ic]);
        Ok(())
    }

    /// 볼록 사각형 `a b c d`(둘레 순서).
    fn quad(&mut self, a: P, b: P, c: P, d: P) -> Result<(), TessellationError> {
        let ia = self.vertex(a)?;
        let ib = self.vertex(b)?;
        let ic = self.vertex(c)?;
        let id = self.vertex(d)?;
        self.indices.extend_from_slice(&[ia, ib, ic, ia, ic, id]);
        Ok(())
    }

    /// `center`를 중심으로 단위 벡터 `units` 순서의 원호 점을 잇는 fan.
    fn fan(&mut self, center: P, radius: f64, units: &[P]) -> Result<(), TessellationError> {
        for pair in units.windows(2) {
            self.triangle(
                center,
                center.add(pair[0].scale(radius)),
                center.add(pair[1].scale(radius)),
            )?;
        }
        Ok(())
    }

    fn finish(self) -> Result<FillMesh, TessellationError> {
        FillMesh::new(self.vertices, self.indices)
    }
}

/// 단위 벡터 `a`에서 `c`까지(180° 이하) 원호를 이등분해 `out`에 `a`, 중간 점들, `c` 순서로
/// 넣습니다. 두 벡터가 정반대면 `hint` 방향을 지나는 반원입니다.
fn arc_units(a: P, c: P, hint: P, relative_tolerance: f64, out: &mut Vec<P>) {
    out.push(a);
    let mut stack: Vec<(P, P, u32)> = Vec::new();
    if a.add(c).len() <= 1e-9 {
        // 정반대: 이등분이 정의되지 않으므로 hint를 중간점으로 씁니다.
        let mid = hint.unit().unwrap_or(a.left());
        stack.push((mid, c, 1));
        stack.push((a, mid, 1));
    } else {
        stack.push((a, c, 0));
    }
    while let Some((u, v, depth)) = stack.pop() {
        let sum = u.add(v);
        let len = sum.len();
        // chord 높이 / 반지름 = 1 - cos(φ/2) = 1 - |u + v| / 2.
        let sagitta = 1.0 - len / 2.0;
        if depth < MAX_ARC_DEPTH && sagitta > relative_tolerance && len > 0.0 {
            let mid = sum.scale(1.0 / len);
            stack.push((mid, v, depth + 1));
            stack.push((u, mid, depth + 1));
        } else {
            out.push(v);
        }
    }
}

/// segment 하나의 사각형.
fn segment(mesh: &mut MeshBuilder, a: P, b: P, dir: P, half: f64) -> Result<(), TessellationError> {
    let n = dir.left().scale(half);
    mesh.quad(a.add(n), b.add(n), b.sub(n), a.sub(n))
}

/// 꺾인 점 `p`(들어오는 방향 `d0`, 나가는 방향 `d1`)의 join.
fn join(
    mesh: &mut MeshBuilder,
    p: P,
    d0: P,
    d1: P,
    style: &Resolved,
) -> Result<(), TessellationError> {
    let cross = d0.cross(d1);
    let dot = d0.dot(d1);
    if cross.abs() <= 1e-12 && dot > 0.0 {
        // 일직선 연결: 두 segment 사각형이 맞닿습니다.
        return Ok(());
    }
    // 왼쪽으로 돌면(cross > 0) 바깥은 오른쪽입니다.
    let side = if cross > 0.0 { -1.0 } else { 1.0 };
    let a = d0.left().scale(side);
    let c = d1.left().scale(side);
    let half = style.half;
    let pa = p.add(a.scale(half));
    let pc = p.add(c.scale(half));
    match style.join {
        StrokeJoin::Bevel => mesh.triangle(p, pa, pc),
        StrokeJoin::Miter => {
            let sum = a.add(c);
            let sum_len = sum.len();
            // miter 비율 = 1 / sin(θ/2) = 2 / |a + c|.
            if sum_len > 1e-12 && 2.0 / sum_len <= style.miter_limit {
                let tip = p.add(sum.scale(half * 2.0 / (sum_len * sum_len)));
                mesh.triangle(p, pa, tip)?;
                mesh.triangle(p, tip, pc)
            } else {
                mesh.triangle(p, pa, pc)
            }
        }
        StrokeJoin::Round => {
            let mut units = Vec::new();
            arc_units(a, c, d0, style.arc_tolerance, &mut units);
            mesh.fan(p, half, &units)
        }
    }
}

/// 끝점 `p`에서 바깥 방향 `out`(단위)으로 붙는 cap.
fn cap(mesh: &mut MeshBuilder, p: P, out: P, style: &Resolved) -> Result<(), TessellationError> {
    let half = style.half;
    let n = out.left();
    match style.cap {
        StrokeCap::Butt => Ok(()),
        StrokeCap::Square => {
            let ext = out.scale(half);
            let side = n.scale(half);
            mesh.quad(
                p.add(side),
                p.add(side).add(ext),
                p.sub(side).add(ext),
                p.sub(side),
            )
        }
        StrokeCap::Round => {
            let mut units = Vec::new();
            arc_units(n, n.scale(-1.0), out, style.arc_tolerance, &mut units);
            mesh.fan(p, half, &units)
        }
    }
}

/// 길이 0 subpath의 점 cap. `dir`은 Square 방향입니다(없으면 축 정렬).
fn dot(mesh: &mut MeshBuilder, p: P, dir: P, style: &Resolved) -> Result<(), TessellationError> {
    let half = style.half;
    let h = dir.unit().unwrap_or(AXIS_X);
    let n = h.left();
    match style.cap {
        StrokeCap::Butt => Ok(()),
        StrokeCap::Square => {
            let (hh, nn) = (h.scale(half), n.scale(half));
            mesh.quad(
                p.add(hh).add(nn),
                p.sub(hh).add(nn),
                p.sub(hh).sub(nn),
                p.add(hh).sub(nn),
            )
        }
        StrokeCap::Round => {
            let quarters = [h, n, h.scale(-1.0), n.scale(-1.0), h];
            let mut units = Vec::new();
            for pair in quarters.windows(2) {
                let mut part = Vec::new();
                arc_units(pair[0], pair[1], pair[1], style.arc_tolerance, &mut part);
                if !units.is_empty() {
                    // 이전 원호의 끝점과 중복되는 시작점을 뺍니다.
                    part.remove(0);
                }
                units.extend(part);
            }
            mesh.fan(p, half, &units)
        }
    }
}

/// polyline 하나를 stroke합니다. `dot_dir`은 길이 0일 때 점 cap 방향입니다.
fn stroke_polyline(
    mesh: &mut MeshBuilder,
    points: &[P],
    closed: bool,
    dot_dir: P,
    style: &Resolved,
) -> Result<(), TessellationError> {
    let count = points.len();
    if count == 0 {
        return Ok(());
    }
    if count == 1 {
        return dot(mesh, points[0], dot_dir, style);
    }
    let segment_count = if closed { count } else { count - 1 };
    let mut dirs = Vec::with_capacity(segment_count);
    for i in 0..segment_count {
        let a = points[i];
        let b = points[(i + 1) % count];
        // 연속 중복을 제거했으므로 길이는 0이 아닙니다. 방어적으로 0 길이 segment는 건너뜁니다.
        let Some(dir) = b.sub(a).unit() else {
            continue;
        };
        segment(mesh, a, b, dir, style.half)?;
        dirs.push((i, dir));
    }
    if dirs.is_empty() {
        return dot(mesh, points[0], dot_dir, style);
    }
    // join: 연속한 두 segment가 만나는 점.
    for pair in dirs.windows(2) {
        let (_, d0) = pair[0];
        let (index, d1) = pair[1];
        join(mesh, points[index], d0, d1, style)?;
    }
    let (first_index, first_dir) = dirs[0];
    let (_, last_dir) = dirs[dirs.len() - 1];
    if closed {
        join(mesh, points[first_index], last_dir, first_dir, style)?;
    } else {
        cap(mesh, points[0], first_dir.scale(-1.0), style)?;
        cap(mesh, points[count - 1], last_dir, style)?;
    }
    Ok(())
}

/// dash 계산용 해석 값.
struct ResolvedDash {
    intervals: Vec<f64>,
    period: f64,
    offset: f64,
}

impl ResolvedDash {
    fn new(dash: DashPattern) -> ResolvedDash {
        let intervals: Vec<f64> = (0..dash.len())
            .filter_map(|i| dash.interval(i))
            .map(f64::from)
            .collect();
        let period = intervals.iter().sum();
        ResolvedDash {
            intervals,
            period,
            offset: f64::from(dash.offset()),
        }
    }

    /// offset을 반영한 시작 상태(간격 index, 그 간격의 남은 길이).
    fn start(&self) -> (usize, f64) {
        let count = self.intervals.len();
        let mut phase = self.offset - self.period * (self.offset / self.period).floor();
        if !(0.0..self.period).contains(&phase) {
            phase = 0.0;
        }
        let mut index = 0;
        let mut remaining = self.intervals[0];
        // phase < period이므로 간격을 한 바퀴 넘게 돌지 않습니다(반올림 대비 두 바퀴 상한).
        for _ in 0..count * 2 {
            if phase <= 0.0 {
                break;
            }
            if phase >= remaining {
                phase -= remaining;
                index = (index + 1) % count;
                remaining = self.intervals[index];
            } else {
                remaining -= phase;
                phase = 0.0;
            }
        }
        (index, remaining)
    }
}

/// dash 하나(열린 polyline) 또는 끝까지 on인 닫힌 subpath.
#[derive(Debug, Clone, PartialEq)]
struct DashRun {
    points: Vec<P>,
    closed: bool,
    /// 길이 0 dash의 방향.
    dir: P,
}

impl DashRun {
    fn start(at: P, dir: P) -> DashRun {
        DashRun {
            points: vec![at],
            closed: false,
            dir,
        }
    }

    fn push(&mut self, point: P) {
        if self.points.last() != Some(&point) {
            self.points.push(point);
        }
    }
}

/// path 전체의 dash 수·반복 수 계수기.
#[derive(Default)]
struct DashBudget {
    dashes: usize,
    steps: usize,
}

impl DashBudget {
    fn dash(&mut self) -> Result<(), TessellationError> {
        self.dashes += 1;
        if self.dashes > MAX_DASHES {
            Err(TessellationError::TooComplex { limit: MAX_DASHES })
        } else {
            Ok(())
        }
    }

    fn step(&mut self) -> Result<(), TessellationError> {
        self.steps += 1;
        if self.steps > MAX_DASH_STEPS {
            Err(TessellationError::TooComplex {
                limit: MAX_DASH_STEPS,
            })
        } else {
            Ok(())
        }
    }
}

/// polyline을 dash run 목록으로 나눕니다.
fn dash_polyline(
    line: &Polyline,
    dash: &ResolvedDash,
    budget: &mut DashBudget,
) -> Result<Vec<DashRun>, TessellationError> {
    let points = &line.points;
    let count = points.len();
    let mut runs = Vec::new();
    let is_on = |index: usize| index.is_multiple_of(2);
    let (mut index, mut remaining) = dash.start();
    if count == 1 {
        // 길이 0 subpath: 시작 상태가 on이면 점 cap.
        if is_on(index) {
            budget.dash()?;
            runs.push(DashRun::start(points[0], AXIS_X));
        }
        return Ok(runs);
    }
    let segment_count = if line.closed { count } else { count - 1 };
    let total: f64 = (0..segment_count)
        .map(|i| points[(i + 1) % count].sub(points[i]).len())
        .sum();
    if total / dash.period > MAX_DASHES as f64 {
        return Err(TessellationError::TooComplex { limit: MAX_DASHES });
    }
    let first_dir = points[1].sub(points[0]).unit().unwrap_or(AXIS_X);
    let starts_on = is_on(index);
    let mut current = starts_on.then(|| DashRun::start(points[0], first_dir));
    for i in 0..segment_count {
        let a = points[i];
        let b = points[(i + 1) % count];
        let length = b.sub(a).len();
        let Some(dir) = b.sub(a).unit() else {
            continue;
        };
        let mut pos = 0.0f64;
        loop {
            budget.step()?;
            if remaining <= 0.0 {
                // 간격 소진(길이 0 간격 포함): on이 끝나면 dash를 닫고 다음 간격으로.
                if is_on(index) {
                    if let Some(run) = current.take() {
                        budget.dash()?;
                        runs.push(run);
                    }
                }
                index = (index + 1) % dash.intervals.len();
                remaining = dash.intervals[index];
                if is_on(index) {
                    if pos >= length && i + 1 == segment_count {
                        // subpath 끝점에서 시작하는 dash는 그릴 길이가 없습니다.
                    } else {
                        let at = if pos >= length {
                            b
                        } else {
                            a.add(dir.scale(pos))
                        };
                        current = Some(DashRun::start(at, dir));
                    }
                }
                continue;
            }
            let left = length - pos;
            if left <= 0.0 {
                break;
            }
            let step = remaining.min(left);
            let at = if step >= left {
                pos = length;
                b
            } else {
                pos += step;
                a.add(dir.scale(pos))
            };
            remaining -= step;
            if let Some(run) = current.as_mut() {
                run.push(at);
            }
        }
    }
    let ends_on = current.is_some();
    if let Some(run) = current.take() {
        budget.dash()?;
        runs.push(run);
    }
    if line.closed && starts_on && ends_on {
        if runs.len() == 1 {
            // 처음부터 끝까지 on: 닫힌 subpath 그대로(cap 없음).
            if let Some(run) = runs.first_mut() {
                run.points.clone_from(points);
                run.closed = true;
            }
        } else if let Some(last) = runs.pop() {
            // 끝 dash와 첫 dash를 시작점에서 이어 join을 둡니다.
            if let Some(first) = runs.first_mut() {
                let mut merged = last.points;
                for point in first.points.iter().skip(1) {
                    if merged.last() != Some(point) {
                        merged.push(*point);
                    }
                }
                first.points = merged;
                first.dir = last.dir;
            }
        }
    }
    Ok(runs)
}

#[cfg(test)]
mod tests;
