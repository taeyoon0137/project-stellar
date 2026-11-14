# 작업 보고서: XV.5.1 읽기 전용 LibrarySnapshot과 resource key

- WBS: XV.5.1
- 작업명: library project export ComponentDefinition·TokenSet index를 읽는 `LibrarySnapshot`, 모든 resource key에 LibraryId+commit+StableId 포함
- 상태: 완료 (`[x]`)
- 선행: XV.2.1(`[x]`), IX.1.1(`[x]`), VIII.5.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 실제 Git fixture 테스트는 시스템 `git`을 씁니다.

## 변경 파일 목록

- `crates/dependencies/src/library_snapshot.rs`(신규) — `LibraryResourceKey`, `LibraryResourceKind`,
  export record(`ExportedComponent`·`ExportedProperty`·`ExportedTokenSet`·`ExportedToken`·
  `ExportFingerprint`·`ExportVisibility`·`LibraryExports`), `SnapshotLimits`, `SnapshotError`,
  `LibrarySnapshot`(+ `LibraryComponent`·`LibraryToken` view), `LibraryExportSource` trait,
  `ExportReadFailure`, `SnapshotRequest`, `SnapshotLoadError`, `load_cached_snapshot`, 테스트 fixture.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/adr/ADR-0027-library-snapshot-resource-key.md`(신규) — resource key·형식 중립 export·상한 결정.
- `docs/task-reports/XV.5.1.md`(신규), `docs/task-reports/XV.5.1-blocked.md`(삭제, 차단 사유였던
  VIII.5.1이 `[x]`가 됨), 계획서(XV.5.1 `[!]`→`[x]`).

## 구현 요약

- **resource key**: `LibraryResourceKey { node: DependencyNodeKey, resource: StableId }`. node가
  LibraryId + exact commit이라 key 하나에 세 값이 모두 들어갑니다. 정렬은 LibraryId → commit →
  StableId이고 표시는 `<library>@<commit>#<stable id>`입니다.
- **export record**: ComponentDefinition(id, 표시 이름, 공개·비공개 property, slot)과 TokenSet(id,
  token: id·이름·타입 표기·값 encoding)을 형식 중립 record로 받습니다. document·bindings crate에 의존할
  수 없어(crate graph) 저장 데이터 → record 변환은 `LibraryExportSource` 구현이 맡습니다(ADR-0027).
- **검증**: 상한(component·property·slot·TokenSet·token 수, 이름·타입 표기·encoding 크기), 같은 library
  안 resource StableId 중복, component 안 property·slot 중복, set 안 token 이름 중복, 빈·제어 문자 타입
  표기를 거부합니다. token은 set 안에서 StableId 순으로 정렬합니다.
- **읽기 전용**: 필드 비공개, 변경 API 없음, 검증 생성자만 존재합니다.
- **cache 읽기**: `load_cached_snapshot`은 `open_entry`(공유 read lock)로 entry를 연 채 export를 읽고,
  snapshot 생성 후 lock을 풉니다. Git을 실행하지 않습니다. 기대 content hash(lock 값)가 다르면 export를
  읽지 않고 `ContentHashMismatch`로 실패합니다. entry가 없으면 `MissingEntry`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-dependencies` | 종료 코드 0 |
| `cargo test -p stellar-dependencies` | 70 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| v1/v2가 같은 StableId(component·token) export → 한 index에 4개 key, 충돌 없음, commit만 다름, 각 버전 값 보존(`same_stable_id_in_v1_and_v2_does_not_collide`) | 통과 |
| 종류 색인·token 정렬·resource key 순서(`snapshot_indexes_kinds_and_sorts_tokens`) | 통과 |
| 중복 resource·property·slot·token 이름, 잘못된 타입 표기 거부(`invalid_exports_are_rejected`) | 통과 |
| 모든 상한 초과 거부, 오류 메시지에 이름 원문 없음(`export_limits_are_enforced`) | 통과 |
| 실제 Git으로 materialize한 entry: 미 cache 시 `MissingEntry`, 읽는 동안 삭제 lock `Busy`, 읽은 뒤 lock 해제·Git 호출 0, hash 불일치 시 export 미호출, export 실패·검증 실패 구조화 오류(`cached_snapshot_is_read_under_shared_lock_and_detached_after`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 ComponentLibrary(kind 4)·TokenSet(kind 5) chunk를 읽는 `LibraryExportSource` 구현은 만들지
  않았습니다. 저장된 프로젝트에서 두 chunk를 문서 모델로 읽는 경로가 아직 없고(`document_io`는 manifest·
  view tree만 처리), dependencies·project crate는 bindings(TokenSet)에 의존할 수 없습니다. 검증 기준
  (v1/v2 같은 StableId 충돌 없음)은 형식 중립 record로 확인했습니다.

## 남은 위험 / 한계

- export 타입 표기·encoding의 canonical 규칙은 운영 `LibraryExportSource` 구현이 정해야 합니다. 규칙이
  바뀌면 XV.7.2 diff가 변경으로 판정합니다.
- ADR 번호 0023은 병렬 lane과 겹쳐 통합 때 ADR-0027로 옮겼습니다(0023 번호는 다른 결정용으로 비워 둡니다).

## 리뷰 수정(9A 리뷰 P4, 2026-11-14)

- 문제: `SnapshotLimits`는 `LibraryExportSource::read_exports`가 export 전체를 메모리에 만든 뒤에만
  적용됐고, trait이 상한을 받지 않아 공급자가 읽는 동안 멈출 수 없었습니다. 전체 합계 상한도 없어
  component 100,000개 × property 4,096개, token 1,000,000개 × 1 MiB fingerprint 같은 입력이 모두
  통과했습니다. 즉 상한은 메모리·CPU를 막지 못하고 검증 뒤 모양만 제한했습니다.
- 재현: `export_totals_are_enforced` — 개별 상한 안이지만 합계(property·slot·외부 참조·byte)가 상한을
  넘는 export가 수정 전 snapshot으로 만들어져 실패했습니다.
- 수정: `SnapshotLimits`에 전체 합계 상한 4개(`max_total_properties`·`max_total_slots`·
  `max_total_references` 각 1,000,000, `max_total_bytes` 256 MiB)와 `SnapshotError::TotalLimitExceeded`를
  추가하고, `LibrarySnapshot::new`가 resource별 색인 전에 합계를 검사합니다. `read_exports`는 호출자의
  `SnapshotLimits`를 받고, 공급자가 읽는 중 상한을 넘으면 `ExportReadFailure::LimitExceeded`로 멈춥니다
  (trait 문서·모듈 문서·ADR-0027 결정 3·5·Security 갱신).
- 검증: `export_totals_are_enforced` 통과. 실제 Git으로 materialize한 entry에서 공급자가 호출자 상한을
  받고, 1,000개 중 2개만 만든 뒤 `LimitExceeded`로 멈추며, 상한 안이면 같은 공급자로 snapshot을 만듦
  (`cached_snapshot_is_read_under_shared_lock_and_detached_after` 확장).
- 한계: 읽기 중 메모리 상한은 공급자 계약입니다. 운영 공급자(실제 ComponentLibrary·TokenSet chunk reader)가
  아직 없으므로 그 구현이 이 계약을 지키는지는 그 작업에서 검증해야 합니다.

## 상태 검토(9A 리뷰 P2, 2026-11-14)

- `[x]` 유지: 검증 열 "v1/v2 same StableId 충돌 없음"은 이 작업이 정의하는 실제 `LibrarySnapshot`·
  `LibraryResourceKey` 타입으로 검증했습니다. key가 LibraryId + commit + StableId이므로 결과는 export를
  어디서 읽었는지와 무관합니다.
- 사실 기록: 실제 ComponentLibrary·TokenSet chunk를 읽는 운영 `LibraryExportSource`는 없습니다(차이 절).
  저장된 프로젝트에서 두 chunk를 읽는 경로 자체가 아직 없습니다. 그 reader를 만드는 작업이 trait 구현과 읽기
  중 상한 계약(P4)을 함께 검증해야 합니다. 통합 판단에서 이 차이를 완료 기준 미달로 보면 `[~]`로 낮추고, 이
  행을 선행으로 둔 XV.5.2·XV.5.4·XV.7.1·XV.7.2도 함께 다시 판정해야 합니다.
