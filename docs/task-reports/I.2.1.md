# 작업 보고서: I.2.1 TaskStatus 데이터 모델

- WBS: I.2.1
- 작업명: `TaskStatusRecord`, `TaskState`, `VerificationEnvironment`, `VerificationEvidence` 데이터 모델 정의
- 상태: 완료 (`[x]`)
- 선행: I.1.10 (`[~]` — CI 증거만 연기, 모델 정의는 의존하지 않음)

## 필수 실행 환경

- `DEV-MAC`.

## 변경 파일 목록

- `tools/xtask/src/lib.rs` (신규, 모듈 노출)
- `tools/xtask/src/task_status/mod.rs` (신규)
- `tools/xtask/src/task_status/model.rs` (신규, 데이터 모델 + 단위 테스트)
- `docs/task-reports/I.2.1.md` (신규)
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` (상태 열 갱신: I.2.1 `[ ]` → `[x]`)

## 구현 요약

- `TaskState` enum: `not_started`, `in_progress`, `reported_done`, `verified`, `blocked`, `cancelled` + `as_str`/`from_token`.
- `VerificationEnvironment` enum: 6개 코드(DEV-MAC … REF-WIN) + `as_str`/`from_token`.
- `VerificationEvidence` struct: environment, locator, artifact_sha256.
- `TaskStatusRecord` struct: id, title, status, owner, depends_on, required_environments, evidence, commit, report, verified_at + `new` 생성자.
- 표준 라이브러리만 사용. filesystem/YAML parser import 0(검증에서 확인).
- xtask를 lib+bin 구조로 전환해 모듈을 노출(pub 항목이라 dead_code 경고 없음). bin(`main.rs`)은 변경 없음.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p xtask -- --check` | 종료 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 종료 0 |
| `cargo test -p xtask --locked` | 4 passed |
| `model.rs` fs/yaml import 점검 | 없음(주석 언급만) |

## 남은 경고 수

- 0

## 작업 지시와 다른 점

- 모듈을 컴파일·테스트하기 위해 `lib.rs`와 `task_status/mod.rs`를 함께 추가했습니다(데이터 모델 노출 목적, 로직 없음).
