//! 캔버스 탐색 입력: 확대 제스처(phase 포함)·smart magnify·스크롤·Hand·Zoom 도구 (XII.10.36, ADR-0117 결정 8).
//!
//! 셸 입력 변환 결과가 runtime에 들어오는 모양입니다. 좌표는 캔버스 화면 왼쪽 위 원점 pt, 길이는 pt입니다. 셸
//! `StellarMacEvent`·FFI `InputEventKind`의 magnify 변환과 version 올림은 FFI를 더하지 않는 이번 방향 때문에 셸 재설계
//! 때 합니다(ADR-0117 결정 8.1). 그때 FFI 변환 결과를 이 타입으로 옮깁니다.
//!
//! 값 검사([`NavigationInput::validate`])는 비유한 좌표·배율과 `magnification <= -1`(배율 0 이하가 되는 값)을
//! 거부합니다.

use crate::canvas::tool::Modifiers;

/// 연속 제스처 phase(`NSEvent.phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GesturePhase {
    /// 시작.
    Began,
    /// 진행.
    Changed,
    /// 끝.
    Ended,
    /// 취소.
    Cancelled,
}

/// 스크롤 관성 phase(`NSEvent.momentumPhase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MomentumPhase {
    /// 관성 event가 아님.
    None,
    /// 관성 시작.
    Began,
    /// 관성 진행.
    Changed,
    /// 관성 끝.
    Ended,
}

/// 캔버스 탐색 입력 하나.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NavigationInput {
    /// 트랙패드 핀치(`NSEvent.magnification`). 배율을 `1 + magnification`배 합니다.
    Magnify {
        /// phase.
        phase: GesturePhase,
        /// 이 event의 배율 변화.
        magnification: f64,
        /// 커서 화면 위치.
        location: [f64; 2],
    },
    /// 두 손가락 두 번 두드리기(smart magnify).
    SmartMagnify {
        /// 커서 화면 위치.
        location: [f64; 2],
    },
    /// 스크롤. `delta`는 내용이 움직이는 방향 양수 pt입니다(마우스 휠 줄 단위는 셸이 pt로 바꿈).
    Scroll {
        /// 이동량(pt).
        delta: [f64; 2],
        /// 트랙패드 같은 정밀 스크롤인지.
        precise: bool,
        /// 제스처 phase(정밀 스크롤만, 마우스 휠은 `None`).
        phase: Option<GesturePhase>,
        /// 관성 phase.
        momentum: MomentumPhase,
        /// 수식키.
        modifiers: Modifiers,
        /// 커서 화면 위치.
        location: [f64; 2],
    },
    /// Hand 도구 또는 Space 드래그.
    HandDrag {
        /// phase.
        phase: GesturePhase,
        /// 커서 화면 위치.
        location: [f64; 2],
    },
    /// Zoom 도구 클릭. Option이면 축소.
    ZoomClick {
        /// 커서 화면 위치.
        location: [f64; 2],
        /// 축소(Option).
        zoom_out: bool,
    },
    /// Zoom 도구 영역 드래그를 놓음.
    ZoomArea {
        /// 누른 화면 점.
        from: [f64; 2],
        /// 놓은 화면 점.
        to: [f64; 2],
    },
}

/// 입력 값 오류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationInputError {
    /// 좌표·이동량·배율이 유한하지 않습니다.
    NonFinite,
    /// 배율 변화가 -1 이하입니다.
    Magnification,
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

impl NavigationInput {
    /// 값을 검사합니다.
    pub fn validate(&self) -> Result<(), NavigationInputError> {
        let ok = match *self {
            NavigationInput::Magnify {
                magnification,
                location,
                ..
            } => {
                if !finite(&[magnification, location[0], location[1]]) {
                    return Err(NavigationInputError::NonFinite);
                }
                if magnification <= -1.0 {
                    return Err(NavigationInputError::Magnification);
                }
                true
            }
            NavigationInput::SmartMagnify { location }
            | NavigationInput::HandDrag { location, .. }
            | NavigationInput::ZoomClick { location, .. } => finite(&location),
            NavigationInput::Scroll {
                delta, location, ..
            } => finite(&[delta[0], delta[1], location[0], location[1]]),
            NavigationInput::ZoomArea { from, to } => finite(&[from[0], from[1], to[0], to[1]]),
        };
        if ok {
            Ok(())
        } else {
            Err(NavigationInputError::NonFinite)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnify_and_smart_magnify_kinds_validate() {
        let pinch = |m: f64| NavigationInput::Magnify {
            phase: GesturePhase::Changed,
            magnification: m,
            location: [10.0, 20.0],
        };
        assert_eq!(pinch(0.25).validate(), Ok(()));
        assert_eq!(pinch(-0.5).validate(), Ok(()));
        assert_eq!(
            pinch(-1.0).validate(),
            Err(NavigationInputError::Magnification)
        );
        assert_eq!(
            pinch(f64::NAN).validate(),
            Err(NavigationInputError::NonFinite)
        );
        assert_eq!(
            NavigationInput::SmartMagnify {
                location: [f64::INFINITY, 0.0]
            }
            .validate(),
            Err(NavigationInputError::NonFinite)
        );
        for phase in [
            GesturePhase::Began,
            GesturePhase::Changed,
            GesturePhase::Ended,
            GesturePhase::Cancelled,
        ] {
            let input = NavigationInput::Magnify {
                phase,
                magnification: 0.0,
                location: [0.0, 0.0],
            };
            assert_eq!(input.validate(), Ok(()));
        }
        let scroll = NavigationInput::Scroll {
            delta: [1.0, f64::NAN],
            precise: true,
            phase: Some(GesturePhase::Changed),
            momentum: MomentumPhase::None,
            modifiers: Modifiers::NONE,
            location: [0.0, 0.0],
        };
        assert_eq!(scroll.validate(), Err(NavigationInputError::NonFinite));
    }
}
