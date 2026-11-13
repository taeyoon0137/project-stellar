# 작업 보고서: XIV.10.1 MergeInput·MergeResult·SemanticConflict 모델

- WBS: XIV.10.1
- 작업명: `MergeInput { base, ours, theirs }`, `MergeResult`, `SemanticConflict` 모델 정의
- 상태: 완료 (`[x]`)
- 선행: XIV.3.1 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/model.rs`(신규) — `MergeInput`, `MergeResult`, `SemanticConflict`,
  `ConflictKind`, `ConflictTarget`, `SideState`, `MergeError`, canonical codec.
- `crates/semantic-merge/src/lib.rs` — module 선언.
- `crates/semantic-diff/src/codec.rs` — 위치·출처·선택 값·선택 id encode/decode helper 공개
  (semantic-merge는 semantic-diff에만 의존하므로 codec을 재사용).
- `docs/task-reports/XIV.10.1.md`(신규), 계획서(XIV.10.1 `[ ]`→`[x]`).

## 구현 요약

- **입력**: `MergeInput { base, ours, theirs }` — 세 `SemanticSnapshot`.
- **결과**: `MergeResult { merged, changes, conflicts }` — 자동 병합 candidate snapshot(conflict
  대상은 base 상태 유지), base → merged `SemanticDiff`, conflict 목록. `is_clean()`.
- **conflict**: `SemanticConflict { kind, provenance, object, target, base, ours, theirs }`.
  대상은 객체 전체·위치·속성 경로이고, 세 쪽 상태(`SideState`: 없음·객체·위치·속성 값)를 모두
  보존합니다. 종류 6개에 안정 code: property-value(1), delete-vs-modify(2), delete-vs-move(3),
  type-change-vs-value-change(4), conflicting-reorder(5), inconsistent-definition(6, 동일
  StableId 불일치 정의 — 아키텍처 §27.1).
- **canonical bytes**: `SMIN`/`SMRS` 헤더 + version, semantic-diff codec 재사용. decoder는
  truncated·trailing·magic·알 수 없는 tag·경로 root 불일치·변경 목록 순서 위반을 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-merge` | 4 passed, 0 failed |
| `cargo test -p stellar-semantic-diff` | 45 passed, 0 failed |
| `cargo fmt -p stellar-semantic-merge -- --check`, `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-merge -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| conflict 종류 code·이름 golden(`conflict_kind_codes_golden`) | 통과 |
| MergeResult(merged·changes·property/객체 conflict) round-trip, 빈 결과 round-trip, 모든 절단 입력 거부(`merge_result_round_trips`) | 통과 |
| MergeInput round-trip, trailing·magic 거부(`merge_input_round_trips`) | 통과 |
| 알 수 없는 kind·경로 root 불일치 거부(`malformed_conflict_is_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- XIV.10.7 전용 conflict(Property 삭제 대 override 추가, token 타입 대 값, drilling chain
  break)는 선행(IX.1.3·IX.4.6·VIII.5.5) 미완료로 이 모델에 넣지 않았습니다.

## 남은 위험 / 한계

- 형식은 비교·전달용이며 저장 포맷이 아닙니다(FFI 노출은 XIV.13.4에서 별도 결정).
