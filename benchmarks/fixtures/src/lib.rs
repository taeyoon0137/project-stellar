//! stellar-bench-fixtures — 성능 기준 fixture 생성기(XVI.3, ADR-0019).
//!
//! 제품 crate가 아니며 제품 코드가 의존하지 않습니다. fixture 파일을 저장소에 넣지 않고
//! `(generator version, fixture 종류, seed, 규모 매개변수)`로 필요할 때 다시 만듭니다. 문서는
//! 실제 문서 모델(`stellar_document`)로 조립해 전체 validator를 통과시키고, 실제 serializer
//! (`stellar_project::document_io`)로 chunk model을 만든 뒤 그 결과로 content hash를 계산합니다.
//!
//! - [`synthetic`]: 1만·20만·프로젝트 100만 node 규모 fixture(XVI.3.1).
//! - [`limits`]: 가시 node 2만·컴포넌트 인스턴스 5만·중첩 64 fixture(XVI.3.2).
//! - [`workload`]: text·image·effect·constraint-heavy fixture와 계측값(XVI.3.3).
//! - [`asset_provider`]: 20 GiB sparse synthetic asset provider와 기준 장비 manifest(XVI.3.4).
//! - [`split`]: fixture의 chunk split 계획 요약(V.6.4 planner 연결).
//! - [`digest`]: fixture content hash.
//!
//! 대형 tier는 테스트에서 `#[ignore]`로 두고(필요할 때 `--ignored`로 실행), CI에는 같은 생성
//! 경로를 쓰는 축소 변형을 둡니다. `cargo run -p stellar-bench-fixtures --release -- <fixture>`로
//! 요약(JSON)을 출력할 수 있습니다.

/// fixture generator version. 같은 명세의 출력이 바뀌는 변경(생성 규칙, PRNG 사용 순서 등)은
/// 이 값을 올려야 합니다. content hash 입력에 들어갑니다.
pub const GENERATOR_VERSION: u32 = 1;

/// 결정론적 PRNG.
pub mod rng;

/// 결정론적 StableId 발급기.
pub mod ids;

/// fixture 생성 오류.
pub mod error;

/// fixture content hash.
pub mod digest;

/// 생성기 공통 부품(payload factory, 문서 마무리).
pub mod build;

/// 규모별 synthetic fixture (XVI.3.1).
pub mod synthetic;

/// chunk split 계획 요약 (V.6.4).
pub mod split;

/// 규모 상한 fixture: 가시 2만, 인스턴스 5만, 중첩 64 (XVI.3.2).
pub mod limits;

/// 병목 특성별 workload fixture (XVI.3.3).
pub mod workload;

/// sparse synthetic asset provider와 기준 장비 manifest (XVI.3.4).
pub mod asset_provider;

/// on-demand 생성 CLI 인자 해석과 실행.
pub mod cli;
