# 작업 보고서: IV.6.1 clone request + 민감 URL

- WBS: IV.6.1
- 작업명: `CloneRequest` 정의 + Git URL을 로그·telemetry 민감값으로 취급
- 상태: 완료 (`[x]`)
- 선행: IV.3.4, II.3.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/clone.rs` (신규: `SensitiveUrl`, `CloneRequest`, redaction·노출 테스트)
- `crates/project/src/lib.rs` (`pub mod clone;`)
- `docs/task-reports/IV.6.1.md`(신규), 계획서(IV.6.1 `[ ]`→`[x]`)

## 구현 요약

- `CloneRequest { url: SensitiveUrl, parent_dir, folder_name }` + `target_path()`.
- `SensitiveUrl`: Git URL을 **민감값**으로 취급합니다. `Debug`/`Display`는 원문을 노출하지 않고
  **host 힌트만** 보여줍니다(`[redacted Git URL @ <host>]`). 자격증명(userinfo)·경로는 절대
  보이지 않습니다. 원문은 `expose()`로만 접근하며 실제 git 실행 시에만 사용합니다.
- host 추출은 scheme·userinfo·경로를 제거하고 host만 남깁니다(`https://`, scp형 `git@host:` 모두).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (전체) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |

| 검사 | 결과 |
|---|---|
| 다양한 URL 형식에서 host 추출 | 통과 |
| **Debug/Display/redacted가 토큰·비밀 경로·원문 미노출**(host만) | 통과 |
| **CloneRequest Debug가 URL 자격증명·경로 미노출** | 통과 |
| `expose()`는 git 실행용 원문 반환 | 통과 |

## 차이(작업 지시와 다른 점)

- redaction은 host 힌트를 남깁니다(디버깅 편의). 자격증명·경로·원문은 어떤 포맷에서도 노출되지
  않음을 테스트로 보장합니다.

## 남은 위험 / 한계

- 실제 `git clone`(임시 폴더 → rename), NotAStellarProject 거부, ProjectProbe 통과는
  IV.6.2~IV.6.4에서 추가합니다.
