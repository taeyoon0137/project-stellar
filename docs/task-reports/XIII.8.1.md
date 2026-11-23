# 작업 보고서: XIII.8.1 dirty revision 복구 snapshot의 debounce와 OS app data 기록

## WBS

- WBS: XIII.8.1
- 작업명: dirty revision 복구 snapshot의 debounce와 OS app data 기록
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록(ledger-refresh): 행 지시·검증 열을 코드와 테스트로 확인했고 선행이 모두 `[x]`입니다. 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 원장이 근거 커밋을 찾지 못했습니다(window-session-stale lane). 선행 XIII.1.3도 같은 판정으로 `[x]`입니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: V.9.2, XIII.1.3
- 결정: [ADR-0083](../adr/ADR-0083-window-session-stale-and-external-sync.md)
- lane: `window-session-stale`

## 변경 파일

- `crates/runtime/src/app/recovery.rs`: 복구본 저장소 root를 인자로 받는 내부 경로
- `crates/runtime/src/app/recovery/lifecycle_tests.rs`(신규)

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime --locked` | 0 — lib 239 passed·2 ignored, 통합 테스트 전부 통과 |
| `cargo clippy -p stellar-runtime -p stellar-project -p stellar-ops -p stellar-cli -p stellar-ffi --all-targets --locked -- -D warnings` | 0 — 경고 0 |
| `cargo xtask validate-adr --base d2bf97b3` | 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행(아래 테스트). lane 전체 `check-core` 결과는 lane 결과 보고에 적습니다.

## 테스트

| 테스트 | 결과 |
|---|---|
| `app::recovery::lifecycle_tests::checkpoint_is_debounced_into_app_data_and_never_into_the_project_folder` | 통과 — debounce 전 기록 0, 뒤에 app data에 1개, 프로젝트 폴더 전체 bytes 불변 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음). 판정 비용은 ADR-0083 Performance에 적었습니다.

## 차이

- 검증 열의 `git status`는 fixture의 `.git`이 실제 저장소가 아니라 프로젝트 폴더 전체 파일·bytes 비교로 대신했습니다(같은 조건보다 강함).

## 남은 위험

- debounce 1초·최대 5초는 writer.rs의 임시 값입니다(벤치마크 미확정).

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XIII.8.1)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `00c83c8b` Feat: 자동 저장과 복구 체크포인트가 같은 불변 snapshot을 공유 (XIII.8.2·XIII.8.1·XIII.8.4·XIII.8.7)
- `2152aa06` Fix: 복구 체크포인트 캡처를 편집마다 하지 않고 debounce 뒤 한 번 제출
