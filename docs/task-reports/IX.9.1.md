# 작업 보고서: IX.9.1 Button·TextInput·Modal·List·Radio 합성 component library fixture

- WBS: IX.9.1
- 작업명: Button, TextInput, Modal, List, Radio 합성 예제 component library fixture
- 상태: 완료 (`[x]`)
- 선행: IX.6(`[x]` 전체), IX.8(section 단위 선행 — 완화 적용, 아래 "차이" 참고)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/test_support/component_fixtures.rs`(신규, 테스트 전용) — `component_library_fixture`(컴포넌트 5개 +
  컴포넌트별 contract 표 + drilling 모음), `fixture_scene`(인스턴스 5개 scene), `library_snapshot`(결정론적 JSON), 테스트 4개.
- `crates/document/src/test_support/fixtures/component_library.json`(신규) — library 요약·checksum golden.
- `crates/document/src/test_support/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.9.1.md`(신규), 계획서(IX.9.1 `[ ]`→`[x]`).

## 구현 요약

- **Button**: Button 계약(activate = `activated`, disabled = Property), Activate → emit.
- **TextInput**: TextInput 계약(value = 상태 `text`, placeholder = Property), Input → `text = payload`.
- **Modal**: Modal 계약(focus scope = content, overlay, dismiss, open = 상태), body slot, nested 닫기 Button **인스턴스**,
  `closeLabel` → 닫기 Button `label` drilling link(IX.4.1), nested `activated` → `open = false` + emit `dismissed`(Event chain).
- **List**: List 계약(collection = items, Single, selection = 상태), SelectionChange → `selection = payload`.
- **Radio**: RadioButton node 없이 SingleSelectionGroup + List(Single) + option Frame 3개의 Button 계약으로 합성(IX.6.6).
- **scene**: 인스턴스 5개, Button `label` override, Modal body slot에 소유 Text 투영.
- **snapshot**: 컴포넌트별 node·property·slot·상태·계약 요약과 library·contract·drilling canonical bytes의 FNV-1a 64
  checksum을 ASCII JSON으로 출력해 golden과 byte 비교합니다(IX.9.4 입력).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1 cargo test -p stellar-document --lib component_fixtures` | golden 최초 생성, 통과 |
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 348 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| library 저장 검증(`ComponentLibrary::decode`: 정의·tree·Slot·Property·nested override·상태 머신) 통과·round-trip, 컴포넌트별 property·slot record 검사 통과, contract 검사 issue 0건(오류·진단 모두), 상태 머신 진단 0건, contract 표 round-trip, drilling 검증 통과·round-trip, Radio에 Frame·Text 외 node 없음(`composite_library_passes_every_document_validator_with_zero_errors`) | 통과 |
| scene 인스턴스 5개 property override·slot override 검사 통과, expansion walk 성공, Modal expansion에 nested Button 가상 node와 투영 slot content 포함(`scene_instances_validate_and_expand`) | 통과 |
| 검사가 실제로 동작함: option Button 계약 제거 시 `InvalidLinkedContract`, Button.label 삭제 시 drilling link orphan(`validators_are_active_on_the_fixture`) | 통과 |
| snapshot 결정론·ASCII·golden byte 일치(`library_snapshot_matches_fixture_bytes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 IX.8(접근성 의미 트리)은 아직 구현되지 않았습니다(IX.8.1~IX.8.6 `[ ]`). section 단위 선행이고 이 작업의 완료 기준
  ("document validator 0 오류")은 접근성 코드를 요구하지 않아 완화했습니다. IX.8이 구현되면 이 fixture에 접근성 tree golden을
  추가해야 합니다.
- "document validator"는 컴포넌트 계층 검사기 전부(library 저장 검증, property·slot record, control contract registry,
  상태 머신 진단, drilling, scene override·slot 투영, expansion)로 해석했습니다. scene `DocumentModel` validator(VI.10.1)는
  ComponentInstance payload를 아직 다루지 않아 대상이 아닙니다.

## 남은 위험 / 한계

- 플랫폼 간 byte 비교는 IX.9.4 범위입니다(Windows는 ADR-0006 보류).
