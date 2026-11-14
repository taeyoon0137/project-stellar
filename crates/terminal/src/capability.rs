//! 터미널 시작 capability — 신뢰된 프로젝트에서만 시작 (XVIII.1.4).
//!
//! 기술 아키텍처 32 "프로젝트 신뢰 전 실행 금지"와 37.1 "터미널 제한"에 따라, 신뢰되지 않은 프로젝트
//! 창에서는 터미널 child를 **만들지 않습니다**.
//!
//! 신뢰 상태의 원본은 project crate의 trust store(IV.5.2)와 capability matrix(XVI.7.1의
//! `ProjectCapability::Terminal` 행: 비신뢰 금지·신뢰 허용)입니다. 이 crate는 project crate에 의존할
//! 수 없으므로(ADR-0019), 호출자가 matrix 판정 결과를 [`TerminalTrust`]로 바꿔 공급하고, 신뢰된
//! 경우에만 [`TerminalCapability`] token을 얻습니다.
//!
//! - token은 프로젝트 창([`WindowSessionId`]) 하나에 묶입니다. 다른 창의 registry에서는 쓸 수
//!   없습니다(다른 창의 프로젝트는 신뢰 상태가 다를 수 있음).
//! - token은 `Clone`·`Copy`가 아니고 시작 한 번에 **소비**됩니다. 호출자는 시작할 때마다 현재 신뢰
//!   상태로 새로 받아야 하므로, 신뢰를 철회한 뒤 예전 판정을 재사용할 수 없습니다.
//! - registry는 token을 다른 어떤 검사(cwd·셸·환경)보다 먼저 확인하고, 실패하면 실행기를 호출하지
//!   않습니다([`TerminalRegistry::start`](crate::registry::TerminalRegistry::start)).
//! - token은 실행 요청([`LaunchRequest`](crate::launch::LaunchRequest))을 만들 때 그 안으로
//!   소비됩니다. 요청 생성자는 crate 밖에 공개하지 않으므로, token 없이 실행기
//!   ([`TerminalLauncher`](crate::launch::TerminalLauncher), macOS `PtyLauncher`)를 직접 불러 child를
//!   만드는 경로가 없습니다(`launch` 모듈 문서의 `compile_fail` 예).
//!
//! **연결 지점**: [`TerminalCapability::grant`]에 넘기는 [`TerminalTrust`]는 이 crate가 검증할 수
//! 없습니다. 의존 정책상(ADR-0019, `tools/xtask/src/crate_graph.rs`) 이 crate는 project crate를 볼 수
//! 없기 때문입니다. 이 crate를 앱 셸에 연결하는 계층이 XVI.7.1 matrix 판정
//! (`check_capability(ProjectCapability::Terminal, trust)`)만으로 이 값을 정해야 하며, 다른 근거로
//! `Trusted`를 만들지 않습니다. 이 crate가 보장하는 범위는 "token 없이는 child를 만들 수 없다"까지입니다.

use std::fmt;

use stellar_core_types::ids::WindowSessionId;

/// 호출자가 프로젝트 신뢰 capability matrix에서 얻은 터미널 허용 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalTrust {
    /// 신뢰된 프로젝트(matrix `terminal` 행이 허용).
    Trusted,
    /// 신뢰되지 않은 프로젝트(matrix `terminal` 행이 금지, 기본값).
    NotTrusted,
}

/// 신뢰되지 않은 프로젝트에서 터미널 시작을 요청했습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalNotTrusted;

impl fmt::Display for TerminalNotTrusted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("신뢰되지 않은 프로젝트에서는 터미널을 시작할 수 없습니다")
    }
}

impl std::error::Error for TerminalNotTrusted {}

/// 신뢰된 프로젝트 창 하나에서 터미널 세션 하나를 시작할 수 있다는 증명.
#[derive(Debug)]
pub struct TerminalCapability {
    window: WindowSessionId,
}

impl TerminalCapability {
    /// 신뢰된 경우에만 token을 만듭니다.
    pub fn grant(
        window: WindowSessionId,
        trust: TerminalTrust,
    ) -> Result<TerminalCapability, TerminalNotTrusted> {
        match trust {
            TerminalTrust::Trusted => Ok(TerminalCapability { window }),
            TerminalTrust::NotTrusted => Err(TerminalNotTrusted),
        }
    }

    /// token이 묶인 프로젝트 창.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::window;

    #[test]
    fn capability_is_granted_only_for_trusted_projects() {
        assert_eq!(
            TerminalCapability::grant(window(1), TerminalTrust::NotTrusted).unwrap_err(),
            TerminalNotTrusted
        );
        let capability = TerminalCapability::grant(window(1), TerminalTrust::Trusted).unwrap();
        assert_eq!(capability.window(), window(1));
        assert!(TerminalNotTrusted.to_string().contains("신뢰"));
    }
}
