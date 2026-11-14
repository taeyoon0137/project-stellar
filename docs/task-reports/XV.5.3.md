# 작업 보고서: XV.5.3 library resource 읽기 전용 정책

- WBS: XV.5.3
- 작업명: library resource를 수정하는 command를 사용 project에서 거부, instance 공개 override만 허용
- 상태: 완료 (`[x]`)
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
