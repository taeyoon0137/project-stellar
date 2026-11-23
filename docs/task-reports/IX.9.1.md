# 작업 보고서: IX.9.1 Button·TextInput·Modal·List·Radio 합성 component library fixture

- WBS: IX.9.1
- 작업명: Button, TextInput, Modal, List, Radio 합성 예제 component library fixture
- 상태: 부분 완료 (`[~]`) — 2026-11-23 library-finish, 아래 "작업 (2026-11-23, library-finish)" 절. 이전 기록: `[~]` 유지 — 남은 범위·결함 있음(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 이 fixture에 접근성 tree golden과 IX.8.5 접근성 진단 0건 확인이 아직 없고, scene document validator(VI.10.1)는 ComponentInstance를 표현하지 못해 "document validator 0 오류"를 컴포넌트 계층 검사기로만 확인했습니다(아래 "후속 수정 (리뷰 반영)", "통합 후 확인").
- 선행: IX.6(`[x]` 전체), IX.8(section 단위 선행. 작성 당시 미착수라 완화 적용, 통합 후 IX.8.1\~IX.8.6 모두 `[x]` — 아래 "차이"·"통합 후 확인" 참고)

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

## 후속 수정 (리뷰 반영)

리뷰 지적 두 가지(선행 IX.8 미완료, "document validator" 범위 재해석)에 따라 상태를 `[x]`에서 `[~]`로 낮추고 검사
범위를 넓혔습니다.

- **상태를 낮춘 근거**
  - 계획서 1.2 원자적 작업 패키지 규칙 4는 선행 작업이 모두 `[x]`가 아니면 코드를 수정하지 않게 합니다. 이 행의 선행
    IX.8(IX.8.1\~IX.8.6)은 모두 `[ ]`입니다. section 단위 선행이라 ledger `depends_on`에는 나타나지 않지만 규칙의
    예외는 아닙니다. IX.8.4는 이 fixture의 Button·TextInput·Modal·List·SingleSelectionGroup contract를
    role·state·action으로 매핑하고 IX.8.5는 label 누락 등 접근성 진단을 추가합니다. 따라서 IX.8이 들어오면 "document
    validator 0 오류"에 접근성 진단과 golden이 더해져야 합니다.
  - VI.10.1 `validate_document`는 `DocumentRecords`를 검사하는데 `NodePayload`에 ComponentInstance가 없습니다. 이
    fixture의 scene(인스턴스 5개)을 그 validator에 넣을 수 없고, 넣으려면 파일 포맷을 바꿔야 합니다. 그래서 "document
    validator"는 컴포넌트 계층 검사기 전부로만 확인했습니다. 문서 전체 validator로 확인한 것은 아닙니다.
- **검사 범위 확장**(`Test: IX.9.1 fixture에 정의 tree override·scene 캡슐화 검사 추가`)
  - 컴포넌트마다 `validate_definition_property_overrides`와 `validate_definition_slot_overrides`를 미해결 참조 건너뛰기
    없이 실행합니다. library 디코딩 경로는 미해결 참조를 건너뜁니다. Modal의 nested 닫기 Button 인스턴스는 override가
    없어서, 이 검사는 참조 컴포넌트 해석과 빈 override 검사만 합니다.
  - scene 전체 node(root, 인스턴스 5개, 투영 Text)에 `validate_scene_encapsulation`(IX.4.2 nested 직접 경로 금지)을
    실행합니다.
  - 모두 0 오류로 통과했습니다.
- **`[x]`로 올릴 조건**: IX.8 완료 후 이 fixture에 접근성 tree golden과 IX.8.5 진단 0건을 추가하고, scene document
  validator가 ComponentInstance를 다루게 되면 그 validator 0 오류를 함께 확인합니다.
- **ledger**: 계획서 상태를 바꾼 뒤 `cargo run -q -p xtask -- generate-ledger`로 `planning/task-status.yaml`을 다시
  만들었습니다. 이 lane이 ledger를 재생성하지 않아서, 같은 재생성으로 lane의 나머지 `[x]` 행(IX.1.3, IX.1.4,
  IX.4.1\~IX.4.6, IX.7.9, IX.9.3, XVI.6.3)도 `verified`로 함께 반영됐습니다. IX.9.1은 `reported_done`입니다.

| 명령(후속 수정) | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 354 passed, 통합 7+1+1, doc 4 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |
| `cargo run -q -p xtask -- generate-ledger` | 1023 tasks 생성, 종료 코드 0 |
| `cargo run -q -p xtask -- validate-task-status` | 종료 코드 0 |
| `cargo run -q -p xtask -- sync-plan` | 종료 코드 0 |

## 통합 후 확인 (2026-11-14)

- 통합 브랜치 계획서에서 IX.8.1\~IX.8.6이 모두 `[x]`입니다. 위 "차이"와 "상태를 낮춘 근거"의 "IX.8 미착수"는 작성 당시
  상태이며, 이제 선행 부족은 `[~]` 사유가 아닙니다.
- `[~]`는 유지합니다. 남은 조건은 위 "`[x]`로 올릴 조건"과 같습니다. 이 fixture에 접근성 tree golden(IX.8.3·IX.8.4
  생성기)과 IX.8.5 진단 0건 확인을 추가하고, scene document validator가 ComponentInstance를 다루게 되면 그 validator 0 오류를
  함께 확인해야 합니다. 이 수정에서는 코드와 fixture를 바꾸지 않았습니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 남은 범위·결함 있음.
- 근거: 컴포넌트 다섯 개 fixture와 컴포넌트 계층 검사기 0 오류는 있습니다. 검증 열의 "document validator 0 오류"는 아직 `validate_document`로 확인하지 않았습니다. 이전에 막혔던 이유(validator가 ComponentInstance를 표현하지 못함)는 사라졌습니다. 지금 `NodePayload::ComponentInstance`가 있습니다(`crates/document/src/records.rs` 43행).
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/document/src/test_support/component_fixtures.rs`: `component_library_fixture`, `fixture_scene`, golden `component_library.json`. 이 파일은 `validate_document`(`crates/document/src/validate.rs` 457행)를 부르지 않습니다.

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-document` | 368 passed, 0 failed |

### 남은 조건과 finding

- 결함 finding(document crate test_support, 소유 lane 없음 → backlog): fixture scene을 영속 레코드(`NodePayload::ComponentInstance`)로 만들어 `validate_document` 0 오류를 확인합니다. 선행 IX.8이 모두 `[x]`이므로 이전 보고서가 정한 접근성 tree golden·IX.8.5 진단 0건도 같은 작업에서 더합니다.
- X.11.2가 IX.9(절 단위)를 선행으로 둡니다.

## 작업 (2026-11-23, library-finish): 영속 레코드 fixture와 validator finding

### 한 일

- `tests/integration/tests/library_component_fixture.rs`(신규): 합성 fixture scene을 영속 문서 레코드(`NodePayload::ComponentInstance`)로
  옮겨, 같은 레코드에서 읽은 instance payload가 fixture library의 override 규칙(`check_payload_overrides`)과 instance 펼침
  (`InstanceExpansion::walk`)을 통과하는지 확인했습니다.
- 제품 문서 validator(`validate_document`)를 실행했더니 위반 1건이 나왔습니다: Modal instance가 body slot에 투영하는 instance 소유
  Text(IX.3 slot override)를 `ParentNotContainer`로 거부합니다(validator는 Frame·Fragment만 자식을 허용). 이 시험은 사유를 단
  `#[ignore]`로 두었습니다(ADR-0119 결정 8).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-project --lib library_` | library_entry·library_git 단위 시험 통과, 0 failed |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-assets -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |
| `cargo test -p stellar-integration-tests --test library_component_fixture` | 1 passed, 1 ignored |

| 검사 | 결과 |
|---|---|
| 영속 레코드의 instance 5개가 override 규칙·펼침 통과(`persisted_fixture_instances_follow_library_rules`) | 통과 |
| **document validator 0 오류**(`composed_component_library_fixture_passes_document_validator`, ignored) | 실패: `ParentNotContainer`(instance 소유 slot content) 1건 |

### 남은 조건과 finding

- 결함 finding(document crate `validate.rs`, 소유 밖): validator가 ComponentInstance의 instance 소유 slot content 자식을 받아야 합니다.
  고치면 `#[ignore]`를 지우고 이 행을 `[x]`로 올립니다.
- 접근성 tree golden은 문서 → 접근성 source tree 제품 bridge(XII.8.3)가 없어 넣지 않았습니다. 계획서 검증 열은 validator 0 오류입니다.
