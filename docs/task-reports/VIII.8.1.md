# 작업 보고서: VIII.8.1 Orphaned Binding validator

- WBS: VIII.8.1
- 작업명: target/source ref가 사라진 binding을 삭제하지 않고 `OrphanedBinding`으로 변환하는 validator(원래 ID·타입·
  path·원인 보존)
- 상태: 완료 (`[x]`)
- 선행: VIII.6.1(`[x]`, 이 lane), III.2.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/bindings/src/orphan.rs`(신규) — `BindingWorld`, `OrphanCause`, `OrphanedBinding`, `break_cause`,
  `BindingStore`(live + orphan, `orphan_broken`)와 테스트.
- `crates/bindings/src/lib.rs` — `orphan` 모듈 등록.
- `docs/task-reports/VIII.8.1.md`(신규), 계획서(VIII.8.1 `[ ]`→`[x]`).

## 구현 요약

- **판정**(`break_cause`, 첫 원인 하나): ① target 속성 없음 `TargetMissing` → ② 참조 대상 없음 `SourceMissing`(대상과
  참조 표현식 node StableId 목록) → ③ 참조 대상 타입 변경으로 source type 오류 `SourceTypeChanged`(오류 node 포함) →
  ④ target 선언 타입 변경으로 결과를 둘 수 없음 `TargetTypeChanged`. 선언 타입이 바뀌어도 결과를 둘 수 있으면 깨진 것이
  아닙니다.
- **보존**: `OrphanedBinding`은 원래 `ValueBinding` 전체(ID·target 경로·마지막 유효 타입·source AST·owner)와 원인을
  가집니다. 없는 대상은 III.2.4 `UnresolvedRef`(원래 ID·기대 종류·발견 위치 = target 경로)로 볼 수 있습니다. 마지막
  계산값은 보관하지 않습니다.
- **저장소**: `BindingStore::orphan_broken(world)`가 깨진 live binding을 orphan으로 옮기고 새 orphan ID를 돌려줍니다.
  아무것도 삭제하지 않고, 이미 orphan인 binding을 자동 재연결하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bindings` | 종료 코드 0 |
| `cargo test -p stellar-bindings` | 종료 코드 0 (41 unit passed, 0 failed) |
| `cargo clippy -p stellar-bindings --all-targets -- -D warnings` | 종료 코드 0 (경고 0; 첫 빌드의 미사용 내부 helper 경고는 helper 제거로 해소) |
| `cargo fmt -p stellar-bindings -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **삭제 fixture 정보 보존**(`deleted_references_and_targets_become_orphans_with_information`): 토큰 삭제·target node 삭제 시 b1·b4만 orphan, 전체 수 4 유지, b1 binding·canonical bytes 원본과 동일, 경로·마지막 타입·원인(없는 토큰 + 참조 node)·UnresolvedRef(ID·Token·위치) 정확, target 원인 우선, 재검사·대상 복구 후에도 orphan 불변 | 통과 |
| **타입 변경 fixture**(`type_changes_become_orphans_only_when_broken`): target Length→Color는 `TargetTypeChanged`, Length→Optional\<Length>는 유지, Component Property Length→Color는 `SourceTypeChanged`(덧셈 node 오류), 마지막 유효 타입 보존 | 통과 |

## 차이(작업 지시와 다른 점)

- 신규 모듈이므로 테스트는 구현과 함께 작성했습니다(구현 전 상태는 컴파일 불가).
- Component Property 미해결 참조의 기대 종류는 III.2.4 `ExpectedRefType`에 Component Property가 없어 `Component`로
  표시합니다(원래 ID는 그대로 보존).

## 남은 위험 / 한계

- 현재 상태(속성 선언 타입·존재)는 `BindingWorld`로 받습니다. document·component 모델과의 연결은 Phase IX 소관입니다.
- orphan 저장 형식은 VIII.8.3(손상 파일 처리)에서 다룹니다.
