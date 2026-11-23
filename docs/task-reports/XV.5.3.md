# 작업 보고서: XV.5.3 library resource 읽기 전용 정책

- WBS: XV.5.3
- 작업명: library resource를 수정하는 command를 사용 project에서 거부, instance 공개 override만 허용
- 상태: 부분 완료 (`[~]`) — 2026-11-24 library-finish 비판 검토, 아래 "검토 (2026-11-24, library-finish 비판 검토)" 절: 사전 검사 함수 `check_component_edits`는 생겼지만 command를 적용하는 제품 경로(창 history·CLI·MCP transaction)가 아직 부르지 않습니다. 이전 기록: 완료 (`[x]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 판정 정책은 완료했지만 실제 command가 이 판정을 거치지 않습니다(9A 리뷰 P2).
- 선행: XV.5.2(`[x]`, 이 lane), IX.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/dependencies/src/read_only.rs`(신규) — `LibraryEditRequest`, `AllowedLibraryEdit`,
  `LibraryEditRejection`, `check_library_edit`, `check_library_edits`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.5.3.md`(신규), 계획서(XV.5.3 `[ ]`→`[x]`).

## 구현 요약

- **요청 분류**: 사용 프로젝트 command가 외부 resource에 요청하는 변경을 형식 중립 enum으로 받습니다.
  component 정의 편집, component property 정의 편집, token 정의 편집, resource 삭제, instance override
  설정, instance override reset입니다.
- **판정**: 정의·property·token 편집과 삭제는 snapshot 내용과 무관하게 `ReadOnlyResource`(전역 key와
  종류 포함)로 거부합니다. override 설정은 component·property가 snapshot에 있고, property가 공개이며,
  값 타입 표기가 같을 때만 허용합니다(IX.2.4 override command와 같은 공개 property 규칙). reset은
  instance 쪽 sparse 값만 지우므로 property가 사라진 뒤에도 허용합니다. 참조 library와 snapshot
  library가 다르면 `LibraryMismatch`입니다.
- **transaction 단위**: `check_library_edits`는 요청을 순서대로 판정하고, 하나라도 거부되면 그 위치와
  사유로 transaction 전체를 거부합니다. 의존성 snapshot이 없으면(offline 등) `LibraryUnavailable`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 76 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| component 정의·property 정의·token 편집, component·token 삭제, 없는 resource 편집 모두 `ReadOnlyResource`(`editing_library_definitions_is_rejected`) | 통과 |
| 공개 property override 성공, 비공개·타입 불일치·없는 property·없는 component·다른 library 거부, reset은 사라진 property도 허용(`public_override_is_allowed_and_others_are_rejected`) | 통과 |
| override만 있는 transaction 허용, 정의 편집이 섞이면 해당 위치로 전체 거부, snapshot 없는 의존성 거부(`transaction_with_any_library_edit_is_rejected_as_a_whole`) | 통과 |

## 차이(작업 지시와 다른 점)

- document·commands crate의 실제 command를 이 분류로 바꿔 호출하는 연결은 하지 않았습니다. 두 crate는 이
  lane 범위 밖이고, 아직 문서가 외부 참조를 저장하지 않아(XV.5.2 차이 참고) 외부 resource를 대상으로 하는
  실제 command가 없습니다. 판정 정책과 검증 기준 fixture는 이 crate에서 완료했습니다.

## 남은 위험 / 한계

- nested instance 경로를 거치는 override(`OverrideKey` 경로)는 직접 property override만 판정합니다. 경로
  override 허용 여부는 IX.2.3·IX.4.2 validator 규칙을 외부 component에 연결할 때 함께 정해야 합니다.

## 리뷰 수정(9A 리뷰 P3, 2026-11-14)

- 문제: 모듈 정책 표는 instance override reset을 항상 허용(instance 쪽 sparse 값 삭제)한다고 했지만,
  `check_library_edits`는 의존성 snapshot이 없으면(offline·로드 실패) reset까지 `LibraryUnavailable`로
  거부했고, `check_library_edit`는 참조 library가 선언과 다르면 reset을 `LibraryMismatch`로 거부했습니다.
  그래서 library를 쓸 수 없을 때 사용자가 자기 override를 지울 수 없었습니다.
- 재현: `reset_is_allowed_without_snapshot_or_with_mismatched_library`가 수정 전 실패했습니다.
- 수정: reset은 snapshot을 읽지 않고 허용합니다. `AllowedLibraryEdit::ResetOverride`는 전역 key 대신
  instance가 가진 외부 component 참조(`ExternalComponentRef`)를 담아 offline·library 불일치에서도 같은 값을
  돌려줍니다. 정의 편집·삭제·override 설정의 판정은 그대로입니다(snapshot 없으면 `LibraryUnavailable`).
- 검증: 위 테스트 통과(offline reset 허용, library 불일치 reset 허용, reset과 offline override가 섞인
  transaction은 override 위치 1에서 거부), 기존 `read_only` 테스트 3건 통과.

## 상태 정정(9A 리뷰 P2, 2026-11-14)

- `[x]` → `[~]`: 작업 행은 library resource를 수정하는 **command**를 사용 프로젝트에서 거부하라고 하지만,
  실제 command는 이 판정을 거치지 않습니다. 검증 열 "edit definition fixture 실패, override 성공"은 형식 중립
  `LibraryEditRequest` 분류 fixture로만 충족했습니다. 실제 command를 대신하는 분류로 검증한 것입니다.
- 남은 일: 외부 참조를 대상으로 하는 command가 transaction 적용 전에 `check_library_edits`를 호출하게
  연결하고, 실제 정의 편집 command가 거부되고 공개 override command가 성공하는 fixture를 확인하면 `[x]`로
  올립니다. document·commands crate는 dependencies에 의존할 수 없으므로(crate graph) 연결 위치는 둘을 함께
  쓰는 project crate 또는 runtime입니다. 문서가 외부 참조를 저장하는 통합(XV.5.2 차이)이 먼저 필요합니다.
- 이 crate의 판정 정책(정의·property·token 편집과 삭제 거부, 공개 override만 허용, reset 항상 허용)은
  완료했고 리뷰 수정 P3까지 반영했습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 판정 정책(정의·property·token 편집과 삭제 거부, 공개 override 허용)은 있지만 실제 command가 이 판정을 거치지 않아 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/dependencies/src/read_only.rs` `check_library_edit`·`check_library_edits`. commands·project·runtime에서 부르는 곳이 없습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-dependencies` | 96 passed |

### 남은 조건과 finding

- 결함 finding(소유 lane 없음 → backlog): 문서가 외부 참조를 저장하는 통합(XV.5.2 차이) 뒤, project crate나 runtime에서 transaction 적용 전에 `check_library_edits`를 부르고 실제 정의 편집 command 거부·override 성공 fixture를 확인합니다. document·commands는 dependencies에 의존할 수 없으므로 연결 위치는 project·runtime입니다.

## 작업 (2026-11-23, library-finish): 실제 document command 사전 검사 연결

### 한 일

- `crates/project/src/library_guard.rs`(신규): `check_component_edits`가 document command(`PropertyCommand`·`PropertyOverrideCommand`)를
  `LibraryEditRequest`로 바꿔 `check_library_edits` 하나로 판정합니다. 한 건이라도 거부되면 전체 거부입니다.
- `crates/project/src/library_refs.rs`(신규): 대상 component가 외부인지 정하는 규칙 하나(`classify_component`). 문서가 아직 외부
  참조를 저장하지 않으므로 자기 library에 없고 직접 의존성 하나만 export한 StableId를 외부 참조로 봅니다. 둘 이상이면 모호해서
  거부합니다(ADR-0119 결정 4).
- override 타입 표기 규칙(`override_type_accepted`, `Optional<T>`는 `T` 값을 받음)을 의존성 crate에 두었습니다(결정 3).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_read_only_commands` | 1 passed |

| 검사 | 결과 |
|---|---|
| **edit definition fixture 실패**: 실제 Git library Card에 대한 `PropertyCommand::Rename`·`Delete`가 `ReadOnlyResource`(전역 key = LibraryId + lock commit + StableId)로 거부 | 통과 |
| **override 성공**: 공개 `title` override가 허용되고 실제 `PropertyOverrideCommand::apply`로 instance payload에 적용(정의는 entry에서 제품 `read_library_resources`로 읽음) | 통과 |
| 비공개 property·타입이 다른 값 override 거부, override와 정의 편집이 섞인 transaction은 정의 편집 위치에서 전체 거부, 자기 component 편집은 통과, offline reset 허용, 같은 Card를 두 직접 의존성이 export하면 `AmbiguousComponent` | 통과 |

### 남은 위험

- 외부 component 판정은 문서가 외부 참조를 저장하기 전까지의 추론 규칙입니다. 저장 형식이 생기면 `classify_component`만 바뀝니다.
- `stellar-commands` CommandTransaction(Phase VII closed enum)에는 component command 갈래가 아직 없어, 사전 검사 진입점은 document
  command 묶음입니다.

## 검토 (2026-11-24, library-finish 비판 검토)

- 판정: `[~]` — 구현자 보고의 `[x]`를 받지 않습니다.
- 근거: 재판정 finding은 "transaction 적용 전에 `check_library_edits`를 부르고"였습니다. `crates/project/src/library_guard.rs`의
  `check_component_edits`는 실제 document command를 판정하지만, document command를 적용하는 제품 경로(`stellar-runtime` 창
  history, `stellar-ops` command 실행, CLI·MCP)는 이 함수를 부르지 않습니다(`grep -rn check_component_edits crates tools`). 제품에서
  library 정의 편집 command가 거부되는 경로는 아직 없습니다(지금은 문서가 외부 component를 저장하지 않아 그런 command가 만들어지지도
  않음).
- 확인한 것: `tests/integration/tests/library_read_only_commands.rs`의 정의 편집 거부·공개 override 성공(실제 command 적용)·혼합
  transaction 전체 거부·모호 대상 거부는 의미대로 판정합니다.
- 남은 조건: 문서가 외부 참조를 저장하는 작업(XV.5.2 차이) 뒤, component command를 적용하는 runtime·ops 경로가 transaction 적용 전에
  `check_component_edits`를 부르고 거부를 사용자 오류로 돌려줍니다. 소유 경로(runtime·ops·commands)는 이 lane 밖입니다.
