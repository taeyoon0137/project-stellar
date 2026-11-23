//! Frame이 소유하는 눈금자 가이드와 레이아웃 그리드 값 (XII.10.18~XII.10.20, ADR-0117 결정 2).
//!
//! 가이드와 레이아웃 그리드는 문서 데이터입니다. 소유자는 Frame node이고 좌표는 그 Frame의 local CSS px입니다.
//! View root Frame의 가이드는 캔버스 가이드, 최상위 Frame의 가이드는 그 Frame을 따라 움직이는 가이드입니다.
//!
//! - [`RulerGuide`]: 축 하나의 선(`x` 세로선, `y` 가로선).
//! - [`LayoutGrid`]: 열·행·정사각 칸 그리드. 자식 배치(layout 입력)에 들어가지 않는 설계 기준입니다(ADR-0074).
//! - [`NodeGuides`]: Frame 하나의 기록. 편집은 [`GuideEdit`]이고 [`NodeGuides::apply`]가 역 편집을 돌려줍니다.
//! - canonical bytes([`NodeGuides::encode`], record v1)와 텍스트 원본 표기([`NodeGuides::to_text`]).
//!
//! 이 module은 값과 계산을 둡니다. 문서 모델·subtree chunk 확장 절·validator 연결은 [`persist`](ADR-0126)입니다.
//! command(`SetNodeGuides`)는 commands crate, 텍스트 원본 `guides` 목록은 project crate, Semantic Diff 투영은
//! semantic-diff crate가 이 module의 [`NodeGuides::encode`]·[`NodeGuides::lines`]를 그대로 씁니다.

/// 문서 모델·subtree 확장 절·validator 연결 (ADR-0126).
pub mod persist;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use stellar_core_types::ids::StableId;

use crate::css::grammar::{LENGTH_NON_NEGATIVE, LENGTH_SIGNED};
use crate::css::value::{format_length, parse, CssValue};

/// owner 하나의 가이드 상한.
pub const MAX_GUIDES: usize = 1_024;
/// owner 하나의 레이아웃 그리드 상한.
pub const MAX_GRIDS: usize = 16;
/// 열·행 그리드 트랙 수 상한.
pub const MAX_COUNT: u32 = 1_000;
/// 좌표·길이 절댓값 상한(px).
pub const MAX_COORD: f32 = 1.0e7;
/// 정사각 그리드 한 축의 트랙 상한. 넘으면 앞쪽 트랙만 냅니다.
pub const MAX_SQUARE_TRACKS: usize = 10_000;
/// canonical record 형식 번호.
pub const RECORD_VERSION: u8 = 1;

/// 가이드·그리드 값 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuideError {
    /// 좌표·길이가 유한하지 않거나 상한을 넘습니다.
    InvalidNumber(&'static str),
    /// 트랙 수가 0이거나 상한을 넘습니다.
    InvalidCount,
    /// 가이드·그리드 수가 상한을 넘습니다.
    TooMany(&'static str),
    /// 같은 id의 가이드가 이미 있습니다.
    DuplicateGuide(StableId),
    /// id에 해당하는 가이드가 없습니다.
    UnknownGuide(StableId),
    /// NIL id.
    NilId,
    /// bytes·텍스트 형식이 맞지 않습니다.
    Malformed(&'static str),
}

impl std::fmt::Display for GuideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuideError::InvalidNumber(what) => write!(f, "{what} 값이 유효하지 않습니다"),
            GuideError::InvalidCount => {
                write!(f, "그리드 트랙 수는 1~{MAX_COUNT}입니다")
            }
            GuideError::TooMany(what) => write!(f, "{what} 수가 상한을 넘습니다"),
            GuideError::DuplicateGuide(id) => write!(f, "가이드 {id}가 이미 있습니다"),
            GuideError::UnknownGuide(id) => write!(f, "가이드 {id}가 없습니다"),
            GuideError::NilId => f.write_str("NIL id는 쓸 수 없습니다"),
            GuideError::Malformed(reason) => write!(f, "가이드 기록 형식 오류: {reason}"),
        }
    }
}

impl std::error::Error for GuideError {}

fn check(value: f32, what: &'static str) -> Result<f32, GuideError> {
    if value.is_finite() && value.abs() <= MAX_COORD {
        // -0.0을 0.0으로 정규화합니다(같은 값은 같은 bytes).
        Ok(value + 0.0)
    } else {
        Err(GuideError::InvalidNumber(what))
    }
}

fn check_non_negative(value: f32, what: &'static str) -> Result<f32, GuideError> {
    let value = check(value, what)?;
    if value < 0.0 {
        return Err(GuideError::InvalidNumber(what));
    }
    Ok(value)
}

/// 가이드 축.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GuideAxis {
    /// x 좌표의 세로선.
    X,
    /// y 좌표의 가로선.
    Y,
}

impl GuideAxis {
    /// 좌표 index(x = 0, y = 1).
    pub const fn index(self) -> usize {
        match self {
            GuideAxis::X => 0,
            GuideAxis::Y => 1,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            GuideAxis::X => "x",
            GuideAxis::Y => "y",
        }
    }

    fn from_name(name: &str) -> Option<GuideAxis> {
        match name {
            "x" => Some(GuideAxis::X),
            "y" => Some(GuideAxis::Y),
            _ => None,
        }
    }
}

/// 눈금자 가이드 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RulerGuide {
    /// 영속 id(Semantic Diff·Merge가 가이드를 이 값으로 구분합니다).
    pub id: StableId,
    /// 축.
    pub axis: GuideAxis,
    /// 소유 Frame local 좌표(px).
    pub position: f32,
}

impl RulerGuide {
    /// 값을 검사해 가이드를 만듭니다.
    pub fn new(id: StableId, axis: GuideAxis, position: f32) -> Result<RulerGuide, GuideError> {
        if id == StableId::NIL {
            return Err(GuideError::NilId);
        }
        Ok(RulerGuide {
            id,
            axis,
            position: check(position, "가이드 위치")?,
        })
    }
}

/// 레이아웃 그리드 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKind {
    /// 세로 열(x 축 트랙).
    Columns,
    /// 가로 행(y 축 트랙).
    Rows,
    /// 정사각 칸(두 축, 한 변 = `size`).
    Square,
}

impl GridKind {
    const fn name(self) -> &'static str {
        match self {
            GridKind::Columns => "columns",
            GridKind::Rows => "rows",
            GridKind::Square => "square",
        }
    }

    fn from_name(name: &str) -> Option<GridKind> {
        match name {
            "columns" => Some(GridKind::Columns),
            "rows" => Some(GridKind::Rows),
            "square" => Some(GridKind::Square),
            _ => None,
        }
    }

    const fn tag(self) -> u8 {
        match self {
            GridKind::Columns => 0,
            GridKind::Rows => 1,
            GridKind::Square => 2,
        }
    }
}

/// 트랙 크기.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackSize {
    /// 남는 길이를 나눠 늘림(`auto`).
    Auto,
    /// 고정 px.
    Px(f32),
}

/// 트랙 정렬(CSS `justify-content`·`align-content` 키워드).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridAlign {
    /// `stretch`.
    Stretch,
    /// `start`.
    Start,
    /// `center`.
    Center,
    /// `end`.
    End,
}

impl GridAlign {
    const ALL: [GridAlign; 4] = [
        GridAlign::Stretch,
        GridAlign::Start,
        GridAlign::Center,
        GridAlign::End,
    ];

    const fn name(self) -> &'static str {
        match self {
            GridAlign::Stretch => "stretch",
            GridAlign::Start => "start",
            GridAlign::Center => "center",
            GridAlign::End => "end",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            GridAlign::Stretch => 0,
            GridAlign::Start => 1,
            GridAlign::Center => 2,
            GridAlign::End => 3,
        }
    }
}

/// Frame 레이아웃 그리드 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutGrid {
    /// 종류.
    pub kind: GridKind,
    /// 열·행 트랙 수(정사각 그리드는 쓰지 않으며 0).
    pub count: u32,
    /// 트랙 크기(정사각 그리드는 한 변 px, `Auto` 불가).
    pub size: TrackSize,
    /// 트랙 사이 px.
    pub gap: f32,
    /// 정렬 기준 가장자리에서의 바깥 여백 px(`center`는 쓰지 않음).
    pub margin: f32,
    /// 트랙 정렬.
    pub align: GridAlign,
    /// 표시 여부(스냅은 표시와 무관).
    pub visible: bool,
}

/// 한 축의 트랙 `(시작, 끝)` 목록(px, 오름차순).
pub type Tracks = Vec<(f32, f32)>;

impl LayoutGrid {
    /// `count`열 `stretch` 열 그리드(가장 흔한 12열 설계 기준).
    pub fn columns(count: u32, gap: f32, margin: f32) -> Result<LayoutGrid, GuideError> {
        LayoutGrid {
            kind: GridKind::Columns,
            count,
            size: TrackSize::Auto,
            gap,
            margin,
            align: GridAlign::Stretch,
            visible: true,
        }
        .validated()
    }

    /// 값을 검사하고 정규화한 사본.
    pub fn validated(self) -> Result<LayoutGrid, GuideError> {
        let gap = check_non_negative(self.gap, "그리드 gap")?;
        let margin = check_non_negative(self.margin, "그리드 margin")?;
        let size = match self.size {
            TrackSize::Auto if self.kind == GridKind::Square => {
                return Err(GuideError::InvalidNumber("정사각 그리드 size"))
            }
            TrackSize::Auto => TrackSize::Auto,
            TrackSize::Px(px) => {
                let px = check_non_negative(px, "그리드 size")?;
                if px <= 0.0 {
                    return Err(GuideError::InvalidNumber("그리드 size"));
                }
                TrackSize::Px(px)
            }
        };
        let count = match self.kind {
            GridKind::Square => 0,
            _ if (1..=MAX_COUNT).contains(&self.count) => self.count,
            _ => return Err(GuideError::InvalidCount),
        };
        Ok(LayoutGrid {
            count,
            size,
            gap,
            margin,
            ..self
        })
    }

    /// 길이 `length`(px)인 축 하나의 열·행 트랙.
    ///
    /// - `auto` 크기: 정렬과 관계없이 늘림입니다. 양쪽 `margin`을 뺀 길이에서 `gap`을 빼고 `count`로 나눕니다(음수면 0).
    /// - 고정 크기: `start`·`stretch`는 `margin` 뒤부터, `end`는 끝에서 `margin` 앞까지, `center`는 가운데에 놓습니다.
    pub fn line_tracks(&self, length: f32) -> Tracks {
        if !(length.is_finite() && length >= 0.0) || self.kind == GridKind::Square {
            return Vec::new();
        }
        let n = self.count.max(1);
        let gaps = self.gap * (n - 1) as f32;
        let (size, offset) = match self.size {
            TrackSize::Auto => (
                ((length - 2.0 * self.margin - gaps) / n as f32).max(0.0),
                self.margin,
            ),
            TrackSize::Px(size) => {
                let total = size * n as f32 + gaps;
                let offset = match self.align {
                    GridAlign::Stretch | GridAlign::Start => self.margin,
                    GridAlign::Center => (length - total) / 2.0,
                    GridAlign::End => length - self.margin - total,
                };
                (size, offset)
            }
        };
        (0..n)
            .map(|i| {
                let start = offset + i as f32 * (size + self.gap);
                (start, start + size)
            })
            .collect()
    }

    /// `width × height` Frame의 x 트랙과 y 트랙.
    pub fn tracks(&self, width: f32, height: f32) -> [Tracks; 2] {
        match self.kind {
            GridKind::Columns => [self.line_tracks(width), Vec::new()],
            GridKind::Rows => [Vec::new(), self.line_tracks(height)],
            GridKind::Square => {
                let TrackSize::Px(size) = self.size else {
                    return [Vec::new(), Vec::new()];
                };
                let square = |length: f32| -> Tracks {
                    if !(length.is_finite() && length > 0.0) {
                        return Vec::new();
                    }
                    let step = size + self.gap;
                    (0..MAX_SQUARE_TRACKS)
                        .map(|i| i as f32 * step)
                        .take_while(|start| *start < length)
                        .map(|start| (start, (start + size).min(length)))
                        .collect()
                };
                [square(width), square(height)]
            }
        }
    }
}

/// 가이드·그리드 편집 하나. [`NodeGuides::apply`]가 역 편집을 돌려줍니다.
#[derive(Debug, Clone, PartialEq)]
pub enum GuideEdit {
    /// 가이드를 더합니다.
    Add(RulerGuide),
    /// 가이드 위치를 바꿉니다.
    Move {
        /// 대상 가이드.
        id: StableId,
        /// 새 위치(소유자 local px).
        position: f32,
    },
    /// 가이드를 지웁니다.
    Remove {
        /// 대상 가이드.
        id: StableId,
    },
    /// 레이아웃 그리드 목록 전체를 바꿉니다.
    SetGrids(Vec<LayoutGrid>),
}

/// Frame 하나의 가이드·그리드 기록.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeGuides {
    owner: StableId,
    guides: BTreeMap<StableId, RulerGuide>,
    grids: Vec<LayoutGrid>,
}

impl NodeGuides {
    /// 빈 기록.
    pub fn new(owner: StableId) -> NodeGuides {
        NodeGuides {
            owner,
            guides: BTreeMap::new(),
            grids: Vec::new(),
        }
    }

    /// 소유 Frame.
    pub fn owner(&self) -> StableId {
        self.owner
    }

    /// 가이드(id 오름차순).
    pub fn guides(&self) -> impl Iterator<Item = &RulerGuide> {
        self.guides.values()
    }

    /// id의 가이드.
    pub fn guide(&self, id: StableId) -> Option<&RulerGuide> {
        self.guides.get(&id)
    }

    /// 레이아웃 그리드(그리는 순서).
    pub fn grids(&self) -> &[LayoutGrid] {
        &self.grids
    }

    /// 가이드도 그리드도 없으면 참(저장할 기록이 없음).
    pub fn is_empty(&self) -> bool {
        self.guides.is_empty() && self.grids.is_empty()
    }

    /// 축의 가이드 위치(오름차순, 같은 값 한 번).
    pub fn positions(&self, axis: GuideAxis) -> Vec<f32> {
        let mut out: Vec<f32> = self
            .guides
            .values()
            .filter(|g| g.axis == axis)
            .map(|g| g.position)
            .collect();
        out.sort_by(f32::total_cmp);
        out.dedup_by(|a, b| a.to_bits() == b.to_bits());
        out
    }

    /// 편집을 적용하고 역 편집을 돌려줍니다. 실패하면 기록은 그대로입니다.
    pub fn apply(&mut self, edit: &GuideEdit) -> Result<GuideEdit, GuideError> {
        match edit {
            GuideEdit::Add(guide) => {
                let guide = RulerGuide::new(guide.id, guide.axis, guide.position)?;
                if self.guides.contains_key(&guide.id) {
                    return Err(GuideError::DuplicateGuide(guide.id));
                }
                if self.guides.len() >= MAX_GUIDES {
                    return Err(GuideError::TooMany("가이드"));
                }
                self.guides.insert(guide.id, guide);
                Ok(GuideEdit::Remove { id: guide.id })
            }
            GuideEdit::Move { id, position } => {
                let position = check(*position, "가이드 위치")?;
                let guide = self
                    .guides
                    .get_mut(id)
                    .ok_or(GuideError::UnknownGuide(*id))?;
                let before = std::mem::replace(&mut guide.position, position);
                Ok(GuideEdit::Move {
                    id: *id,
                    position: before,
                })
            }
            GuideEdit::Remove { id } => {
                let guide = self
                    .guides
                    .remove(id)
                    .ok_or(GuideError::UnknownGuide(*id))?;
                Ok(GuideEdit::Add(guide))
            }
            GuideEdit::SetGrids(grids) => {
                if grids.len() > MAX_GRIDS {
                    return Err(GuideError::TooMany("레이아웃 그리드"));
                }
                let grids = grids
                    .iter()
                    .map(|g| g.validated())
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(GuideEdit::SetGrids(std::mem::replace(
                    &mut self.grids,
                    grids,
                )))
            }
        }
    }

    /// canonical bytes: version, owner, 가이드 수(u16)와 (id, 축, 위치 f32 LE)*, 그리드 수(u8)와
    /// (종류, count u32, size 표지·값, gap, margin, 정렬, 표시)*.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![RECORD_VERSION];
        out.extend_from_slice(&self.owner.to_bytes());
        out.extend_from_slice(&(self.guides.len() as u16).to_le_bytes());
        for guide in self.guides.values() {
            out.extend_from_slice(&guide.id.to_bytes());
            out.push(guide.axis.index() as u8);
            out.extend_from_slice(&guide.position.to_le_bytes());
        }
        put_grids(&mut out, &self.grids);
        out
    }

    /// [`NodeGuides::encode`]의 역. 정규형이 아닌 bytes(정렬되지 않은 id, 남는 bytes, 범위 밖 값)는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<NodeGuides, GuideError> {
        let mut input = Cursor { bytes, at: 0 };
        if input.u8()? != RECORD_VERSION {
            return Err(GuideError::Malformed("지원하지 않는 version"));
        }
        let owner = input.id()?;
        let mut record = NodeGuides::new(owner);
        let count = u16::from_le_bytes(input.array()?) as usize;
        if count > MAX_GUIDES {
            return Err(GuideError::TooMany("가이드"));
        }
        let mut last: Option<StableId> = None;
        for _ in 0..count {
            let id = input.id()?;
            if last.is_some_and(|prev| prev >= id) {
                return Err(GuideError::Malformed("가이드 id 순서"));
            }
            last = Some(id);
            let axis = match input.u8()? {
                0 => GuideAxis::X,
                1 => GuideAxis::Y,
                _ => return Err(GuideError::Malformed("가이드 축")),
            };
            let position = f32::from_le_bytes(input.array()?);
            record.apply(&GuideEdit::Add(RulerGuide::new(id, axis, position)?))?;
        }
        let list = read_grids(&mut input)?;
        record.apply(&GuideEdit::SetGrids(list))?;
        if input.at != bytes.len() {
            return Err(GuideError::Malformed("남는 bytes"));
        }
        if record.encode() != bytes {
            return Err(GuideError::Malformed("정규형이 아닌 값"));
        }
        Ok(record)
    }

    /// 텍스트 원본 표기. 한 줄에 하나이고 가이드(id 순) 다음 그리드(그리는 순)입니다.
    ///
    /// ```text
    /// guide x 120px 0195c530-3acd-7bc1-8414-c213f61cb965
    /// grid columns 12 auto gap 20px margin 32px stretch
    /// grid square - 8px gap 0px margin 0px start hidden
    /// ```
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in self.lines() {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// [`NodeGuides::to_text`]의 줄 목록(줄바꿈 없음). 텍스트 원본 `guides` 목록과 semantic snapshot 값이 이 줄을
    /// 그대로 씁니다.
    pub fn lines(&self) -> Vec<String> {
        self.guides
            .values()
            .map(RulerGuide::to_line)
            .chain(self.grids.iter().map(LayoutGrid::to_line))
            .collect()
    }

    /// [`NodeGuides::to_text`]의 역. 길이는 CSS 길이 parser(`css::value`)로 읽습니다.
    pub fn from_text(owner: StableId, text: &str) -> Result<NodeGuides, GuideError> {
        NodeGuides::from_lines(owner, text.lines().filter(|l| !l.trim().is_empty()))
    }

    /// 줄 목록에서 기록을 만듭니다([`NodeGuides::lines`]의 역). 가이드 줄은 순서와 무관하고 그리드 줄은 쓴 순서가
    /// 그리는 순서입니다.
    pub fn from_lines<'a>(
        owner: StableId,
        lines: impl IntoIterator<Item = &'a str>,
    ) -> Result<NodeGuides, GuideError> {
        let mut record = NodeGuides::new(owner);
        let mut grids = Vec::new();
        for line in lines {
            match GuideLine::parse(line)? {
                GuideLine::Guide(guide) => {
                    record.apply(&GuideEdit::Add(guide))?;
                }
                GuideLine::Grid(grid) => grids.push(grid),
            }
        }
        record.apply(&GuideEdit::SetGrids(grids))?;
        Ok(record)
    }
}

impl RulerGuide {
    /// 텍스트 원본 한 줄(`guide x 120px <id>`, 줄바꿈 없음).
    pub fn to_line(&self) -> String {
        format!(
            "guide {} {} {}",
            self.axis.name(),
            format_length(self.position),
            self.id
        )
    }
}

impl LayoutGrid {
    /// 텍스트 원본 한 줄(`grid columns 12 auto gap 20px margin 32px stretch`, 줄바꿈 없음).
    pub fn to_line(&self) -> String {
        let count = match self.kind {
            GridKind::Square => "-".to_string(),
            _ => self.count.to_string(),
        };
        let size = match self.size {
            TrackSize::Auto => "auto".to_string(),
            TrackSize::Px(px) => format_length(px),
        };
        let mut out = String::new();
        let _ = write!(
            out,
            "grid {} {count} {size} gap {} margin {} {}",
            self.kind.name(),
            format_length(self.gap),
            format_length(self.margin),
            self.align.name()
        );
        if !self.visible {
            out.push_str(" hidden");
        }
        out
    }
}

/// 텍스트 원본 한 줄의 내용.
#[derive(Debug, Clone, PartialEq)]
pub enum GuideLine {
    /// `guide …` 줄.
    Guide(RulerGuide),
    /// `grid …` 줄(검사·정규화한 값).
    Grid(LayoutGrid),
}

impl GuideLine {
    /// 한 줄을 읽습니다([`RulerGuide::to_line`]·[`LayoutGrid::to_line`]의 역).
    pub fn parse(line: &str) -> Result<GuideLine, GuideError> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["guide", axis, position, id] => {
                let axis = GuideAxis::from_name(axis).ok_or(GuideError::Malformed("가이드 축"))?;
                let position = length(position, true)?;
                let id: StableId = id.parse().map_err(|_| GuideError::Malformed("가이드 id"))?;
                Ok(GuideLine::Guide(RulerGuide::new(id, axis, position)?))
            }
            ["grid", kind, count, size, "gap", gap, "margin", margin, align, rest @ ..] => {
                let kind = GridKind::from_name(kind).ok_or(GuideError::Malformed("그리드 종류"))?;
                let count = match (kind, *count) {
                    (GridKind::Square, "-") => 0,
                    (GridKind::Square, _) => return Err(GuideError::Malformed("그리드 count")),
                    (_, count) => count
                        .parse::<u32>()
                        .map_err(|_| GuideError::Malformed("그리드 count"))?,
                };
                let size = match *size {
                    "auto" => TrackSize::Auto,
                    px => TrackSize::Px(length(px, false)?),
                };
                let align = GridAlign::ALL
                    .into_iter()
                    .find(|a| a.name() == *align)
                    .ok_or(GuideError::Malformed("그리드 정렬"))?;
                let visible = match rest {
                    [] => true,
                    ["hidden"] => false,
                    _ => return Err(GuideError::Malformed("그리드 줄 끝")),
                };
                LayoutGrid {
                    kind,
                    count,
                    size,
                    gap: length(gap, false)?,
                    margin: length(margin, false)?,
                    align,
                    visible,
                }
                .validated()
                .map(GuideLine::Grid)
            }
            _ => Err(GuideError::Malformed("알 수 없는 줄")),
        }
    }
}

fn put_grids(out: &mut Vec<u8>, grids: &[LayoutGrid]) {
    out.push(grids.len() as u8);
    for grid in grids {
        out.push(grid.kind.tag());
        out.extend_from_slice(&grid.count.to_le_bytes());
        match grid.size {
            TrackSize::Auto => {
                out.push(0);
                out.extend_from_slice(&0f32.to_le_bytes());
            }
            TrackSize::Px(px) => {
                out.push(1);
                out.extend_from_slice(&px.to_le_bytes());
            }
        }
        out.extend_from_slice(&grid.gap.to_le_bytes());
        out.extend_from_slice(&grid.margin.to_le_bytes());
        out.push(grid.align.tag());
        out.push(u8::from(grid.visible));
    }
}

fn read_grids(input: &mut Cursor<'_>) -> Result<Vec<LayoutGrid>, GuideError> {
    let grids = input.u8()? as usize;
    let mut list = Vec::with_capacity(grids);
    for _ in 0..grids {
        let kind = match input.u8()? {
            0 => GridKind::Columns,
            1 => GridKind::Rows,
            2 => GridKind::Square,
            _ => return Err(GuideError::Malformed("그리드 종류")),
        };
        let count = u32::from_le_bytes(input.array()?);
        let size_tag = input.u8()?;
        let size_value = f32::from_le_bytes(input.array()?);
        let size = match size_tag {
            0 if size_value.to_bits() == 0 => TrackSize::Auto,
            1 => TrackSize::Px(size_value),
            _ => return Err(GuideError::Malformed("그리드 size")),
        };
        let gap = f32::from_le_bytes(input.array()?);
        let margin = f32::from_le_bytes(input.array()?);
        let align = *GridAlign::ALL
            .get(input.u8()? as usize)
            .ok_or(GuideError::Malformed("그리드 정렬"))?;
        let visible = match input.u8()? {
            0 => false,
            1 => true,
            _ => return Err(GuideError::Malformed("그리드 표시")),
        };
        list.push(LayoutGrid {
            kind,
            count,
            size,
            gap,
            margin,
            align,
            visible,
        });
    }
    Ok(list)
}

const EDIT_ADD: u8 = 1;
const EDIT_MOVE: u8 = 2;
const EDIT_REMOVE: u8 = 3;
const EDIT_SET_GRIDS: u8 = 4;

impl GuideEdit {
    /// canonical bytes(command payload, ADR-0126): tag `u8` 뒤에 Add(id, 축, 위치 f32 LE)·Move(id, 위치)·Remove(id)·
    /// SetGrids([`NodeGuides::encode`]와 같은 그리드 목록 형식).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            GuideEdit::Add(guide) => {
                out.push(EDIT_ADD);
                out.extend_from_slice(&guide.id.to_bytes());
                out.push(guide.axis.index() as u8);
                out.extend_from_slice(&guide.position.to_le_bytes());
            }
            GuideEdit::Move { id, position } => {
                out.push(EDIT_MOVE);
                out.extend_from_slice(&id.to_bytes());
                out.extend_from_slice(&position.to_le_bytes());
            }
            GuideEdit::Remove { id } => {
                out.push(EDIT_REMOVE);
                out.extend_from_slice(&id.to_bytes());
            }
            GuideEdit::SetGrids(grids) => {
                out.push(EDIT_SET_GRIDS);
                put_grids(&mut out, grids);
            }
        }
        out
    }

    /// [`GuideEdit::encode`]의 역. 값 검사는 적용([`NodeGuides::apply`]) 때 합니다. 남는 bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<GuideEdit, GuideError> {
        let mut input = Cursor { bytes, at: 0 };
        let edit = match input.u8()? {
            EDIT_ADD => {
                let id = input.id()?;
                let axis = match input.u8()? {
                    0 => GuideAxis::X,
                    1 => GuideAxis::Y,
                    _ => return Err(GuideError::Malformed("가이드 축")),
                };
                let position = f32::from_le_bytes(input.array()?);
                GuideEdit::Add(RulerGuide { id, axis, position })
            }
            EDIT_MOVE => GuideEdit::Move {
                id: input.id()?,
                position: f32::from_le_bytes(input.array()?),
            },
            EDIT_REMOVE => GuideEdit::Remove { id: input.id()? },
            EDIT_SET_GRIDS => GuideEdit::SetGrids(read_grids(&mut input)?),
            _ => return Err(GuideError::Malformed("가이드 편집 tag")),
        };
        if input.at != bytes.len() {
            return Err(GuideError::Malformed("남는 bytes"));
        }
        Ok(edit)
    }
}

fn length(text: &str, signed: bool) -> Result<f32, GuideError> {
    let grammar = if signed {
        LENGTH_SIGNED
    } else {
        LENGTH_NON_NEGATIVE
    };
    match parse(grammar, text) {
        Ok(CssValue::Length(px)) => Ok(px),
        _ => Err(GuideError::Malformed("CSS 길이")),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N], GuideError> {
        let end = self
            .at
            .checked_add(N)
            .ok_or(GuideError::Malformed("길이"))?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(GuideError::Malformed("bytes가 짧습니다"))?;
        self.at = end;
        Ok(slice.try_into().expect("길이 N"))
    }

    fn u8(&mut self) -> Result<u8, GuideError> {
        Ok(self.array::<1>()?[0])
    }

    fn id(&mut self) -> Result<StableId, GuideError> {
        let id = StableId::from_bytes(self.array()?);
        if id == StableId::NIL {
            return Err(GuideError::NilId);
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u128) -> StableId {
        StableId::from_bytes(n.to_be_bytes())
    }

    fn sample() -> NodeGuides {
        let mut record = NodeGuides::new(sid(10));
        for (id, axis, at) in [(3, GuideAxis::X, 120.0), (1, GuideAxis::Y, -40.5)] {
            record
                .apply(&GuideEdit::Add(RulerGuide::new(sid(id), axis, at).unwrap()))
                .unwrap();
        }
        let square = LayoutGrid {
            kind: GridKind::Square,
            count: 0,
            size: TrackSize::Px(8.0),
            gap: 0.0,
            margin: 0.0,
            align: GridAlign::Start,
            visible: false,
        };
        record
            .apply(&GuideEdit::SetGrids(vec![
                LayoutGrid::columns(12, 20.0, 32.0).unwrap(),
                square,
            ]))
            .unwrap();
        record
    }

    #[test]
    fn edits_return_exact_inverses() {
        let mut record = sample();
        let original = record.clone();
        let edits = [
            GuideEdit::Add(RulerGuide::new(sid(7), GuideAxis::X, 5.0).unwrap()),
            GuideEdit::Move {
                id: sid(3),
                position: 300.0,
            },
            GuideEdit::Remove { id: sid(1) },
            GuideEdit::SetGrids(Vec::new()),
        ];
        let mut inverses = Vec::new();
        for edit in &edits {
            inverses.push(record.apply(edit).unwrap());
        }
        assert_eq!(record.guide(sid(3)).unwrap().position, 300.0);
        assert!(record.guide(sid(1)).is_none());
        for inverse in inverses.iter().rev() {
            record.apply(inverse).unwrap();
        }
        assert_eq!(record, original);
    }

    #[test]
    fn invalid_edits_leave_the_record_unchanged() {
        let mut record = sample();
        let before = record.clone();
        let duplicate = GuideEdit::Add(RulerGuide::new(sid(3), GuideAxis::Y, 1.0).unwrap());
        assert_eq!(
            record.apply(&duplicate),
            Err(GuideError::DuplicateGuide(sid(3)))
        );
        assert!(record
            .apply(&GuideEdit::Move {
                id: sid(3),
                position: f32::NAN
            })
            .is_err());
        assert_eq!(
            record.apply(&GuideEdit::Remove { id: sid(99) }),
            Err(GuideError::UnknownGuide(sid(99)))
        );
        let bad = LayoutGrid {
            count: 0,
            ..LayoutGrid::columns(12, 0.0, 0.0).unwrap()
        };
        assert_eq!(
            record.apply(&GuideEdit::SetGrids(vec![bad])),
            Err(GuideError::InvalidCount)
        );
        assert!(RulerGuide::new(StableId::NIL, GuideAxis::X, 0.0).is_err());
        assert!(RulerGuide::new(sid(1), GuideAxis::X, 2.0e7).is_err());
        assert_eq!(record, before);
    }

    #[test]
    fn bytes_and_text_round_trip_canonically() {
        let record = sample();
        let bytes = record.encode();
        assert_eq!(NodeGuides::decode(&bytes).unwrap(), record);
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(NodeGuides::decode(&longer).is_err());
        assert!(NodeGuides::decode(&bytes[..bytes.len() - 1]).is_err());
        let text = record.to_text();
        assert_eq!(
            text,
            format!(
                "guide y -40.5px {}\nguide x 120px {}\ngrid columns 12 auto gap 20px margin 32px stretch\n\
                 grid square - 8px gap 0px margin 0px start hidden\n",
                sid(1),
                sid(3)
            )
        );
        assert_eq!(NodeGuides::from_text(sid(10), &text).unwrap(), record);
        assert!(NodeGuides::from_text(sid(10), "guide x 1em ").is_err());
        assert!(
            NodeGuides::from_text(sid(10), "grid columns 0 auto gap 0px margin 0px stretch")
                .is_err()
        );
    }

    #[test]
    fn twelve_column_tracks_fill_the_frame_between_margins() {
        // 1440px Frame, 12열, gap 24px, margin 120px: (1440 - 240 - 11*24) / 12 = 78px.
        let grid = LayoutGrid::columns(12, 24.0, 120.0).unwrap();
        let [x, y] = grid.tracks(1440.0, 900.0);
        assert!(y.is_empty());
        assert_eq!(x.len(), 12);
        assert_eq!(x[0], (120.0, 198.0));
        assert_eq!(x[1], (222.0, 300.0));
        assert_eq!(x[11].1, 1320.0);
    }

    #[test]
    fn fixed_tracks_follow_alignment() {
        let fixed = |align| LayoutGrid {
            kind: GridKind::Rows,
            count: 3,
            size: TrackSize::Px(100.0),
            gap: 10.0,
            margin: 20.0,
            align,
            visible: true,
        };
        let first = |align| fixed(align).tracks(500.0, 1000.0)[1][0];
        assert_eq!(first(GridAlign::Start), (20.0, 120.0));
        assert_eq!(first(GridAlign::Stretch), (20.0, 120.0));
        // 전체 3×100 + 2×10 = 320px.
        assert_eq!(first(GridAlign::Center), (340.0, 440.0));
        assert_eq!(first(GridAlign::End), (660.0, 760.0));
        let square = LayoutGrid {
            kind: GridKind::Square,
            count: 0,
            size: TrackSize::Px(8.0),
            gap: 0.0,
            margin: 0.0,
            align: GridAlign::Start,
            visible: true,
        };
        let [x, y] = square.tracks(20.0, 16.0);
        assert_eq!(x, vec![(0.0, 8.0), (8.0, 16.0), (16.0, 20.0)]);
        assert_eq!(y.len(), 2);
    }
}
