//! stellar-terminal — 내장 터미널 세션 코어(Phase XVIII, ADR-0019). 의존: core-types.
//!
//! 운영체제 셸을 호스팅하는 터미널 세션의 **코어**만 둡니다(UI는 앱 셸). 터미널은 Git이나 AI
//! 로직을 재구현하지 않습니다(기술 아키텍처 32).
//!
//! - [`id`]: 세션 메모리 전용 식별자.
//! - [`profile`]: argv 배열 명령·환경·cwd를 묶은 profile.
//! - [`env`]: child 환경 변수 허용 목록.
//! - [`cwd`]: 작업 디렉터리 선택.
//! - [`state`]: child process 상태.
//! - [`registry`]: 프로젝트 창별 다중 세션 registry.
//!
//! **저장하지 않는 상태**: 세션·profile 복사본·출력·프로세스 상태는 세션 메모리에만 있습니다.
//! 프로젝트 저장과 복구 snapshot에 넣지 않으며(기술 아키텍처 22.3 "복구하지 않는 대상: 터미널
//! 프로세스와 출력", ADR-0019 결정 2), 이 crate는 저장소·프로젝트·문서 crate와 직렬화 crate에
//! 의존하지 않습니다(테스트로 고정).

pub mod cwd;
pub mod env;
pub mod id;
pub mod profile;
pub mod registry;
pub mod state;

#[cfg(test)]
mod tests {
    /// 터미널 상태가 저장 경로에 닿을 수 없도록 의존성을 고정합니다.
    #[test]
    fn terminal_state_has_no_persistence_dependency() {
        let manifest = include_str!("../Cargo.toml");
        let dependencies = manifest
            .split("[dependencies]")
            .nth(1)
            .and_then(|rest| rest.split("\n[").next())
            .unwrap_or("");
        let names: Vec<&str> = dependencies
            .lines()
            .filter_map(|line| line.split('=').next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.starts_with('#'))
            .collect();
        assert_eq!(names, ["stellar-core-types"]);
        for forbidden in [
            "stellar-storage",
            "stellar-project",
            "stellar-document",
            "serde",
        ] {
            assert!(!manifest.contains(forbidden), "{forbidden}");
        }
    }
}
