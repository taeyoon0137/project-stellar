//! stellar-git-porcelain — System Git porcelain.
//!
//! document를 직접 수정하지 않습니다. Git 상태는 [`SnapshotSource`] 원천별 snapshot(파일 목록·
//! blob·materialize된 임시 프로젝트 디렉터리)으로만 제공합니다.

/// Git executable 탐색 (IV.3.1).
pub mod discovery;

/// Git 버전 probe (IV.3.2).
pub mod version;

/// 보안 Git 실행기 (IV.3.3).
pub mod runner;

/// Git 명령 취소와 프로세스 tree 종료 (XIV.1.4).
pub mod cancel;

/// Git 명령 결과 (IV.3.4).
pub mod command;

/// Git 출력 디코딩 (XIV.1.5).
pub mod decode;

/// Git 설치 상태 머신 (IV.3.5).
pub mod install_state;

/// macOS Git 설치 어댑터 (IV.3.6).
pub mod install_macos;

/// Windows Git 설치 어댑터 (IV.3.7).
pub mod install_windows;

/// 설치 후 재탐지·재probe (IV.3.8).
pub mod reprobe;

/// 저장소 porcelain 연산 (IV.4.4~).
pub mod repo;

/// 프로젝트 신뢰 gate (XIV.1.1).
pub mod trust;

/// 저장소 위치 판별 (XIV.1.2).
pub mod location;

/// Git capability probe (XIV.1.3).
pub mod capability;

/// 커밋 실행 (XIV.8.2~).
pub mod commit;

/// commit identity 해석(IV.4.5·XIV.8.3 공통).
pub mod identity;

/// Git object ID·file mode 값 타입(XIV.2.1~).
pub mod object;

/// 저장소 연산 공통 실행·오류(XIV.2.1~).
pub mod ops;

/// `git status --porcelain=v2 -z --branch` 해석(XIV.2.1~).
pub mod status;

/// `RepositoryStatus` 저장소 요약 상태(XIV.2.2).
pub mod repo_status;

/// Stellar 프로젝트 파일 분류와 raw Git status 분리(XIV.4.4).
pub mod classify;

/// Remote 목록·연결·push(XIV.9.*).
pub mod remote;

/// Git LFS 설치 감지 (XIII.7.1).
pub mod lfs;

/// 테스트 공용 도구(테스트 빌드 전용).
#[cfg(test)]
mod test_support;

/// Git snapshot 원천과 provider(XIV.2.3~).
pub mod snapshot;

pub use snapshot::SnapshotSource;
