//! 구독 CLI 연결 상태 판정 (XXIX.2.5, ADR-0005 §4.1, ADR-0092 결정 3).
//!
//! 각 CLI 공식 문서가 제공하는 상태 명령만 실행하고(launch profile `status_command`), 판정은 profile의
//! [`StatusRule`]로 합니다.
//!
//! - Claude Code: `claude auth status`는 "로그인했으면 종료 코드 0, 아니면 1"로 문서화되어 있어 종료 코드만
//!   씁니다([`StatusRule::ExitCode`]).
//! - 출력은 읽지 않습니다(stdout 보관 상한 0). CLI가 보고한 계정 식별 정보가 Stellar 메모리·저장·로그·진단에
//!   들어오지 않습니다(XVI.8).
//! - CLI 자격 증명 저장 위치(설정 디렉터리, 인증 파일, Keychain)에 접근하지 않습니다.
//! - 판정할 수 없으면(실행 실패, timeout, signal 종료, 문서에 없는 종료 코드) 사용 불가입니다(fail closed).
//! - 판정은 프로세스를 실행하므로 UI·Document 스레드 밖에서 합니다([`spawn_probe`]). 판정 시점은 세션 실행
//!   전 확인([`crate::preflight`])과 설정 화면의 상태 새로 고침(XXIX.4.8)입니다. 결과는 캐시하지 않습니다.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::env::ChildEnv;
use crate::process::{run_capture, CaptureOutcome, ExitKind, ScratchDir};
use crate::profile::{LaunchProfile, StatusRule};
use crate::target::VerifiedTarget;

/// 연결 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// CLI가 로그인 상태라고 보고함.
    Connected,
    /// CLI가 로그인하지 않았다고 보고함(사용자가 CLI 공식 절차로 로그인해야 함).
    NotConnected,
    /// 판정할 수 없음(사용 불가로 처리).
    Undeterminable(&'static str),
}

impl ConnectionState {
    /// 세션에 쓸 수 있는지. [`ConnectionState::Connected`]만 참입니다.
    pub fn is_usable(self) -> bool {
        self == ConnectionState::Connected
    }
}

/// 상태 명령 결과를 판정합니다.
pub fn judge(rule: StatusRule, outcome: &CaptureOutcome) -> ConnectionState {
    match rule {
        StatusRule::ExitCode => {
            if outcome.timed_out {
                return ConnectionState::Undeterminable("timeout");
            }
            match outcome.exit {
                Some(ExitKind::Code(0)) => ConnectionState::Connected,
                Some(ExitKind::Code(1)) => ConnectionState::NotConnected,
                Some(ExitKind::Code(_)) => ConnectionState::Undeterminable("exit-code"),
                Some(ExitKind::Signal(_)) => ConnectionState::Undeterminable("signal"),
                None => ConnectionState::Undeterminable("no-exit"),
            }
        }
    }
}

/// 연결 상태를 판정합니다(호출 스레드에서 실행, 오래 걸릴 수 있음).
pub fn probe_connection(
    target: &VerifiedTarget,
    profile: &LaunchProfile,
    env: &ChildEnv,
    scratch_base: &std::path::Path,
) -> ConnectionState {
    let Ok(scratch) = ScratchDir::create(scratch_base) else {
        return ConnectionState::Undeterminable("scratch");
    };
    if !target.still_unchanged() {
        return ConnectionState::Undeterminable("target-changed");
    }
    match run_capture(
        target.path(),
        &profile.status_command,
        env,
        &scratch.work_dir(),
        Duration::from_millis(profile.timeouts.status_ms),
        0,
    ) {
        Ok(outcome) => judge(profile.status_rule, &outcome),
        Err(_) => ConnectionState::Undeterminable("spawn"),
    }
}

/// UI·Document 스레드 밖의 worker 스레드에서 판정하고 결과를 channel로 돌려줍니다.
pub fn spawn_probe(
    target: VerifiedTarget,
    profile: LaunchProfile,
    env: ChildEnv,
    scratch_base: PathBuf,
) -> mpsc::Receiver<ConnectionState> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("stellar-agent-status".into())
        .spawn(move || {
            let _ = tx.send(probe_connection(&target, &profile, &env, &scratch_base));
        })
        .map(drop)
        .unwrap_or(());
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(exit: Option<ExitKind>, timed_out: bool) -> CaptureOutcome {
        CaptureOutcome {
            exit,
            timed_out,
            stdout: Vec::new(),
        }
    }

    #[test]
    fn exit_code_rule_fails_closed() {
        let r = StatusRule::ExitCode;
        assert_eq!(
            judge(r, &outcome(Some(ExitKind::Code(0)), false)),
            ConnectionState::Connected
        );
        assert_eq!(
            judge(r, &outcome(Some(ExitKind::Code(1)), false)),
            ConnectionState::NotConnected
        );
        for o in [
            outcome(Some(ExitKind::Code(2)), false),
            outcome(Some(ExitKind::Signal(9)), false),
            outcome(None, true),
            outcome(None, false),
        ] {
            assert!(!judge(r, &o).is_usable(), "{o:?}");
        }
    }
}
