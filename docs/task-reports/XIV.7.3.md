# 작업 보고서 XIV.7.3 선택 Unstage의 실제 Git 적용

## WBS

- WBS: XIV.7.3
- 작업명: 승인한 Unstage 후보를 실제 index에 기록
- 상태: `[x]`
- 선행: XIV.7.2, XIV.6.6

## 변경 파일

- `crates/runtime/src/staging.rs`와 `crates/ops/src/candidate.rs`에서 기존 inverse·dependency 검사와 index 적용·재검증·조건부 복원을 연결했습니다.
- `crates/runtime/src/staging/tests.rs`가 실제 Git 결과를 검사합니다.
- 처음 구현은 macOS Git 패널의 이름 행에 Unstage를 연결했습니다. 2026-11-15 검토에서 이름 전용 Git 패널 ABI를 삭제했습니다(ADR-0028). 앱 연결은 XIV.13.1·XIV.13.2의 범용 Git 패널 API에서 다시 합니다.

## 환경과 증거

- 환경: `DEV-MAC`
- 구현 커밋: `bb90f21d`

## 테스트

DEV-MAC에서 시스템 Git과 임시 저장소를 사용했습니다. 두 속성을 함께 Stage하고 작업 파일을 추가 편집한 뒤 이름만 Unstage했습니다. 재열린 index에는 HEAD 이름과 staged 잠금이 남고 다른 staged 파일도 보존됩니다. 잘못된 미리보기와 이동한 HEAD를 거부하고 index 바이트가 유지되는지 확인했습니다. Working Tree의 generation 포인터가 해석되지 않는 경우에도 해당 파일에 쓰지 않습니다.

## 명령과 종료 코드

| 명령 | 종료 코드 | 결과 |
|---|---:|---|
| `RUST_TEST_THREADS=1 cargo test --locked --offline -p stellar-runtime --lib staging::` | 0 | 9개 통과 |
| `cargo clippy --locked --offline -p stellar-runtime -p stellar-ffi --all-targets -- -D warnings` | 0 | 경고 없음 |
| `cargo fmt --all -- --check`, `git diff --check` | 0 | 통과 |

## 성능

대규모 저장소에서의 응답 시간은 측정하지 않았습니다. Unstage 준비 시 HEAD와 Index를 열며, 적용 전후에는 기준 HEAD를 다시 확인합니다.

## 차이

앱 연결은 없습니다. runtime 함수(`prepare_unstage_candidate`, `apply_stage_candidate`)만 제공합니다.

## 남은 위험

- 제품 UI 연결은 없습니다. 후보 준비는 Working Tree 밖의 별도 임시 폴더를 사용합니다. unborn HEAD는 진단합니다.
- 실제 index 쓰기는 기존 macOS 적용 범위와 512바이트 입력 한도를 따릅니다. 지원 범위 밖의 문서·필수 의존성·Git 형식에는 기존 거부 정책을 적용합니다.
- 새 generation의 의미 상태를 기준으로 복원합니다. 기존 Git에 남은 immutable object·manifest를 보존하므로, 의미 변경이 사라진 뒤에도 일반 Git diff에 generation 파일이 남을 수 있습니다. 파일 단위 전체 Unstage는 별도 동작입니다.
- HEAD와 index는 서로 다른 Git 자원입니다. 적용 전후의 HEAD 관찰과 기존 index 소유 확인·조건부 복원을 사용합니다. 외부 writer와의 원자적 다중 ref 트랜잭션은 보장하지 않습니다.
