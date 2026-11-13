//! Path command (VI.6).
//!
//! 저장 path command는 **MoveTo, LineTo, CubicTo, Close 네 가지뿐**입니다.
//! Quadratic 곡선은 import 단계에서 [`quadratic_to_cubic`]으로 정확히 같은
//! 곡선의 Cubic으로 정규화합니다 — 문서 모델에는 Quadratic이 존재하지 않습니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{NonFiniteError, Vec2};

/// Path 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// 좌표가 유한하지 않습니다.
    NonFinite,
    /// 알 수 없는 command tag.
    UnknownCommand {
        /// 발견된 tag.
        tag: u8,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// subpath가 MoveTo 없이 시작합니다(VI.6.2). index는 문제 command 위치입니다.
    MissingMoveTo {
        /// command index.
        index: usize,
    },
    /// 알 수 없는 winding rule tag.
    UnknownWinding {
        /// 발견된 tag.
        tag: u8,
    },
    /// command와 id 목록의 모양이 맞지 않습니다(VI.6.3).
    IdShapeMismatch {
        /// command index.
        index: usize,
    },
    /// point·handle id가 중복됩니다(VI.6.3).
    DuplicateId {
        /// 중복 id.
        id: StableId,
    },
    /// 삽입 위치가 범위를 벗어납니다.
    IndexOutOfRange {
        /// 요청 index.
        index: usize,
    },
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::NonFinite => f.write_str("path 좌표가 유한하지 않습니다"),
            PathError::UnknownCommand { tag } => write!(f, "알 수 없는 path command tag: {tag}"),
            PathError::Malformed { reason } => write!(f, "Path binary 형식 위반: {reason}"),
            PathError::MissingMoveTo { index } => {
                write!(f, "command {index}: subpath는 MoveTo로 시작해야 합니다")
            }
            PathError::UnknownWinding { tag } => write!(f, "알 수 없는 winding rule tag: {tag}"),
            PathError::IdShapeMismatch { index } => {
                write!(f, "command {index}의 point/handle id 구성이 맞지 않습니다")
            }
            PathError::DuplicateId { id } => write!(f, "path point/handle id 중복: {id}"),
            PathError::IndexOutOfRange { index } => write!(f, "삽입 위치가 범위 밖입니다: {index}"),
        }
    }
}

impl std::error::Error for PathError {}

impl From<NonFiniteError> for PathError {
    fn from(_: NonFiniteError) -> PathError {
        PathError::NonFinite
    }
}

/// 저장 path command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathCommand {
    /// 새 subpath 시작점.
    MoveTo(Vec2),
    /// 직선.
    LineTo(Vec2),
    /// 3차 Bézier.
    CubicTo {
        /// 첫 control point.
        c1: Vec2,
        /// 둘째 control point.
        c2: Vec2,
        /// 끝점.
        to: Vec2,
    },
    /// 현재 subpath를 시작점으로 닫습니다.
    Close,
}

const TAG_MOVE: u8 = 1;
const TAG_LINE: u8 = 2;
const TAG_CUBIC: u8 = 3;
const TAG_CLOSE: u8 = 4;

/// Quadratic Bézier(`from`, `control`, `to`)를 정확히 같은 곡선의 Cubic으로
/// 변환합니다(import 정규화). `c1 = from + 2/3(control − from)`,
/// `c2 = to + 2/3(control − to)`.
pub fn quadratic_to_cubic(from: Vec2, control: Vec2, to: Vec2) -> Result<PathCommand, PathError> {
    let two_thirds = 2.0 / 3.0;
    let c1 = Vec2::new(
        from.x() + two_thirds * (control.x() - from.x()),
        from.y() + two_thirds * (control.y() - from.y()),
    )?;
    let c2 = Vec2::new(
        to.x() + two_thirds * (control.x() - to.x()),
        to.y() + two_thirds * (control.y() - to.y()),
    )?;
    Ok(PathCommand::CubicTo { c1, c2, to })
}

fn put_point(buf: &mut Vec<u8>, p: Vec2) {
    buf.extend_from_slice(&p.x().to_le_bytes());
    buf.extend_from_slice(&p.y().to_le_bytes());
}

/// command 목록을 canonical bytes로 인코딩합니다: `u32` 개수 + (tag + 좌표)*.
pub fn encode_commands(commands: &[PathCommand]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(commands.len() as u32).to_le_bytes());
    for command in commands {
        match *command {
            PathCommand::MoveTo(p) => {
                buf.push(TAG_MOVE);
                put_point(&mut buf, p);
            }
            PathCommand::LineTo(p) => {
                buf.push(TAG_LINE);
                put_point(&mut buf, p);
            }
            PathCommand::CubicTo { c1, c2, to } => {
                buf.push(TAG_CUBIC);
                put_point(&mut buf, c1);
                put_point(&mut buf, c2);
                put_point(&mut buf, to);
            }
            PathCommand::Close => buf.push(TAG_CLOSE),
        }
    }
    buf
}

struct Reader<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], PathError> {
        if self.input.len() - self.pos < n {
            return Err(PathError::Malformed {
                reason: "입력 부족",
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_point(&mut self) -> Result<Vec2, PathError> {
        let x = f32::from_le_bytes(self.take(4)?.try_into().expect("4바이트"));
        let y = f32::from_le_bytes(self.take(4)?.try_into().expect("4바이트"));
        Ok(Vec2::new(x, y)?)
    }
}

/// canonical bytes에서 command 목록을 복원합니다.
pub fn decode_commands(bytes: &[u8]) -> Result<Vec<PathCommand>, PathError> {
    let mut reader = Reader {
        input: bytes,
        pos: 0,
    };
    let count = u32::from_le_bytes(reader.take(4)?.try_into().expect("4바이트")) as usize;
    let mut commands = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let tag = reader.take(1)?[0];
        commands.push(match tag {
            TAG_MOVE => PathCommand::MoveTo(reader.take_point()?),
            TAG_LINE => PathCommand::LineTo(reader.take_point()?),
            TAG_CUBIC => PathCommand::CubicTo {
                c1: reader.take_point()?,
                c2: reader.take_point()?,
                to: reader.take_point()?,
            },
            TAG_CLOSE => PathCommand::Close,
            tag => return Err(PathError::UnknownCommand { tag }),
        });
    }
    if reader.pos != bytes.len() {
        return Err(PathError::Malformed {
            reason: "trailing bytes",
        });
    }
    Ok(commands)
}

/// 채우기 winding rule (VI.6.2). tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum WindingRule {
    /// non-zero(기본).
    #[default]
    NonZero = 0,
    /// even-odd.
    EvenOdd = 1,
}

/// 하나의 subpath: MoveTo로 시작하는 command 구간과 닫힘 여부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subpath {
    /// MoveTo를 포함한 command들(Close 제외).
    pub commands: Vec<PathCommand>,
    /// Close로 끝났는지.
    pub closed: bool,
}

/// 검증된 path 데이터 (VI.6.2).
///
/// 불변식: 비어 있지 않다면 첫 command는 MoveTo이고, Close 다음에 이어지는
/// command도 MoveTo여야 합니다(암묵적 subpath 재시작을 허용하지 않는 canonical
/// 형식). 빈 path는 유효합니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathData {
    winding: WindingRule,
    commands: Vec<PathCommand>,
}

/// command 목록이 subpath 규칙을 지키는지 검사합니다.
pub fn validate_commands(commands: &[PathCommand]) -> Result<(), PathError> {
    let mut in_subpath = false;
    for (index, command) in commands.iter().enumerate() {
        match command {
            PathCommand::MoveTo(_) => in_subpath = true,
            PathCommand::LineTo(_) | PathCommand::CubicTo { .. } => {
                if !in_subpath {
                    return Err(PathError::MissingMoveTo { index });
                }
            }
            PathCommand::Close => {
                if !in_subpath {
                    return Err(PathError::MissingMoveTo { index });
                }
                in_subpath = false;
            }
        }
    }
    Ok(())
}

impl PathData {
    /// 검증 후 path를 만듭니다.
    pub fn new(winding: WindingRule, commands: Vec<PathCommand>) -> Result<PathData, PathError> {
        validate_commands(&commands)?;
        Ok(PathData { winding, commands })
    }

    /// winding rule.
    pub fn winding(&self) -> WindingRule {
        self.winding
    }

    /// command들.
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// subpath로 나눕니다.
    pub fn subpaths(&self) -> Vec<Subpath> {
        let mut out: Vec<Subpath> = Vec::new();
        for command in &self.commands {
            match command {
                PathCommand::MoveTo(_) => out.push(Subpath {
                    commands: vec![*command],
                    closed: false,
                }),
                PathCommand::Close => {
                    if let Some(last) = out.last_mut() {
                        last.closed = true;
                    }
                }
                _ => {
                    if let Some(last) = out.last_mut() {
                        last.commands.push(*command);
                    }
                }
            }
        }
        out
    }

    /// canonical bytes: winding tag + command 인코딩.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![self.winding as u8];
        buf.extend_from_slice(&encode_commands(&self.commands));
        buf
    }

    /// bytes에서 복원하고 subpath 규칙을 검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<PathData, PathError> {
        let (&tag, rest) = bytes.split_first().ok_or(PathError::Malformed {
            reason: "winding 없음",
        })?;
        let winding = match tag {
            0 => WindingRule::NonZero,
            1 => WindingRule::EvenOdd,
            tag => return Err(PathError::UnknownWinding { tag }),
        };
        PathData::new(winding, decode_commands(rest)?)
    }
}

// --- point·handle 식별자 (VI.6.3) ---

/// path anchor point의 안정 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPointId(pub StableId);

/// cubic control handle의 안정 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathHandleId(pub StableId);

/// command 하나에 붙는 id. 끝점이 있는 command(MoveTo·LineTo·CubicTo)는 point
/// id를, CubicTo는 두 handle id를 추가로 가집니다. Close는 id가 없습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandIds {
    /// 끝점 id.
    pub point: Option<PathPointId>,
    /// (c1, c2) handle id.
    pub handles: Option<(PathHandleId, PathHandleId)>,
}

/// point·handle에 StableId가 붙은 path. 개별 point 단위 semantic diff의 기준입니다.
///
/// 불변식: id 목록 길이 = command 수, 각 id 구성이 command 종류와 일치, 모든
/// id가 유일.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedPath {
    data: PathData,
    ids: Vec<CommandIds>,
}

fn expected_shape(command: &PathCommand) -> (bool, bool) {
    match command {
        PathCommand::MoveTo(_) | PathCommand::LineTo(_) => (true, false),
        PathCommand::CubicTo { .. } => (true, true),
        PathCommand::Close => (false, false),
    }
}

impl IdentifiedPath {
    /// 검증 후 만듭니다.
    pub fn new(data: PathData, ids: Vec<CommandIds>) -> Result<IdentifiedPath, PathError> {
        if ids.len() != data.commands().len() {
            return Err(PathError::IdShapeMismatch {
                index: ids.len().min(data.commands().len()),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, (command, id)) in data.commands().iter().zip(&ids).enumerate() {
            let (needs_point, needs_handles) = expected_shape(command);
            if needs_point != id.point.is_some() || needs_handles != id.handles.is_some() {
                return Err(PathError::IdShapeMismatch { index });
            }
            let mut all = Vec::new();
            if let Some(point) = id.point {
                all.push(point.0);
            }
            if let Some((h1, h2)) = id.handles {
                all.extend([h1.0, h2.0]);
            }
            for raw in all {
                if !seen.insert(raw) {
                    return Err(PathError::DuplicateId { id: raw });
                }
            }
        }
        Ok(IdentifiedPath { data, ids })
    }

    /// 새 id를 생성기에서 받아 모든 point·handle에 부여합니다.
    pub fn assign(data: PathData, next_id: &mut dyn FnMut() -> StableId) -> IdentifiedPath {
        let ids = data
            .commands()
            .iter()
            .map(|command| {
                let (needs_point, needs_handles) = expected_shape(command);
                CommandIds {
                    point: needs_point.then(|| PathPointId(next_id())),
                    handles: needs_handles
                        .then(|| (PathHandleId(next_id()), PathHandleId(next_id()))),
                }
            })
            .collect();
        IdentifiedPath { data, ids }
    }

    /// path 데이터.
    pub fn data(&self) -> &PathData {
        &self.data
    }

    /// command별 id.
    pub fn ids(&self) -> &[CommandIds] {
        &self.ids
    }

    /// anchor point id 목록(command 순).
    pub fn point_ids(&self) -> Vec<PathPointId> {
        self.ids.iter().filter_map(|id| id.point).collect()
    }

    /// `index` 위치에 LineTo point를 삽입합니다. 기존 point·handle id는 그대로
    /// 유지되고 새 point만 `id`를 받습니다. 결과가 subpath 규칙을 어기면(예: 맨
    /// 앞, Close 직후) 거부하고 path를 바꾸지 않습니다.
    pub fn insert_line_point(
        &mut self,
        index: usize,
        to: Vec2,
        id: PathPointId,
    ) -> Result<(), PathError> {
        if index > self.ids.len() {
            return Err(PathError::IndexOutOfRange { index });
        }
        let mut commands = self.data.commands().to_vec();
        commands.insert(index, PathCommand::LineTo(to));
        let mut ids = self.ids.clone();
        ids.insert(
            index,
            CommandIds {
                point: Some(id),
                handles: None,
            },
        );
        let data = PathData::new(self.data.winding(), commands)?;
        *self = IdentifiedPath::new(data, ids)?;
        Ok(())
    }
}

// --- bounds (VI.6.4) ---

/// 3차 Bézier 한 축의 극값 파라미터(0 < t < 1).
fn cubic_extrema_t(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    // B'(t)/3 = a t² + b t + c.
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let mut roots = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            roots.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sq = disc.sqrt();
            roots.push((-b + sq) / (2.0 * a));
            roots.push((-b - sq) / (2.0 * a));
        }
    }
    roots.into_iter().filter(|t| *t > 0.0 && *t < 1.0).collect()
}

fn cubic_at(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
}

/// path의 정확한 geometry bounds(control point가 아니라 곡선 자체).
///
/// cubic은 각 축 도함수의 근(극값)을 포함해 계산합니다. 빈 path는 `None`.
pub fn path_bounds(commands: &[PathCommand]) -> Option<stellar_core_types::value::Rect> {
    let mut min = (f64::INFINITY, f64::INFINITY);
    let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut include = |x: f64, y: f64| {
        min = (min.0.min(x), min.1.min(y));
        max = (max.0.max(x), max.1.max(y));
    };
    let mut current: Option<(f64, f64)> = None;
    for command in commands {
        match *command {
            PathCommand::MoveTo(p) | PathCommand::LineTo(p) => {
                let point = (p.x() as f64, p.y() as f64);
                include(point.0, point.1);
                current = Some(point);
            }
            PathCommand::CubicTo { c1, c2, to } => {
                let start = current.unwrap_or((c1.x() as f64, c1.y() as f64));
                let (x1, y1) = (c1.x() as f64, c1.y() as f64);
                let (x2, y2) = (c2.x() as f64, c2.y() as f64);
                let (x3, y3) = (to.x() as f64, to.y() as f64);
                include(start.0, start.1);
                include(x3, y3);
                for t in cubic_extrema_t(start.0, x1, x2, x3) {
                    include(
                        cubic_at(start.0, x1, x2, x3, t),
                        cubic_at(start.1, y1, y2, y3, t),
                    );
                }
                for t in cubic_extrema_t(start.1, y1, y2, y3) {
                    include(
                        cubic_at(start.0, x1, x2, x3, t),
                        cubic_at(start.1, y1, y2, y3, t),
                    );
                }
                current = Some((x3, y3));
            }
            PathCommand::Close => {}
        }
    }
    if min.0.is_infinite() {
        return None;
    }
    let origin = Vec2::new(min.0 as f32, min.1 as f32).ok()?;
    let size =
        stellar_core_types::value::Size2::new((max.0 - min.0) as f32, (max.1 - min.1) as f32)
            .ok()?;
    Some(stellar_core_types::value::Rect::new(origin, size))
}

// --- Shape → Path 파괴 변환 계획 (VI.6.5) ---

/// Shape node를 Path node로 바꾸는 변환 계획.
///
/// **사용자가 명시한 command(`ConvertShapeToPath`, VII.2.7)에서만** 적용합니다 —
/// 렌더·hit test는 VI.5.6 순수 평가를 쓰며 node kind를 바꾸지 않습니다. 계획은
/// StableId를 유지하고, 변환 전 렌더 geometry와 같은 command를 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeToPathPlan {
    /// 유지되는 node StableId.
    pub stable_id: StableId,
    /// 새 node kind(항상 Path).
    pub new_kind: crate::node_kind::NodeKind,
    /// 새 Path payload(point·handle id 부여 완료).
    pub path: IdentifiedPath,
}

/// Shape geometry로부터 파괴 변환 계획을 만듭니다(순수 함수).
///
/// `next_id`는 새 point·handle id 생성기입니다. path는 VI.5.6 평가 결과 그대로라
/// 변환 전후 렌더 geometry가 같습니다.
pub fn plan_shape_to_path(
    stable_id: StableId,
    geometry: &crate::shape::ShapeGeometry,
    next_id: &mut dyn FnMut() -> StableId,
) -> Result<ShapeToPathPlan, PathError> {
    let commands = crate::shape::evaluate_shape_path(geometry);
    let data = PathData::new(WindingRule::NonZero, commands)?;
    Ok(ShapeToPathPlan {
        stable_id,
        new_kind: crate::node_kind::NodeKind::Path,
        path: IdentifiedPath::assign(data, next_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    #[test]
    fn command_round_trip() {
        // 검증 기준(VI.6.1): command round-trip.
        let commands = vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::LineTo(p(10.0, 0.0)),
            PathCommand::CubicTo {
                c1: p(12.0, 2.0),
                c2: p(12.0, 8.0),
                to: p(10.0, 10.0),
            },
            PathCommand::Close,
            PathCommand::MoveTo(p(-5.5, 3.25)),
            PathCommand::LineTo(p(1.0, 1.0)),
        ];
        let bytes = encode_commands(&commands);
        assert_eq!(decode_commands(&bytes).unwrap(), commands);
        assert_eq!(encode_commands(&decode_commands(&bytes).unwrap()), bytes);
        assert!(decode_commands(&encode_commands(&[])).unwrap().is_empty());
    }

    #[test]
    fn quadratic_is_normalized_to_equivalent_cubic() {
        let from = p(0.0, 0.0);
        let control = p(30.0, 60.0);
        let to = p(60.0, 0.0);
        let PathCommand::CubicTo { c1, c2, to: end } =
            quadratic_to_cubic(from, control, to).unwrap()
        else {
            panic!("CubicTo여야 함");
        };
        assert_eq!(end, to);
        // 같은 곡선: t에서 quadratic과 cubic 점이 일치.
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let u = 1.0 - t;
            let qx = u * u * from.x() + 2.0 * u * t * control.x() + t * t * to.x();
            let qy = u * u * from.y() + 2.0 * u * t * control.y() + t * t * to.y();
            let cx = u * u * u * from.x()
                + 3.0 * u * u * t * c1.x()
                + 3.0 * u * t * t * c2.x()
                + t * t * t * end.x();
            let cy = u * u * u * from.y()
                + 3.0 * u * u * t * c1.y()
                + 3.0 * u * t * t * c2.y()
                + t * t * t * end.y();
            assert!((qx - cx).abs() < 1e-3 && (qy - cy).abs() < 1e-3, "t={t}");
        }
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = encode_commands(&[PathCommand::MoveTo(p(1.0, 2.0))]);
        for cut in [0, 3, 4, 5, bytes.len() - 1] {
            assert!(decode_commands(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut unknown = bytes.clone();
        unknown[4] = 9;
        assert_eq!(
            decode_commands(&unknown).unwrap_err(),
            PathError::UnknownCommand { tag: 9 }
        );
        let mut nan = bytes.clone();
        nan[5..9].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_eq!(decode_commands(&nan).unwrap_err(), PathError::NonFinite);
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_commands(&trailing).is_err());
    }

    #[test]
    fn malformed_path_fixtures_fail() {
        // 검증 기준(VI.6.2): malformed path fixture 실패.
        let cases: Vec<(&str, Vec<PathCommand>, usize)> = vec![
            ("LineTo로 시작", vec![PathCommand::LineTo(p(1.0, 1.0))], 0),
            (
                "CubicTo로 시작",
                vec![PathCommand::CubicTo {
                    c1: p(0.0, 0.0),
                    c2: p(1.0, 1.0),
                    to: p(2.0, 2.0),
                }],
                0,
            ),
            ("Close로 시작", vec![PathCommand::Close], 0),
            (
                "Close 뒤 MoveTo 없이 LineTo",
                vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::LineTo(p(1.0, 0.0)),
                    PathCommand::Close,
                    PathCommand::LineTo(p(5.0, 5.0)),
                ],
                3,
            ),
            (
                "Close 두 번",
                vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::Close,
                    PathCommand::Close,
                ],
                2,
            ),
        ];
        for (name, commands, index) in cases {
            assert_eq!(
                PathData::new(WindingRule::NonZero, commands).unwrap_err(),
                PathError::MissingMoveTo { index },
                "{name}"
            );
        }
    }

    #[test]
    fn subpaths_and_winding_round_trip() {
        let path = PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(10.0, 0.0)),
                PathCommand::LineTo(p(10.0, 10.0)),
                PathCommand::Close,
                PathCommand::MoveTo(p(2.0, 2.0)),
                PathCommand::LineTo(p(4.0, 4.0)),
            ],
        )
        .unwrap();
        let subpaths = path.subpaths();
        assert_eq!(subpaths.len(), 2);
        assert!(subpaths[0].closed);
        assert_eq!(subpaths[0].commands.len(), 3);
        assert!(!subpaths[1].closed, "열린 subpath");
        let decoded = PathData::decode(&path.encode()).unwrap();
        assert_eq!(decoded, path);
        assert_eq!(decoded.winding(), WindingRule::EvenOdd);
        // 빈 path는 유효.
        assert!(PathData::new(WindingRule::NonZero, vec![])
            .unwrap()
            .subpaths()
            .is_empty());
        // 알 수 없는 winding tag.
        let mut bytes = path.encode();
        bytes[0] = 7;
        assert_eq!(
            PathData::decode(&bytes).unwrap_err(),
            PathError::UnknownWinding { tag: 7 }
        );
    }

    mod identified {
        use super::super::*;

        fn p(x: f32, y: f32) -> Vec2 {
            Vec2::new(x, y).unwrap()
        }

        fn sample() -> PathData {
            PathData::new(
                WindingRule::NonZero,
                vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::LineTo(p(10.0, 0.0)),
                    PathCommand::CubicTo {
                        c1: p(12.0, 2.0),
                        c2: p(12.0, 8.0),
                        to: p(10.0, 10.0),
                    },
                    PathCommand::Close,
                ],
            )
            .unwrap()
        }

        fn generator() -> impl FnMut() -> StableId {
            let mut n = 0u64;
            move || {
                n += 1;
                StableId::new_v7(1_000 + n, 0, n)
            }
        }

        #[test]
        fn inserting_a_point_keeps_existing_ids() {
            // 검증 기준(VI.6.3): point 삽입 후 기존 point ID 유지.
            let mut gen = generator();
            let mut path = IdentifiedPath::assign(sample(), &mut gen);
            let before_points = path.point_ids();
            let before_handles: Vec<_> = path.ids().iter().filter_map(|i| i.handles).collect();
            assert_eq!(before_points.len(), 3);

            let new_id = PathPointId(StableId::new_v7(9_999, 0, 9_999));
            path.insert_line_point(2, p(11.0, 5.0), new_id).unwrap();

            let after = path.point_ids();
            assert_eq!(after.len(), 4);
            assert_eq!(after[0], before_points[0]);
            assert_eq!(after[1], before_points[1]);
            assert_eq!(after[2], new_id, "새 point만 새 id");
            assert_eq!(after[3], before_points[2]);
            let after_handles: Vec<_> = path.ids().iter().filter_map(|i| i.handles).collect();
            assert_eq!(after_handles, before_handles, "handle id 유지");
        }

        #[test]
        fn invalid_insertions_are_rejected_without_change() {
            let mut gen = generator();
            let mut path = IdentifiedPath::assign(sample(), &mut gen);
            let snapshot = path.clone();
            let id = PathPointId(StableId::new_v7(9_999, 0, 9_999));
            // 맨 앞(MoveTo 앞)과 Close 뒤는 subpath 규칙 위반.
            assert!(matches!(
                path.insert_line_point(0, p(1.0, 1.0), id),
                Err(PathError::MissingMoveTo { .. })
            ));
            assert!(matches!(
                path.insert_line_point(4, p(1.0, 1.0), id),
                Err(PathError::MissingMoveTo { .. })
            ));
            assert!(matches!(
                path.insert_line_point(99, p(1.0, 1.0), id),
                Err(PathError::IndexOutOfRange { index: 99 })
            ));
            // 기존 id와 중복되는 새 id.
            let existing = path.point_ids()[0];
            assert!(matches!(
                path.insert_line_point(1, p(1.0, 1.0), existing),
                Err(PathError::DuplicateId { .. })
            ));
            assert_eq!(path, snapshot, "거부 시 불변");
        }

        #[test]
        fn id_shape_must_match_commands() {
            let data = sample();
            let bad = vec![
                CommandIds {
                    point: None,
                    handles: None,
                };
                4
            ];
            assert_eq!(
                IdentifiedPath::new(data, bad).unwrap_err(),
                PathError::IdShapeMismatch { index: 0 }
            );
        }
    }

    mod bounds {
        use super::super::*;

        fn p(x: f32, y: f32) -> Vec2 {
            Vec2::new(x, y).unwrap()
        }

        fn assert_bounds(commands: &[PathCommand], expected: (f32, f32, f32, f32), tol: f32) {
            let r = path_bounds(commands).expect("bounds");
            let got = (
                r.origin().x(),
                r.origin().y(),
                r.size().width(),
                r.size().height(),
            );
            let e = [expected.0, expected.1, expected.2, expected.3];
            for (a, b) in [got.0, got.1, got.2, got.3].iter().zip(e) {
                assert!((a - b).abs() <= tol, "{got:?} vs {expected:?}");
            }
        }

        #[test]
        fn cubic_extrema_analytic_fixtures() {
            // 검증 기준(VI.6.4): analytic reference fixture 허용 오차 통과.
            // 아치: y 극값은 t=0.5에서 0.375·100·2 = 75 (control point 100이 아님).
            let arch = [
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::CubicTo {
                    c1: p(0.0, 100.0),
                    c2: p(100.0, 100.0),
                    to: p(100.0, 0.0),
                },
            ];
            assert_bounds(&arch, (0.0, 0.0, 100.0, 75.0), 1e-4);

            // 옆으로 볼록: x(t) = 300·t(1−t) → t=0.5에서 최대 75 (control x=100이 아님).
            let side = [
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::CubicTo {
                    c1: p(100.0, 0.0),
                    c2: p(100.0, 100.0),
                    to: p(0.0, 100.0),
                },
            ];
            assert_bounds(&side, (0.0, 0.0, 75.0, 100.0), 1e-4);

            // 비대칭 곡선: 독립 기준으로 10,001점 고밀도 샘플링과 비교.
            let (p0, p1, p2, p3) = (
                (0.0f64, 0.0f64),
                (130.0, -40.0),
                (-60.0, 90.0),
                (40.0, 20.0),
            );
            let curve = [
                PathCommand::MoveTo(p(p0.0 as f32, p0.1 as f32)),
                PathCommand::CubicTo {
                    c1: p(p1.0 as f32, p1.1 as f32),
                    c2: p(p2.0 as f32, p2.1 as f32),
                    to: p(p3.0 as f32, p3.1 as f32),
                },
            ];
            let mut lo = (f64::INFINITY, f64::INFINITY);
            let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for i in 0..=10_000 {
                let s = i as f64 / 10_000.0;
                let u = 1.0 - s;
                let bx = u * u * u * p0.0
                    + 3.0 * u * u * s * p1.0
                    + 3.0 * u * s * s * p2.0
                    + s * s * s * p3.0;
                let by = u * u * u * p0.1
                    + 3.0 * u * u * s * p1.1
                    + 3.0 * u * s * s * p2.1
                    + s * s * s * p3.1;
                lo = (lo.0.min(bx), lo.1.min(by));
                hi = (hi.0.max(bx), hi.1.max(by));
            }
            assert_bounds(
                &curve,
                (
                    lo.0 as f32,
                    lo.1 as f32,
                    (hi.0 - lo.0) as f32,
                    (hi.1 - lo.1) as f32,
                ),
                1e-2,
            );
            // 극값이 control point hull보다 안쪽(control bbox와 다름).
            let r = path_bounds(&curve).unwrap();
            assert!(r.origin().y() > -40.0 && r.origin().y() + r.size().height() < 90.0);
        }

        #[test]
        fn ellipse_path_bounds_equal_its_bounds() {
            use crate::shape::EllipseParams;
            use stellar_core_types::value::{Rect, Size2};
            let ellipse = EllipseParams {
                bounds: Rect::new(p(10.0, 20.0), Size2::new(200.0, 100.0).unwrap()),
            };
            assert_bounds(&ellipse.to_path(), (10.0, 20.0, 200.0, 100.0), 1e-3);
        }

        #[test]
        fn lines_and_empty_path() {
            let poly = [
                PathCommand::MoveTo(p(5.0, 5.0)),
                PathCommand::LineTo(p(-3.0, 8.0)),
                PathCommand::LineTo(p(4.0, -2.0)),
                PathCommand::Close,
            ];
            assert_bounds(&poly, (-3.0, -2.0, 8.0, 10.0), 0.0);
            assert!(path_bounds(&[]).is_none());
        }
    }

    mod shape_conversion {
        use super::super::*;
        use crate::frame::CornerRadius;
        use crate::node_kind::NodeKind;
        use crate::shape::{
            evaluate_shape_path, EllipseParams, LineCap, LineParams, RectangleParams, ShapeGeometry,
        };
        use stellar_core_types::value::{Rect, Size2};

        fn p(x: f32, y: f32) -> Vec2 {
            Vec2::new(x, y).unwrap()
        }

        fn ids() -> impl FnMut() -> StableId {
            let mut n = 0u64;
            move || {
                n += 1;
                StableId::new_v7(50_000 + n, 0, n)
            }
        }

        #[test]
        fn conversion_keeps_id_and_render_geometry() {
            // 검증 기준(VI.6.5): 변환 전 렌더 geometry 동일.
            let node = StableId::new_v7(7, 0, 7);
            let geometries = [
                ShapeGeometry::Rectangle(RectangleParams {
                    size: Size2::new(80.0, 40.0).unwrap(),
                    corner_radius: CornerRadius::new(6.0, 6.0, 0.0, 12.0).unwrap(),
                }),
                ShapeGeometry::Ellipse(EllipseParams {
                    bounds: Rect::new(p(0.0, 0.0), Size2::new(30.0, 60.0).unwrap()),
                }),
                ShapeGeometry::Line(LineParams {
                    start: p(0.0, 0.0),
                    end: p(10.0, 5.0),
                    cap: LineCap::Round,
                }),
            ];
            for geometry in geometries {
                let mut gen = ids();
                let plan = plan_shape_to_path(node, &geometry, &mut gen).unwrap();
                assert_eq!(plan.stable_id, node, "StableId 유지");
                assert_eq!(plan.new_kind, NodeKind::Path);
                assert_eq!(
                    plan.path.data().commands(),
                    evaluate_shape_path(&geometry).as_slice(),
                    "{}: 렌더 geometry 동일",
                    geometry.kind().name()
                );
                assert_eq!(
                    path_bounds(plan.path.data().commands()),
                    path_bounds(&evaluate_shape_path(&geometry))
                );
                // 모든 point에 id 부여.
                assert!(plan
                    .path
                    .ids()
                    .iter()
                    .all(|i| i.point.is_some() || i.handles.is_none()));
            }
        }

        #[test]
        fn planning_is_pure() {
            let geometry = ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(p(1.0, 1.0), Size2::new(2.0, 2.0).unwrap()),
            });
            let before = geometry;
            let mut gen = ids();
            let _ = plan_shape_to_path(StableId::new_v7(1, 0, 1), &geometry, &mut gen).unwrap();
            assert_eq!(geometry, before, "원본 geometry 불변");
        }
    }
}
