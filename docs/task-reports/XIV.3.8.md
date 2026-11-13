# 작업 보고서: XIV.3.8 semantic change 결정론적 정렬

- WBS: XIV.3.8
- 작업명: semantic changes를 deterministic sort key(document, object path, property path, change kind)로 정렬
- 상태: 완료 (`[x]`)
- 선행: XIV.3.2 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/codec.rs`(신규) — snapshot·객체·값·변경 canonical encode/decode, `Reader`, `DecodeError`.
- `crates/semantic-diff/src/diff.rs` — `canonical_order`, `SemanticDiff`(정렬·bytes·검증 decode), `diff`가 `SemanticDiff` 반환.
- `crates/semantic-diff/src/lib.rs` — `codec` module 선언.
- `docs/task-reports/XIV.3.8.md`(신규), 계획서(XIV.3.8 `[ ]`→`[x]`).

## 구현 요약

- **정렬 key**: (문서 — 프로젝트 범위 `None` 먼저, 객체 StableId, 속성 경로 — 객체 단위 변경
  `None` 먼저, 변경 종류 code), 남은 동률은 변경 canonical bytes. 전순서라 같은 변경 집합은
  만드는 방법과 무관하게 같은 순서입니다. "object path"는 diff 양쪽에서 바뀔 수 있는 tree 경로
  대신 불변 StableId로 해석했습니다.
- **canonical bytes**: `SemanticDiff::to_canonical_bytes`(헤더 `SDIF` + version 1 + 변경 목록,
  LE). 같은 형식으로 snapshot(`SSNP`)·객체·값·변경도 encode/decode합니다(XIV.10.1 round-trip이
  재사용). decoder는 truncated·trailing·알 수 없는 tag·비정규 float(`-0.0`·NaN)·잘못된
  OrderKey/TypedValue/UTF-8·순서 위반을 구조화 오류로 거부하고, 개수 필드로 과할당하지 않으며
  재귀가 없습니다. `from_canonical_bytes`는 canonical 순서가 아닌 목록을 거부합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 40 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 30 seed: 레코드 순서 셔플·builder 호출 순서 변경 snapshot의 diff bytes가 기준과 동일, 변경 목록을 뒤집고 회전해 `SemanticDiff::new`로 다시 만들어도 동일(`diff_bytes_ignore_input_order`) | 통과 |
| 정렬 key 순서: 프로젝트 token → 같은 객체 위치 변경 → 속성 경로 오름차순 → 다음 객체(`canonical_order_key_is_document_object_path_kind`) | 통과 |
| diff bytes round-trip, 역순 목록 `NonCanonicalOrder`, 모든 절단 입력 거부(`diff_bytes_round_trip_and_reject_non_canonical_order`) | 통과 |
| snapshot bytes round-trip 20 seed, 절단·trailing·magic·version 거부, 비정규 값 거부(`codec::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 정렬 key의 마지막 동률 해소용으로 변경 canonical bytes를 추가했습니다(지시 key 4개로 같은
  값이 나오는 비정상 목록에서도 전순서를 보장).
- 같은 객체의 교체(삭제 + 추가)는 종류 code 순서상 AddObject(1)가 RemoveObject(2)보다 앞에
  옵니다. 적용(XIV.3.9)은 목록 순서와 무관한 단계별 적용으로 설계합니다.

## 남은 위험 / 한계

- diff bytes는 비교·전달용이며 저장 포맷이 아닙니다. 형식을 바꾸면 version을 올립니다.
