# 작업 보고서: VI.1.5 열린 문서 수 제한

- WBS: VI.1.5
- 작업명: 최대 10개 열린 문서 제한(config), 초과 시 자동 닫기 금지
- 상태: 완료 (`[x]`)
- 선행: VI.1.4

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `config/document-limits.yaml`(신규) — `max_open_documents: 10`.
- `crates/document/src/open_limits.rs`(신규) — `OpenDocumentLimits`(엄격 config
  파서), `open_with_limit`, `TooManyOpenDocuments`.
- `crates/document/src/lib.rs` — 모듈 등록.
- `docs/task-reports/VI.1.5.md`(신규), 계획서(VI.1.5 `[ ]`→`[x]`).

## 구현 요약

- **config 기반 제한**: 수치는 `config/document-limits.yaml`에서 읽습니다
  (V.7.1과 같은 한 줄 `key: value` 엄격 파서 — 0·비정수·누락·unknown key 거부).
- **`open_with_limit`**: 새 문서가 제한을 초과하면 `TooManyOpenDocuments`
  (사용자 안내 메시지 포함)를 반환하고 **기존 문서를 하나도 닫지 않습니다**.
  이미 열린 문서는 제한 도달 상태에서도 기존 handle을 반환합니다(새 자원 없음).
  닫으면 용량이 회복됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 29 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 10개 성공 + 11번째 명확한 오류 + 기존 문서 유지(`ten_documents_succeed_and_eleventh_fails_clearly`) | 통과 |
| 제한 도달 시 중복 open 허용, close 후 용량 회복, malformed config 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 앱 셸이 config 로드 실패 시의 기본값 정책(fail-closed vs 내장 기본)을
  정해야 합니다. 현재 API는 실패를 그대로 보고합니다.
