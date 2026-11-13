# ADR-0019: 로드맵 crate 추가(export, terminal)와 벤치마크 fixture crate

## Status

Accepted (2026-11-14). 제품 소유자가 기술 문서에 세부가 없는 설계 결정을 ADR 기록 조건으로 위임한 범위(2026-11-14)에서 정했습니다.

## Context

- 기술 아키텍처 §7의 저장소 구성에는 PNG·SVG 내보내기(Phase XX)와 내장 터미널(Phase XVIII)을 담을 crate가 없습니다. 두 Phase의 WBS 행은 별도 crate를 전제로 합니다.
- 성능 기준 fixture 생성기(XVI.3)는 §7의 `benchmarks/` 아래에 두게 되어 있지만 crate가 없습니다.
- 여러 작업이 동시에 진행되므로, workspace member와 crate 의존 정책을 한 번에 정해 두면 각 작업이 루트 `Cargo.toml`과 정책 파일을 따로 고치지 않아도 됩니다(계획 XVII.2 규칙 2).

## Decision

1. `crates/export`(`stellar-export`): 의존은 `core-types`, `scene`만 허용합니다. 문서 모델과 GPU 객체를 직접 다루지 않고 Scene Snapshot을 입력으로 받습니다(§8 상위 런타임 구조와 같은 방향).
2. `crates/terminal`(`stellar-terminal`): 의존은 `core-types`만 허용합니다. 세션·프로필·PTY 같은 코어만 두고 UI는 앱 셸에 둡니다. 프로젝트 저장과 복구 snapshot에는 터미널 상태를 넣지 않습니다(XVIII.1.1).
3. `benchmarks/fixtures`(`stellar-bench-fixtures`): 제품 crate가 아닌 개발용 crate입니다. 제품 crate는 이 crate에 의존하지 않습니다. crate 의존 정책 검사(`crates/*` 대상)의 범위 밖입니다.
4. 세 crate 모두 외부 의존성 없이 시작합니다. 외부 crate가 필요하면 `config/dependencies.yaml`과 별도 ADR로 추가합니다.

## Alternatives

- **기존 crate에 넣기**(내보내기를 `renderer-core`에, 터미널을 `project`에): 렌더러와 프로젝트 수명주기에 무관한 책임이 섞이고, 로드맵 기능을 초기 릴리스에서 빼기 어려워집니다. 채택하지 않습니다.
- **각 작업이 필요할 때 crate 생성**: 동시에 진행되는 작업들이 루트 `Cargo.toml`과 정책 파일을 함께 고쳐 충돌합니다. 채택하지 않습니다.

## Consequences

- 긍정: 내보내기·터미널·성능 fixture 작업을 다른 작업과 병렬로 진행할 수 있습니다.
- 부정: 빈 골격 crate가 한동안 workspace에 남아 빌드 시간이 조금 늘어납니다.

## Compatibility

파일 포맷, 공개 FFI, 저장 구조에 영향이 없습니다.

## Performance

핵심 입력·렌더링 경로에 영향이 없습니다. 벤치마크 fixture crate는 제품 빌드에 들어가지 않습니다.

## Security

터미널 crate는 외부 프로세스를 다루므로 이후 작업에서 신뢰 매트릭스(XVI.7)와 환경 변수 허용 목록 원칙을 따릅니다. 이 ADR은 crate 경계만 정합니다.

## Migration

없습니다.
