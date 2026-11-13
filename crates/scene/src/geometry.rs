//! Scene path geometry (XI.6.1).
//!
//! renderer가 tessellation 입력으로 받는 path geometry입니다. scene은 crate 의존 정책(II.2.8)상
//! document 타입을 import하지 않으므로, document path 모델(VI.6.1~VI.6.2)과 **같은 의미**를
//! renderer 쪽 입력 타입으로 독립 정의합니다.
//!
//! - command는 MoveTo, LineTo, CubicTo, Close 네 가지뿐입니다. Quadratic은 import 단계에서
//!   Cubic으로 정규화되어 들어옵니다.
//! - Shape node도 document 단계의 순수 평가(VI.5.6)로 path command가 된 뒤 Scene에 들어옵니다.
//!   따라서 Shape와 Path는 같은 [`PathGeometry`] 입력을 공유합니다.
//! - 모든 좌표는 [`Vec2`](finite, `-0.0`→`0.0` 정규화)라 [`PathGeometry::canonical_bytes`]가
//!   bit-stable합니다.
//! - command 목록은 `Arc`로 보관해 변경되지 않은 geometry를 snapshot 간에 구조적으로 공유합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::value::Vec2;

/// Scene path command.
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

/// 채우기 규칙. document path의 winding rule(VI.6.2)에 대응하며, Shape는 `NonZero`를 씁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FillRule {
    /// non-zero(기본).
    #[default]
    NonZero,
    /// even-odd.
    EvenOdd,
}

/// path geometry 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathGeometryError {
    /// subpath가 MoveTo 없이 시작합니다. index는 문제 command 위치입니다.
    MissingMoveTo {
        /// command index.
        index: usize,
    },
}

impl fmt::Display for PathGeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathGeometryError::MissingMoveTo { index } => {
                write!(f, "command {index}: subpath는 MoveTo로 시작해야 합니다")
            }
        }
    }
}

impl std::error::Error for PathGeometryError {}

/// 검증된 Scene path geometry.
///
/// 불변식: 비어 있지 않다면 첫 command는 MoveTo이고, Close 다음 command도 MoveTo여야
/// 합니다(암묵적 subpath 재시작 없음). 빈 geometry는 유효합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGeometry {
    commands: Arc<[PathCommand]>,
}

impl PathGeometry {
    /// subpath 규칙을 검증한 뒤 geometry를 만듭니다.
    pub fn new(commands: Vec<PathCommand>) -> Result<PathGeometry, PathGeometryError> {
        let mut in_subpath = false;
        for (index, command) in commands.iter().enumerate() {
            match command {
                PathCommand::MoveTo(_) => in_subpath = true,
                PathCommand::LineTo(_) | PathCommand::CubicTo { .. } => {
                    if !in_subpath {
                        return Err(PathGeometryError::MissingMoveTo { index });
                    }
                }
                PathCommand::Close => {
                    if !in_subpath {
                        return Err(PathGeometryError::MissingMoveTo { index });
                    }
                    in_subpath = false;
                }
            }
        }
        Ok(PathGeometry {
            commands: commands.into(),
        })
    }

    /// 빈 geometry.
    pub fn empty() -> PathGeometry {
        PathGeometry {
            commands: Arc::from(Vec::new()),
        }
    }

    /// command들.
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// command가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// canonical bytes: `u64` LE command 개수 + (tag `u8` + 좌표 `f32` LE `x, y`)*.
    ///
    /// tag는 MoveTo=1, LineTo=2, CubicTo=3(`c1, c2, to` 순), Close=4입니다. 같은 command
    /// 목록은 항상 같은 bytes가 되며, cache key(renderer-core)의 원본 데이터로 씁니다.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.commands.len() * 9);
        buf.extend_from_slice(&(self.commands.len() as u64).to_le_bytes());
        for command in self.commands.iter() {
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
}

fn put_point(buf: &mut Vec<u8>, p: Vec2) {
    buf.extend_from_slice(&p.x().to_le_bytes());
    buf.extend_from_slice(&p.y().to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    #[test]
    fn subpath_rules_are_enforced() {
        assert!(PathGeometry::new(Vec::new()).unwrap().is_empty());
        assert_eq!(
            PathGeometry::new(vec![PathCommand::LineTo(p(1.0, 1.0))]),
            Err(PathGeometryError::MissingMoveTo { index: 0 })
        );
        assert_eq!(
            PathGeometry::new(vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(1.0, 0.0)),
                PathCommand::Close,
                PathCommand::LineTo(p(2.0, 2.0)),
            ]),
            Err(PathGeometryError::MissingMoveTo { index: 3 })
        );
        assert_eq!(
            PathGeometry::new(vec![PathCommand::Close]),
            Err(PathGeometryError::MissingMoveTo { index: 0 })
        );
        // Close 뒤 새 MoveTo는 허용.
        assert!(PathGeometry::new(vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::Close,
            PathCommand::MoveTo(p(5.0, 5.0)),
            PathCommand::LineTo(p(6.0, 5.0)),
        ])
        .is_ok());
    }

    #[test]
    fn canonical_bytes_are_bit_stable() {
        let geometry = PathGeometry::new(vec![
            PathCommand::MoveTo(p(1.0, -0.0)),
            PathCommand::CubicTo {
                c1: p(0.5, 0.0),
                c2: p(0.0, 0.5),
                to: p(0.0, 1.0),
            },
            PathCommand::Close,
        ])
        .unwrap();
        let mut expected = Vec::new();
        expected.extend_from_slice(&3u64.to_le_bytes());
        expected.push(TAG_MOVE);
        expected.extend_from_slice(&1.0f32.to_le_bytes());
        // -0.0은 +0.0으로 정규화되어 bit 0으로 기록됩니다.
        expected.extend_from_slice(&[0, 0, 0, 0]);
        expected.push(TAG_CUBIC);
        for v in [0.5f32, 0.0, 0.0, 0.5, 0.0, 1.0] {
            expected.extend_from_slice(&v.to_le_bytes());
        }
        expected.push(TAG_CLOSE);
        assert_eq!(geometry.canonical_bytes(), expected);
        // 독립적으로 만든 동일 geometry는 같은 bytes.
        let again = PathGeometry::new(geometry.commands().to_vec()).unwrap();
        assert_eq!(again.canonical_bytes(), geometry.canonical_bytes());
        assert_eq!(
            PathGeometry::empty().canonical_bytes(),
            0u64.to_le_bytes().to_vec()
        );
    }
}
