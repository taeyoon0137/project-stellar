# 작업 보고서: VI.9.4 modifier 참조 의존성 등록

- WBS: VI.9.4
- 작업명: Boolean·Mask·Clip의 다른 NodeRef 의존성을 ReferenceIndex에 등록
- 상태: 완료 (`[x]`)
- 선행: VI.2.5, VI.9.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/modifier.rs` — `ModifierPayload::referenced_nodes`,
  `ModifierStack::sync_references`.
- `docs/task-reports/VI.9.4.md`(신규), 계획서(VI.9.4 `[ ]`→`[x]`).

## 구현 요약

- **참조 추출**: Boolean 피연산 node들, Mask·Clip 원본 node를 반환합니다. 대상 미지정
  (NIL 참조)은 제외합니다.
- **동기화**: `sync_references(index)`는 소유 node의 기존 outbound edge를 지우고 현재
  stack의 참조로 다시 등록합니다(VI.2.5 ReferenceIndex). 참조 추가·제거·modifier 삭제
  후 호출하면 inbound가 정확합니다. 다른 node가 만든 edge는 건드리지 않습니다.
- **비활성 modifier**: 저장에 남아 대상 삭제 시 영향이 있으므로 참조를 계속
  등록합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | 137 passed, 0 failed |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 참조 추가(Boolean 2개·Mask)·제거(피연산 삭제)·비활성 유지·modifier 삭제 시 inbound 정확, 타 node edge 보존(`reference_index_tracks_modifier_dependencies`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 호출 시점(command 적용 후 동기화)은 command 계층(Phase VII)이 보장해야 합니다.
  cycle 검출은 VI.9.5입니다.
