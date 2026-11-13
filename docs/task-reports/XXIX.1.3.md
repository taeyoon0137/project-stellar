# 작업 보고서: XXIX.1.3 sync-plan 본문 ID 참조 검사

- WBS: XXIX.1.3
- 작업명: `cargo xtask sync-plan`이 구현 지시·검증 셀과 Phase 완료 조건의 WBS ID 참조가 계획서에 없으면 보고
- 상태: 완료 (`[x]`)
- 선행: 없음

## 필수 실행 환경

- `DEV-MAC` (순수 Rust xtask, 로컬 검증).

## 변경 파일 목록

- `tools/xtask/src/plan/id_refs.rs` (신규) — 본문 WBS ID 참조 추출(`extract_id_references`, `wbs_id_tokens`).
- `tools/xtask/src/plan/mod.rs` (수정) — `id_refs` 모듈 선언.
- `tools/xtask/src/sync.rs` (수정) — `unknown-id-reference` 검사와 fixture test.
- `tools/xtask/src/commands.rs` (수정) — `sync-plan` 위반 출력에 code와 함께 상세 메시지(행 번호·위치·ID)를 표시.
- `docs/task-reports/XXIX.1.3.md`(신규), 계획서(XXIX.1.3 `[ ]`→`[x]`).

## 구현 요약

- **검사 위치**: task 정의 행(`extract_wbs`와 같은 판정: 셀 5개 이상, 첫 셀 WBS ID, 둘째 셀 상태 marker)의 상태 셀과 선행 셀 사이 본문, 그리고 `**Phase <N> 완료 조건:**`·`**Phase <N> 추가 완료 조건:**`로 시작해 빈 줄·머리글·표 행 전까지 이어지는 문단입니다. 셀 안의 `|`로 셀이 더 나뉘어도 상태와 선행 사이를 모두 합쳐 검사합니다.
- **제외**: 선행 셀(ledger depends_on 검사 담당), fenced code block, 개정 기록·요약 표·게이트 표 같은 그 밖의 본문, 계획서 밖 문서.
- **토큰 규칙**: ASCII 영숫자와 `.`의 최대 연속 구간에서 앞뒤 `.`를 떼고 기존 `WbsId::parse`가 받아들이는 것만 참조로 셉니다. `1.1.1`(Roman Phase 아님), `IIII.1.1`(비정규 로마 숫자), `I.1.1.1`(네 단계), `I.01.1`, `XIX.9`(두 단계), `v0.11.0`, `docs/.../I.2.1.md` 같은 표기는 세지 않습니다. 한국어 조사가 바로 붙은 `XIX.6.4절차`는 `XIX.6.4`로 셉니다.
- **범위 표기**: `~`가 토큰 경계이므로 `A~B`는 양 끝 ID만 각각 참조가 되고 중간 ID는 펼치지 않습니다.
- **보고**: 정의되지 않은 참조마다 `unknown-id-reference`를 `"<행>행 <위치>: 계획서에 없는 WBS ID 참조: <ID>"` 메시지로 계획서 행 순서대로 냅니다. 계획서·ledger는 수정하지 않습니다.
- 현재 계획서에서 검사 대상은 task 행 1023개의 본문 참조 90개와 Phase 완료 조건 참조 2개이며(동일 규칙의 보조 스크립트로 집계), 모두 정의된 ID입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p xtask` | lib 157 passed, 0 failed(통합 test 2+2 passed), 종료 코드 0 |
| `cargo clippy -p xtask --all-targets -- -D warnings` | 경고 0, 종료 코드 0 |
| `cargo fmt -p xtask -- --check` | 통과, 종료 코드 0 |
| `cargo xtask sync-plan --check` (현재 계획서) | 종료 코드 0 |
| 이 변경 전 xtask(`316bd32`)를 scratch 복사본에서 빌드하고, 계획서 복사본의 XIX.1.7 검증 셀에 `XXIV.1.99`, Phase II 완료 조건에 `II.9.9`를 넣어 `sync-plan` 실행 | 종료 코드 0(두 참조 모두 놓침) |
| 같은 scratch 복사본에 새 `tools/xtask/src`를 넣어 실행 | 종료 코드 65, `293행 Phase 완료 조건 … II.9.9`, `1851행 XIX.1.7 구현 지시·검증 … XXIV.1.99` 두 건 보고 |

| 검사 | 결과 |
|---|---|
| 삭제된 ID를 참조하는 fixture 보고(지시 셀·범위 끝·검증 셀·Phase 완료 조건 4건, `reports_references_to_deleted_ids`) | 통과 |
| 토큰 형식 규칙(`tokens_follow_wbs_id_format_only`), 범위 양 끝만 검사(`ranges_contribute_both_ends_only`) | 통과 |
| 검사 위치 한정: 선행 셀·개정 기록·요약 표·fence·일반 본문 제외(`extracts_instruction_verification_and_completion_only`) | 통과 |
| 현재 계획서 종료 0(`real_plan_matches_real_ledger`, `cargo xtask sync-plan --check`) | 통과 |

- 성능 작업이 아니며 자동 생성 파일 변경은 없습니다.

## 차이(작업 지시와 다른 점)

- `sync-plan` 위반 출력에 code만 있던 것을 `code: message` 형식으로 바꿨습니다. 새 검사는 어느 행의 어떤 ID인지 알아야 고칠 수 있기 때문입니다. 종료 코드(65)는 그대로입니다.

## 남은 위험 / 한계

- 두 단계 표기(`XIX.9`, `XIV.1~XIV.4` 같은 group 참조)는 작업 지시대로 WBS ID가 아니므로 검사하지 않습니다. 삭제된 group을 가리키는 참조는 잡지 못합니다.
- Phase 완료 조건 문단은 `**Phase ` 굵은 글씨 접두어로 찾습니다. 이 형식을 벗어난 완료 조건 문장은 검사하지 않습니다.
- 경로 안의 ID(`docs/task-reports/I.2.1.md`)는 네 단계 토큰이 되어 세지 않습니다.
