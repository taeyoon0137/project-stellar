# 작업 보고서: V.7.1 보존 generation 정책

- WBS: V.7.1
- 작업명: 보존 generation 정책 인터페이스 — 기본 current+직전 정상 세대+복구 필요 세대, 수치는 config
- 상태: 완료 (`[x]`)
- 선행: V.4.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `config/storage-gc.yaml`(신규) — `keep_recent_generations: 2`,
  `grace_period_seconds: 86400`.
- `crates/storage/src/gc/mod.rs`(신규), `crates/storage/src/gc/retention.rs`(신규) —
  `RetentionPolicy`, `RetentionInput`, `retained_generations`,
  `RetentionConfigError`.
- `crates/storage/src/lib.rs` — `pub mod gc;` 등록.
- `docs/task-reports/V.7.1.md`(신규), 계획서(V.7.1 `[ ]`→`[x]`).

## 구현 요약

- **정책 수치는 config에서**: `config/storage-gc.yaml`(한 줄 `key: value` 형식)을
  외부 의존성 없는 엄격 파서로 읽습니다 — 0·비정수·누락·알 수 없는 key 거부.
- **`retained_generations(policy, input)`**: ① current 세대는 항상 보존(손상이어도
  진단 자료) ② 정상 세대 중 최신 `keep_recent_generations`개 보존(기본 2 = current
  + 직전) ③ 복구에 필요한 세대(V.4.5 fallback) 보존.
- sweep 유예 시간(`grace_period_seconds`)은 V.7.3에서 사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 164 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 기본 정책 fixture: current+직전 보존(`default_keeps_current_and_previous`) | 통과 |
| 복구 세대 보존, 더 큰 keep 수, current 없음, malformed config 5종 거부 | 통과 |
| 저장소 config 파일 자체 파싱(`repo_config_parses_with_expected_defaults`) | 통과 |

## 차이(작업 지시와 다른 점)

- config 파일 위치를 명시하지 않은 계획에 대해 `config/storage-gc.yaml`을
  신설했습니다(performance-budgets.yaml은 ADR 보호 경로이자 성능 예산 전용).

## 남은 위험 / 한계

- reachable mark(V.7.2)와 sweep(V.7.3)이 이 정책의 소비자입니다.
