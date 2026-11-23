# 작업 보고서: XXVIII.2.3 semantic diff/merge conformance corpus와 runner

- WBS: XXVIII.2.3
- 작업명: semantic diff/merge conformance corpus와 runner 제공
- 상태: 완료 (`[x]`) — 2026-11-23 근거 커밋 기록: 구현·검증과 선행 판정은 이미 `[x]` 조건을 충족했고 작업 커밋 제목이 `Task(<ID>)` 형식이 아니어서 `[~]`에 머물렀습니다. 아래 "근거 커밋" 절에 실제 작업 커밋을 적었습니다. 이전 기록: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증과 선행 판정은 `[x]` 조건을 충족합니다. 보고 당시 기록: 완료 (`[x]`)
- 선행: XXVIII.1.3
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 10

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-merge/src/conformance.rs`(신규) — `Implementation`, `official`, `load_corpus`, `run_case`·`run_cases`·`run_corpus`,
  `Verdict`·`Report`, `expected_case`·`write_case`, 16진수 형식.
- `crates/semantic-merge/src/lib.rs` — 모듈 등록.
- `crates/semantic-merge/conformance/`(신규) — 사례 10개 × 세 파일, `README.md`(형식·판정·사례 표·다시 만들기).
- `crates/semantic-merge/tests/conformance_corpus.rs`(신규).

## 구현 요약

- 사례마다 `MergeInput` canonical bytes, 기대 base → ours diff bytes, 기대 `MergeResult` bytes를 16진수 텍스트로 둡니다. 내용은 합성
  fixture뿐입니다.
- runner는 구현(`Implementation`: diff bytes, merge bytes)을 받아 bytes 단위로 판정합니다. 세 파일 중 하나라도 없거나 16진수·입력
  형식이 틀리면 corpus 오류입니다.
- 사례는 XIV.10.2\~XIV.10.8 규칙(자동 병합, 같은 변경, 속성 값·삭제 대 수정·상충 재정렬 conflict, 독립 삽입 key 재배정, 전용 세
  종류, 결과 검증 되돌리기)을 하나씩 담습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `STELLAR_CONFORMANCE_BLESS=1 cargo test -p stellar-semantic-merge --test conformance_corpus corpus_on_disk` | corpus 생성(30 파일, 172K) |
| `cargo test -p stellar-semantic-merge --test conformance_corpus` | 4 passed |
| `cargo test -p stellar-semantic-merge` | 전체 통과 |
| `cargo clippy -p stellar-semantic-merge -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 디스크 corpus 사례 이름·입력이 생성기와 같음 | 통과 |
| 공식 구현이 10개 사례 모두 bytes 단위 통과, corpus에 clean·conflict 사례가 모두 있음 | 통과 |
| conflict를 버리는 구현은 결과 불일치, diff를 비우는 구현은 diff 불일치로 실패 | 통과 |
| 홀수 길이·비16진수 텍스트 거부, 세 파일이 안 맞는 사례는 `Incomplete` | 통과 |

## 차이(작업 지시와 다른 점)

- corpus는 `docs/spec/conformance/`(XXVIII.2.1, 다른 lane 소유)가 아니라 엔진 crate의 `conformance/`에 둡니다. 공개 정리(라이선스·
  manifest 통합)는 XXVIII.2.1의 라이선스 결정과 함께 합칩니다.

## 남은 위험 / 한계

- 엔진 규칙을 바꾸면 기대 bytes가 바뀌므로 corpus를 다시 써야 하고, 그 diff가 규칙 변경의 리뷰 근거가 됩니다.

## 근거 커밋 (2026-11-23 기록)

이 행의 작업은 아래 커밋에 있습니다. 이 기록 커밋(`Task(XXVIII.2.3)`)은 원장이 근거 커밋을 찾도록 남긴 것이고 코드 변경은 없습니다.

- `09470c43` Docs: ADR-0081을 구현한 종료 코드·신뢰 조건·resolve 명령에 맞춤
- `bfae643f` Test: semantic diff·merge conformance corpus와 runner 추가
