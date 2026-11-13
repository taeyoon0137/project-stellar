//! Path command (VI.6).
//!
//! 저장 path command는 **MoveTo, LineTo, CubicTo, Close 네 가지뿐**입니다.
//! Quadratic 곡선은 import 단계에서 [`quadratic_to_cubic`]으로 정확히 같은
//! 곡선의 Cubic으로 정규화합니다 — 문서 모델에는 Quadratic이 존재하지 않습니다.

use std::fmt;

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
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::NonFinite => f.write_str("path 좌표가 유한하지 않습니다"),
            PathError::UnknownCommand { tag } => write!(f, "알 수 없는 path command tag: {tag}"),
            PathError::Malformed { reason } => write!(f, "Path binary 형식 위반: {reason}"),
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
}
