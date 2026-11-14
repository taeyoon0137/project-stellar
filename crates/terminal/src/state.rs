//! 터미널 child process 상태 (XVIII.1.1).
//!
//! 상태는 세션 메모리에만 있습니다. 창을 닫거나 앱이 다시 실행되면 터미널을 복원하지 않으므로
//! (기술 아키텍처 22.3, 32 "초기에는 세션 복원 없음") 상태를 저장하는 경로가 없습니다.

use std::fmt;

/// child(세션 leader)의 종료 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExitStatus {
    /// 정상 종료(exit code).
    Exited(i32),
    /// 신호로 종료(신호 번호).
    Signaled(i32),
    /// 끝났지만 상태를 알 수 없습니다(이 crate 밖에서 leader를 회수함, `ECHILD`).
    Unknown,
}

impl fmt::Display for TerminalExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminalExitStatus::Exited(code) => write!(f, "exit {code}"),
            TerminalExitStatus::Signaled(signal) => write!(f, "signal {signal}"),
            TerminalExitStatus::Unknown => f.write_str("unknown"),
        }
    }
}

/// 세션이 끝난 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationReason {
    /// child가 스스로 끝났거나 외부 신호로 끝났습니다.
    ChildExited,
    /// 사용자가 세션을 닫았습니다.
    Closed,
    /// 실행이 취소되었습니다.
    Cancelled,
    /// 창 또는 앱이 닫혔습니다.
    AppClosed,
}

impl TerminationReason {
    /// 고정 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            TerminationReason::ChildExited => "child-exited",
            TerminationReason::Closed => "closed",
            TerminationReason::Cancelled => "cancelled",
            TerminationReason::AppClosed => "app-closed",
        }
    }
}

/// 시작 실패 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFailure {
    /// PTY 생성 실패.
    Pty,
    /// child process 생성 실패.
    Spawn,
}

impl StartFailure {
    /// 고정 코드 문자열.
    pub fn code(self) -> &'static str {
        match self {
            StartFailure::Pty => "pty",
            StartFailure::Spawn => "spawn",
        }
    }
}

/// 터미널 child process 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalProcessState {
    /// 아직 시작하지 않았습니다.
    NotStarted,
    /// 실행 중(세션 leader pid).
    Running {
        /// 세션 leader pid.
        pid: u32,
    },
    /// 끝났습니다.
    Finished {
        /// 세션 leader 종료 상태.
        status: TerminalExitStatus,
        /// 끝난 이유.
        reason: TerminationReason,
        /// 정리 뒤에도 남은 세션 프로세스 수(정상은 0).
        remaining_processes: u32,
    },
    /// 시작하지 못했습니다.
    StartFailed {
        /// 실패 분류.
        failure: StartFailure,
    },
}

impl TerminalProcessState {
    /// 실행 중인지.
    pub fn is_running(&self) -> bool {
        matches!(self, TerminalProcessState::Running { .. })
    }

    /// 고정 코드 문자열.
    pub fn code(&self) -> &'static str {
        match self {
            TerminalProcessState::NotStarted => "not-started",
            TerminalProcessState::Running { .. } => "running",
            TerminalProcessState::Finished { .. } => "finished",
            TerminalProcessState::StartFailed { .. } => "start-failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_codes_are_stable() {
        let states = [
            TerminalProcessState::NotStarted,
            TerminalProcessState::Running { pid: 42 },
            TerminalProcessState::Finished {
                status: TerminalExitStatus::Exited(0),
                reason: TerminationReason::Closed,
                remaining_processes: 0,
            },
            TerminalProcessState::StartFailed {
                failure: StartFailure::Spawn,
            },
        ];
        let codes: Vec<&str> = states.iter().map(TerminalProcessState::code).collect();
        assert_eq!(
            codes,
            ["not-started", "running", "finished", "start-failed"]
        );
        assert!(states[1].is_running());
        assert!(!states[2].is_running());
        let reasons = [
            TerminationReason::ChildExited,
            TerminationReason::Closed,
            TerminationReason::Cancelled,
            TerminationReason::AppClosed,
        ];
        let reason_codes: Vec<&str> = reasons.iter().map(|r| r.code()).collect();
        assert_eq!(
            reason_codes,
            ["child-exited", "closed", "cancelled", "app-closed"]
        );
        assert_eq!(TerminalExitStatus::Signaled(9).to_string(), "signal 9");
        assert_eq!(TerminalExitStatus::Unknown.to_string(), "unknown");
        assert_eq!(StartFailure::Pty.code(), "pty");
    }
}
