# 작업 보고서: IV.2.4 NotAStellarProject 거부

- WBS: IV.2.4
- 작업명: 일반 Git repo지만 `stellar.yaml`이 없으면 `NotAStellarProject`로 안내 거부
- 상태: 완료 (`[x]`)
- 선행: IV.2.2(root 검증기)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/root.rs` (`OpenError`, `classify_project_open` 추가 + 테스트, 모듈 doc 갱신)
- `docs/task-reports/IV.2.4.md`(신규), 계획서(IV.2.4 `[ ]`→`[x]`)

## 구현 요약

- `classify_project_open(path)`는 IV.2.2 primitive(매니페스트·`.git` 존재)를 재사용해 열기
  가능성을 분류합니다:
  - 매니페스트 + `.git` → `Ok(GitLink)`.
  - `.git`은 있으나 매니페스트 없음 → `Err(OpenError::NotAStellarProject)`(안내 메시지에
    "자동 변환 미지원" 명시).
  - `.git` 없음 → `Err(OpenError::NotARepository)`.
- **자동 변환 코드를 만들지 않습니다**: 함수는 파일을 만들거나 고치지 않고 분류만 합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| fixture | 결과 |
|---|---|
| Git repo(.git 디렉터리/gitfile)이지만 stellar.yaml 없음 | `Err(NotAStellarProject)` |
| 유효한 Stellar 프로젝트 | `Ok(Directory)` |
| `.git` 없음(빈/매니페스트만) | `Err(NotARepository)` |
| 분류 후 stellar.yaml 미생성(자동 변환 없음) 확인 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. 안내 거부와 무변환을 모두 충족합니다.

## 남은 위험 / 한계

- 이 함수는 경로 존재·종류만 보고 매니페스트 내용은 파싱하지 않습니다(파싱·신뢰는 IV.5).
- IV.2 섹션(프로젝트 루트 검증) 전체가 완료되었습니다.
