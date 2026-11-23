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
    /// path에 없는 point·handle id입니다(ADR-0079).
    UnknownElement {
        /// 요청 id.
        id: StableId,
    },
    /// 이 path에 적용할 수 없는 point 편집입니다(ADR-0079).
    InvalidEdit {
        /// 원인.
        reason: &'static str,
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
            PathError::UnknownElement { id } => write!(f, "path에 point/handle {id}가 없습니다"),
            PathError::InvalidEdit { reason } => write!(f, "path point 편집 거부: {reason}"),
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

/// anchor point의 두 handle 관계(ADR-0079). tag는 저장 포맷의 일부입니다.
///
/// - `Independent`: 두 handle이 따로 움직입니다(기본, 저장하지 않음).
/// - `Aligned`: 한 handle을 움직이면 반대 handle이 같은 직선 위 반대 방향으로 돌고 길이는 유지합니다.
/// - `Mirrored`: 반대 handle이 anchor 기준 점대칭(방향 반대, 길이 같음)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum HandleMode {
    /// 독립(기본).
    #[default]
    Independent = 0,
    /// 방향만 맞춤.
    Aligned = 1,
    /// 방향·길이 대칭.
    Mirrored = 2,
}

impl HandleMode {
    /// 저장 tag에서 복원합니다. 기본값(0)은 저장하지 않으므로 거부합니다.
    fn from_stored_tag(tag: u8) -> Option<HandleMode> {
        match tag {
            1 => Some(HandleMode::Aligned),
            2 => Some(HandleMode::Mirrored),
            _ => None,
        }
    }

    /// tag에서 복원합니다(기본값 포함).
    pub fn from_tag(tag: u8) -> Option<HandleMode> {
        match tag {
            0 => Some(HandleMode::Independent),
            tag => HandleMode::from_stored_tag(tag),
        }
    }
}

/// point·handle에 StableId가 붙은 path. 개별 point 단위 semantic diff의 기준입니다.
///
/// 불변식: id 목록 길이 = command 수, 각 id 구성이 command 종류와 일치, 모든
/// id가 유일. handle mode는 기본값이 아닌 point만, 있는 point id로만 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifiedPath {
    data: PathData,
    ids: Vec<CommandIds>,
    modes: std::collections::BTreeMap<PathPointId, HandleMode>,
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
        Ok(IdentifiedPath {
            data,
            ids,
            modes: Default::default(),
        })
    }

    /// 기본값이 아닌 handle mode를 붙입니다. 없는 point id나 기본값은 거부합니다(ADR-0079).
    pub fn with_handle_modes(
        mut self,
        modes: impl IntoIterator<Item = (PathPointId, HandleMode)>,
    ) -> Result<IdentifiedPath, PathError> {
        for (point, mode) in modes {
            if self.point_index(point).is_none() {
                return Err(PathError::UnknownElement { id: point.0 });
            }
            if mode == HandleMode::Independent {
                return Err(PathError::Malformed {
                    reason: "기본 handle mode는 저장하지 않습니다",
                });
            }
            self.modes.insert(point, mode);
        }
        Ok(self)
    }

    /// point의 handle mode(없으면 기본 `Independent`).
    pub fn handle_mode(&self, point: PathPointId) -> HandleMode {
        self.modes.get(&point).copied().unwrap_or_default()
    }

    /// 기본값이 아닌 handle mode(point id 오름차순).
    pub fn handle_modes(&self) -> impl Iterator<Item = (PathPointId, HandleMode)> + '_ {
        self.modes.iter().map(|(point, mode)| (*point, *mode))
    }

    /// point의 command 위치.
    pub fn point_index(&self, point: PathPointId) -> Option<usize> {
        self.ids.iter().position(|ids| ids.point == Some(point))
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
        IdentifiedPath {
            data,
            ids,
            modes: Default::default(),
        }
    }

    /// path 데이터.
    pub fn data(&self) -> &PathData {
        &self.data
    }

    /// command별 id.
    pub fn ids(&self) -> &[CommandIds] {
        &self.ids
    }

    /// 저장 순서로 각 point 및 cubic handle의 전체 StableId를 순회합니다.
    pub fn element_ids(&self) -> impl Iterator<Item = StableId> + '_ {
        self.ids.iter().flat_map(|ids| {
            ids.point.map(|point| point.0).into_iter().chain(
                ids.handles
                    .into_iter()
                    .flat_map(|(first, second)| [first.0, second.0]),
            )
        })
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
        let modes = std::mem::take(&mut self.modes);
        *self = IdentifiedPath::new(data, ids)?.with_handle_modes(modes)?;
        Ok(())
    }
}

// --- point 편집 (XII.10.28, ADR-0079) ---

/// Path point 편집 연산(closed enum). 대상은 point·handle StableId로 가리키며, 편집하지 않은 point·handle의
/// id는 그대로 유지합니다. tag는 command 직렬화 계약입니다.
///
/// handle 소속: `CubicTo { c1, c2, to }`의 `c1`은 **앞 point의 나가는 handle**, `c2`는 **`to` point의 들어오는
/// handle**입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathEdit {
    /// `segment` point로 끝나는 선분·곡선을 `t`(0 < t < 1)에서 둘로 나눠 새 point `point`를 넣습니다. 곡선은
    /// de Casteljau로 나눠 모양을 그대로 두며, 새로 생기는 두 handle id(`handles`, 앞 곡선의 c2·뒤 곡선의 c1)가
    /// 필요합니다. 직선이면 `handles`는 `None`입니다.
    InsertPoint {
        /// 나눌 segment의 끝 point.
        segment: PathPointId,
        /// 나눌 위치(0 < t < 1).
        t: stellar_core_types::value::FiniteF32,
        /// 새 point id.
        point: PathPointId,
        /// 곡선일 때 새 handle id(앞 곡선 c2, 뒤 곡선 c1).
        handles: Option<(PathHandleId, PathHandleId)>,
    },
    /// anchor를 옮깁니다. 그 point의 두 handle도 같은 만큼 옮깁니다.
    MovePoint {
        /// 대상 point.
        point: PathPointId,
        /// 새 위치.
        to: Vec2,
    },
    /// point를 지웁니다. 앞뒤 segment를 하나로 합치며(둘 다 곡선이면 바깥 두 handle로 곡선, 아니면 직선),
    /// subpath 시작 point면 다음 point가 시작이 됩니다. point 하나뿐인 subpath는 통째로 지웁니다.
    DeletePoint {
        /// 대상 point.
        point: PathPointId,
    },
    /// handle 하나를 옮기고, 소속 point의 [`HandleMode`]에 따라 반대 handle을 맞춥니다.
    MoveHandle {
        /// 대상 handle.
        handle: PathHandleId,
        /// 새 위치.
        to: Vec2,
    },
    /// point의 handle mode를 바꿉니다. `Aligned`·`Mirrored`로 바꾸면 들어오는 handle을 기준으로 나가는
    /// handle을 맞춥니다.
    SetHandleMode {
        /// 대상 point.
        point: PathPointId,
        /// 새 mode.
        mode: HandleMode,
    },
    /// `start` point(MoveTo)로 시작하는 열린 subpath를 닫습니다.
    CloseSubpath {
        /// subpath 시작 point.
        start: PathPointId,
    },
    /// `start` point(MoveTo)로 시작하는 닫힌 subpath를 엽니다.
    OpenSubpath {
        /// subpath 시작 point.
        start: PathPointId,
    },
}

fn invalid_edit(reason: &'static str) -> PathError {
    PathError::InvalidEdit { reason }
}

fn lerp(a: Vec2, b: Vec2, t: f32) -> Result<Vec2, PathError> {
    Ok(Vec2::new(
        a.x() + (b.x() - a.x()) * t,
        a.y() + (b.y() - a.y()) * t,
    )?)
}

fn offset(point: Vec2, dx: f32, dy: f32) -> Result<Vec2, PathError> {
    Ok(Vec2::new(point.x() + dx, point.y() + dy)?)
}

/// `anchor` 기준으로 `reference` handle의 반대쪽 handle을 mode에 맞게 계산합니다. `current`는 지금 반대
/// handle 위치(Aligned에서 길이 유지).
fn opposite_handle(
    mode: HandleMode,
    anchor: Vec2,
    reference: Vec2,
    current: Vec2,
) -> Result<Vec2, PathError> {
    let (dx, dy) = (reference.x() - anchor.x(), reference.y() - anchor.y());
    match mode {
        HandleMode::Independent => Ok(current),
        HandleMode::Mirrored => offset(anchor, -dx, -dy),
        HandleMode::Aligned => {
            let len = (dx * dx + dy * dy).sqrt();
            if len == 0.0 {
                return Ok(current);
            }
            let keep =
                ((current.x() - anchor.x()).powi(2) + (current.y() - anchor.y()).powi(2)).sqrt();
            offset(anchor, -dx / len * keep, -dy / len * keep)
        }
    }
}

fn endpoint(command: &PathCommand) -> Option<Vec2> {
    match command {
        PathCommand::MoveTo(p) | PathCommand::LineTo(p) => Some(*p),
        PathCommand::CubicTo { to, .. } => Some(*to),
        PathCommand::Close => None,
    }
}

fn with_endpoint(command: PathCommand, to: Vec2) -> PathCommand {
    match command {
        PathCommand::MoveTo(_) => PathCommand::MoveTo(to),
        PathCommand::LineTo(_) => PathCommand::LineTo(to),
        PathCommand::CubicTo { c1, c2, .. } => PathCommand::CubicTo { c1, c2, to },
        PathCommand::Close => PathCommand::Close,
    }
}

impl IdentifiedPath {
    /// handle의 (command 위치, `c1`인지).
    fn handle_index(&self, handle: PathHandleId) -> Option<(usize, bool)> {
        self.ids
            .iter()
            .enumerate()
            .find_map(|(index, ids)| match ids.handles {
                Some((c1, _)) if c1 == handle => Some((index, true)),
                Some((_, c2)) if c2 == handle => Some((index, false)),
                _ => None,
            })
    }

    /// `index` command가 속한 subpath의 (MoveTo 위치, 마지막 비-Close command 위치, 닫혔는지).
    fn subpath_bounds(&self, index: usize) -> (usize, usize, bool) {
        let commands = self.data.commands();
        let start = (0..=index)
            .rev()
            .find(|i| matches!(commands[*i], PathCommand::MoveTo(_)))
            .unwrap_or(0);
        let mut last = start;
        let mut closed = false;
        for (i, command) in commands.iter().enumerate().skip(start + 1) {
            match command {
                PathCommand::MoveTo(_) => break,
                PathCommand::Close => {
                    closed = true;
                    break;
                }
                _ => last = i,
            }
        }
        (start, last, closed)
    }

    fn rebuilt(
        &self,
        commands: Vec<PathCommand>,
        ids: Vec<CommandIds>,
        modes: std::collections::BTreeMap<PathPointId, HandleMode>,
    ) -> Result<IdentifiedPath, PathError> {
        let data = PathData::new(self.data.winding(), commands)?;
        IdentifiedPath::new(data, ids)?.with_handle_modes(modes)
    }

    fn required_point(&self, point: PathPointId) -> Result<usize, PathError> {
        self.point_index(point)
            .ok_or(PathError::UnknownElement { id: point.0 })
    }

    /// point 편집을 적용한 새 path를 반환합니다. 이 path는 바꾸지 않습니다. 편집하지 않은 point·handle의 id와
    /// mode는 유지합니다.
    pub fn edited(&self, edit: &PathEdit) -> Result<IdentifiedPath, PathError> {
        let mut commands = self.data.commands().to_vec();
        let mut ids = self.ids.clone();
        let mut modes = self.modes.clone();
        match *edit {
            PathEdit::InsertPoint {
                segment,
                t,
                point,
                handles,
            } => {
                let t = t.get();
                if !(t > 0.0 && t < 1.0) {
                    return Err(invalid_edit("나눌 위치는 0과 1 사이여야 합니다"));
                }
                let i = self.required_point(segment)?;
                let from = match i.checked_sub(1).map(|p| &commands[p]) {
                    Some(previous) => endpoint(previous),
                    None => None,
                };
                match (commands[i], from, handles) {
                    (PathCommand::LineTo(to), Some(from), None) => {
                        commands.insert(i, PathCommand::LineTo(lerp(from, to, t)?));
                        ids.insert(
                            i,
                            CommandIds {
                                point: Some(point),
                                handles: None,
                            },
                        );
                    }
                    (PathCommand::CubicTo { c1, c2, to }, Some(from), Some((left, right))) => {
                        let (q0, q1, q2) = (lerp(from, c1, t)?, lerp(c1, c2, t)?, lerp(c2, to, t)?);
                        let (r0, r1) = (lerp(q0, q1, t)?, lerp(q1, q2, t)?);
                        let split = lerp(r0, r1, t)?;
                        let (h1, h2) = ids[i]
                            .handles
                            .ok_or(PathError::IdShapeMismatch { index: i })?;
                        commands[i] = PathCommand::CubicTo { c1: r1, c2: q2, to };
                        ids[i].handles = Some((right, h2));
                        commands.insert(
                            i,
                            PathCommand::CubicTo {
                                c1: q0,
                                c2: r0,
                                to: split,
                            },
                        );
                        ids.insert(
                            i,
                            CommandIds {
                                point: Some(point),
                                handles: Some((h1, left)),
                            },
                        );
                    }
                    (PathCommand::MoveTo(_), _, _) => {
                        return Err(invalid_edit(
                            "subpath 시작 point로 끝나는 segment는 없습니다",
                        ))
                    }
                    _ => {
                        return Err(invalid_edit(
                            "곡선 분할에는 새 handle id 두 개가, 직선에는 0개가 필요합니다",
                        ))
                    }
                }
            }
            PathEdit::MovePoint { point, to } => {
                let i = self.required_point(point)?;
                let old = endpoint(&commands[i]).ok_or(PathError::IdShapeMismatch { index: i })?;
                let (dx, dy) = (to.x() - old.x(), to.y() - old.y());
                commands[i] = with_endpoint(commands[i], to);
                if let PathCommand::CubicTo { c1, c2, to } = commands[i] {
                    commands[i] = PathCommand::CubicTo {
                        c1,
                        c2: offset(c2, dx, dy)?,
                        to,
                    };
                }
                if let Some(PathCommand::CubicTo { c1, c2, to }) = commands.get(i + 1).copied() {
                    commands[i + 1] = PathCommand::CubicTo {
                        c1: offset(c1, dx, dy)?,
                        c2,
                        to,
                    };
                }
            }
            PathEdit::DeletePoint { point } => {
                let i = self.required_point(point)?;
                modes.remove(&point);
                let next = commands.get(i + 1).copied();
                let next_is_segment = matches!(
                    next,
                    Some(PathCommand::LineTo(_) | PathCommand::CubicTo { .. })
                );
                match commands[i] {
                    PathCommand::MoveTo(_) if next_is_segment => {
                        let to = endpoint(&commands[i + 1]).expect("segment endpoint");
                        commands[i + 1] = PathCommand::MoveTo(to);
                        ids[i + 1].handles = None;
                        commands.remove(i);
                        ids.remove(i);
                    }
                    PathCommand::MoveTo(_) => {
                        // point 하나뿐인 subpath: MoveTo(와 Close)를 지웁니다.
                        let end = if matches!(next, Some(PathCommand::Close)) {
                            i + 2
                        } else {
                            i + 1
                        };
                        commands.drain(i..end);
                        ids.drain(i..end);
                    }
                    current if next_is_segment => {
                        let following = next.expect("segment");
                        let to = endpoint(&following).expect("segment endpoint");
                        match (current, following) {
                            (PathCommand::CubicTo { c1, .. }, PathCommand::CubicTo { c2, .. }) => {
                                let (h1, _) = ids[i]
                                    .handles
                                    .ok_or(PathError::IdShapeMismatch { index: i })?;
                                let (_, h2) = ids[i + 1]
                                    .handles
                                    .ok_or(PathError::IdShapeMismatch { index: i + 1 })?;
                                commands[i + 1] = PathCommand::CubicTo { c1, c2, to };
                                ids[i + 1].handles = Some((h1, h2));
                            }
                            _ => {
                                commands[i + 1] = PathCommand::LineTo(to);
                                ids[i + 1].handles = None;
                            }
                        }
                        commands.remove(i);
                        ids.remove(i);
                    }
                    _ => {
                        commands.remove(i);
                        ids.remove(i);
                    }
                }
            }
            PathEdit::MoveHandle { handle, to } => {
                let (i, is_c1) = self
                    .handle_index(handle)
                    .ok_or(PathError::UnknownElement { id: handle.0 })?;
                let PathCommand::CubicTo { c1, c2, to: end } = commands[i] else {
                    return Err(PathError::IdShapeMismatch { index: i });
                };
                if is_c1 {
                    commands[i] = PathCommand::CubicTo {
                        c1: to,
                        c2,
                        to: end,
                    };
                    // 앞 point의 나가는 handle: 반대는 앞 command의 c2(들어오는 handle).
                    let owner = i - 1;
                    let anchor = endpoint(&commands[owner]).expect("앞 point");
                    let mode = ids[owner]
                        .point
                        .map_or(HandleMode::Independent, |p| self.handle_mode(p));
                    if let PathCommand::CubicTo { c1, c2, to: e } = commands[owner] {
                        commands[owner] = PathCommand::CubicTo {
                            c1,
                            c2: opposite_handle(mode, anchor, to, c2)?,
                            to: e,
                        };
                    }
                } else {
                    commands[i] = PathCommand::CubicTo {
                        c1,
                        c2: to,
                        to: end,
                    };
                    let mode = ids[i]
                        .point
                        .map_or(HandleMode::Independent, |p| self.handle_mode(p));
                    if let Some(PathCommand::CubicTo { c1, c2, to: e }) =
                        commands.get(i + 1).copied()
                    {
                        commands[i + 1] = PathCommand::CubicTo {
                            c1: opposite_handle(mode, end, to, c1)?,
                            c2,
                            to: e,
                        };
                    }
                }
            }
            PathEdit::SetHandleMode { point, mode } => {
                let i = self.required_point(point)?;
                if mode == HandleMode::Independent {
                    modes.remove(&point);
                } else {
                    modes.insert(point, mode);
                }
                let anchor =
                    endpoint(&commands[i]).ok_or(PathError::IdShapeMismatch { index: i })?;
                if let (
                    PathCommand::CubicTo { c2: incoming, .. },
                    Some(PathCommand::CubicTo { c1, c2, to }),
                ) = (commands[i], commands.get(i + 1).copied())
                {
                    commands[i + 1] = PathCommand::CubicTo {
                        c1: opposite_handle(mode, anchor, incoming, c1)?,
                        c2,
                        to,
                    };
                }
            }
            PathEdit::CloseSubpath { start } | PathEdit::OpenSubpath { start } => {
                let i = self.required_point(start)?;
                if !matches!(commands[i], PathCommand::MoveTo(_)) {
                    return Err(invalid_edit(
                        "subpath는 시작 point(MoveTo)로 가리켜야 합니다",
                    ));
                }
                let (_, last, closed) = self.subpath_bounds(i);
                match (edit, closed) {
                    (PathEdit::CloseSubpath { .. }, false) => {
                        commands.insert(last + 1, PathCommand::Close);
                        ids.insert(
                            last + 1,
                            CommandIds {
                                point: None,
                                handles: None,
                            },
                        );
                    }
                    (PathEdit::OpenSubpath { .. }, true) => {
                        commands.remove(last + 1);
                        ids.remove(last + 1);
                    }
                    (PathEdit::CloseSubpath { .. }, true) => {
                        return Err(invalid_edit("이미 닫힌 subpath입니다"))
                    }
                    _ => return Err(invalid_edit("이미 열린 subpath입니다")),
                }
            }
        }
        self.rebuilt(commands, ids, modes)
    }
}

// --- IdentifiedPath canonical 인코딩 (VI.10.2) ---
//
// 형식: `u32` 길이 + [`PathData::encode`] bytes, `u32` id 항목 수, 항목마다 flag(u8: bit0
// point, bit1 handles) + point id 16 + handle id 16×2. Close 항목은 flag 0만 기록합니다.

const IDS_POINT: u8 = 0b01;
const IDS_HANDLES: u8 = 0b10;

fn take_u32(reader: &mut Reader<'_>) -> Result<u32, PathError> {
    let raw: [u8; 4] = reader
        .take(4)?
        .try_into()
        .map_err(|_| PathError::Malformed {
            reason: "입력 부족",
        })?;
    Ok(u32::from_le_bytes(raw))
}

fn take_stable_id(reader: &mut Reader<'_>) -> Result<StableId, PathError> {
    let mut raw = [0u8; 16];
    raw.copy_from_slice(reader.take(16)?);
    Ok(StableId::from_bytes(raw))
}

impl IdentifiedPath {
    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let data = self.data.encode();
        let mut buf = Vec::with_capacity(data.len() + 8 + self.ids.len() * 49);
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&data);
        buf.extend_from_slice(&(self.ids.len() as u32).to_le_bytes());
        for ids in &self.ids {
            let mut flag = 0u8;
            if ids.point.is_some() {
                flag |= IDS_POINT;
            }
            if ids.handles.is_some() {
                flag |= IDS_HANDLES;
            }
            buf.push(flag);
            if let Some(point) = ids.point {
                buf.extend_from_slice(&point.0.to_bytes());
            }
            if let Some((h1, h2)) = ids.handles {
                buf.extend_from_slice(&h1.0.to_bytes());
                buf.extend_from_slice(&h2.0.to_bytes());
            }
        }
        // handle mode 절(ADR-0079): 기본값이 아닌 point가 있을 때만 씁니다. 없으면 이전 형식과 같은 bytes입니다.
        if !self.modes.is_empty() {
            buf.extend_from_slice(&(self.modes.len() as u32).to_le_bytes());
            for (point, mode) in &self.modes {
                buf.extend_from_slice(&point.0.to_bytes());
                buf.push(*mode as u8);
            }
        }
        buf
    }

    /// bytes에서 복원하고 검증합니다(subpath 규칙, id 구성·유일성).
    pub fn decode(bytes: &[u8]) -> Result<IdentifiedPath, PathError> {
        let mut reader = Reader {
            input: bytes,
            pos: 0,
        };
        let data_len = take_u32(&mut reader)? as usize;
        let data = PathData::decode(reader.take(data_len)?)?;
        let count = take_u32(&mut reader)? as usize;
        let mut ids = Vec::with_capacity(count.min(data.commands().len()));
        for _ in 0..count {
            let flag = reader.take(1)?[0];
            if flag & !(IDS_POINT | IDS_HANDLES) != 0 {
                return Err(PathError::Malformed {
                    reason: "알 수 없는 id flag",
                });
            }
            let point = if flag & IDS_POINT != 0 {
                Some(PathPointId(take_stable_id(&mut reader)?))
            } else {
                None
            };
            let handles = if flag & IDS_HANDLES != 0 {
                let h1 = PathHandleId(take_stable_id(&mut reader)?);
                let h2 = PathHandleId(take_stable_id(&mut reader)?);
                Some((h1, h2))
            } else {
                None
            };
            ids.push(CommandIds { point, handles });
        }
        let mut modes = Vec::new();
        if reader.pos != bytes.len() {
            let count = take_u32(&mut reader)? as usize;
            if count == 0 {
                return Err(PathError::Malformed {
                    reason: "빈 handle mode 절",
                });
            }
            for _ in 0..count {
                let point = PathPointId(take_stable_id(&mut reader)?);
                let mode = HandleMode::from_stored_tag(reader.take(1)?[0]).ok_or(
                    PathError::Malformed {
                        reason: "알 수 없는 handle mode",
                    },
                )?;
                if modes.last().is_some_and(|(last, _)| *last >= point) {
                    return Err(PathError::Malformed {
                        reason: "handle mode 순서",
                    });
                }
                modes.push((point, mode));
            }
        }
        if reader.pos != bytes.len() {
            return Err(PathError::Malformed {
                reason: "trailing bytes",
            });
        }
        IdentifiedPath::new(data, ids)?.with_handle_modes(modes)
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
    // 평가 좌표가 f32 범위를 넘는 geometry(디코딩은 거부)는 panic 대신 오류입니다.
    let commands = crate::shape::evaluate_shape_path(geometry).map_err(|_| PathError::NonFinite)?;
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
            assert_bounds(
                &ellipse.to_path().unwrap(),
                (10.0, 20.0, 200.0, 100.0),
                1e-3,
            );
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
                    evaluate_shape_path(&geometry).unwrap().as_slice(),
                    "{}: 렌더 geometry 동일",
                    geometry.kind().name()
                );
                assert_eq!(
                    path_bounds(plan.path.data().commands()),
                    path_bounds(&evaluate_shape_path(&geometry).unwrap())
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

    #[test]
    fn identified_path_codec_round_trips() {
        // VI.10.2: node 레코드 직렬화용 IdentifiedPath 인코딩(point·handle id 보존).
        let data = PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::CubicTo {
                    c1: p(1.0, 2.0),
                    c2: p(3.0, 4.0),
                    to: p(5.0, 0.0),
                },
                PathCommand::LineTo(p(5.0, 5.0)),
                PathCommand::Close,
            ],
        )
        .unwrap();
        let mut next = 0u64;
        let path = IdentifiedPath::assign(data, &mut || {
            next += 1;
            StableId::new_v7(next, 0, next)
        });
        let bytes = path.encode();
        let decoded = IdentifiedPath::decode(&bytes).unwrap();
        assert_eq!(decoded, path);
        assert_eq!(decoded.encode(), bytes);
        assert_eq!(decoded.point_ids(), path.point_ids());
        // 빈 path.
        let empty = IdentifiedPath::new(PathData::default(), vec![]).unwrap();
        assert_eq!(IdentifiedPath::decode(&empty.encode()).unwrap(), empty);
        // 절단·trailing·잘못된 flag 거부.
        for cut in [0, 3, bytes.len() - 1] {
            assert!(IdentifiedPath::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(IdentifiedPath::decode(&extended).is_err());
        let data_len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let flag_at = 4 + data_len + 4;
        let mut bad_flag = bytes.clone();
        bad_flag[flag_at] = 0b100;
        assert!(IdentifiedPath::decode(&bad_flag).is_err());
        // id 구성 불일치(MoveTo에 handle flag) 거부.
        let mut mismatch = bytes;
        mismatch[flag_at] = IDS_POINT | IDS_HANDLES;
        assert!(IdentifiedPath::decode(&mismatch).is_err());
    }

    #[test]
    fn handle_mode_section_is_optional_and_validated() {
        // ADR-0079: 기본 mode만 있는 path는 이전 형식과 같은 bytes이고, mode 절은 있을 때만 씁니다.
        let data = PathData::new(
            WindingRule::NonZero,
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::CubicTo {
                    c1: p(1.0, 0.0),
                    c2: p(2.0, 1.0),
                    to: p(3.0, 1.0),
                },
            ],
        )
        .unwrap();
        let mut next = 0u64;
        let plain = IdentifiedPath::assign(data, &mut || {
            next += 1;
            StableId::new_v7(next, 0, next)
        });
        let plain_bytes = plain.encode();
        let points = plain.point_ids();
        let moded = plain
            .clone()
            .with_handle_modes([(points[1], HandleMode::Mirrored)])
            .unwrap();
        let bytes = moded.encode();
        assert_eq!(&bytes[..plain_bytes.len()], plain_bytes.as_slice());
        assert_eq!(bytes.len(), plain_bytes.len() + 4 + 17);
        assert_eq!(IdentifiedPath::decode(&bytes).unwrap(), moded);
        assert_eq!(moded.handle_mode(points[1]), HandleMode::Mirrored);
        assert_eq!(moded.handle_mode(points[0]), HandleMode::Independent);
        // 없는 point, 기본값, 알 수 없는 tag, 빈 절은 거부합니다.
        assert!(plain
            .clone()
            .with_handle_modes([(
                PathPointId(StableId::new_v7(99, 0, 99)),
                HandleMode::Aligned
            )])
            .is_err());
        assert!(plain
            .clone()
            .with_handle_modes([(points[0], HandleMode::Independent)])
            .is_err());
        let mut bad_tag = bytes.clone();
        *bad_tag.last_mut().unwrap() = 7;
        assert!(IdentifiedPath::decode(&bad_tag).is_err());
        let mut empty_section = plain_bytes.clone();
        empty_section.extend_from_slice(&0u32.to_le_bytes());
        assert!(IdentifiedPath::decode(&empty_section).is_err());
        for cut in plain_bytes.len() + 1..bytes.len() {
            assert!(IdentifiedPath::decode(&bytes[..cut]).is_err(), "절단 {cut}");
        }
    }
}
