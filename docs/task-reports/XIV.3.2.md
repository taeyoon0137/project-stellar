# 작업 보고서: XIV.3.2 SemanticChange kind 정의

- WBS: XIV.3.2
- 작업명: `SemanticChange` kind에 AddObject, RemoveObject, MoveObject, ReorderObject, SetProperty, Add/Remove/ChangeToken, ComponentInterfaceChange, AssetRefChange 정의
- 상태: 완료 (`[x]`)
- 선행: XIV.3.1 `[x]`(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/semantic-diff/src/change.rs`(신규) — `SemanticChangeKind`, `Provenance`, `Position`,
  `ObjectChange`, `PositionChange`, `PropertyChange`, `SemanticChange`.
- `crates/semantic-diff/src/lib.rs` — `change` module 선언.
- `docs/task-reports/XIV.3.2.md`(신규), 계획서(XIV.3.2 `[ ]`→`[x]`).

## 구현 요약

- **kind**: 10종에 안정 numeric code 1~10과 진단 이름(`add-object` …
  `asset-ref-change`)을 고정했습니다. `is_structural`로 객체 단위/속성 단위를 구분합니다.
- **분류 규칙**: token 객체의 추가·삭제는 AddToken/RemoveToken, token 속성은 ChangeToken,
  컴포넌트의 Property·Slot 목록과 Slot record 속성은 ComponentInterfaceChange, asset 참조 key
  (`keys::ASSET_REFERENCE_KEYS`) 속성은 AssetRefChange, 나머지는 SetProperty입니다. 생성자
  (`added`/`removed`/`property`)가 같은 규칙으로 variant를 고릅니다.
- **정보 보존**: 추가·삭제는 객체 전체, 위치 변경은 from/to `Position`, 속성 변경은 old/new
  (`None` = 속성 없음)를 담아 적용 전 검증(XIV.3.9)과 역변경(XIV.3.10)이 가능합니다.
- **출처**: `Provenance`(Project/Document/ComponentDefinition/InstanceOverride, code 1~4)를 모든
  변경이 가집니다. 생성 규칙은 diff 단계(XIV.3.3~XIV.3.6)가 정합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff` | 18 passed, 0 failed |
| `cargo fmt -p stellar-semantic-diff -- --check` | 통과 |
| `cargo clippy -p stellar-semantic-diff --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| kind code·이름 golden, 미정의 code 거부(`kind_codes_and_names_golden`) | 통과 |
| provenance code golden(`provenance_codes_golden`) | 통과 |
| 객체 종류·경로별 분류표(`classification_table`), token 객체 kind(`token_objects_use_token_kinds`) | 통과 |

## 차이(작업 지시와 다른 점)

- 출처(`Provenance`) 필드를 이 단계 모델에 포함했습니다. definition/instance override 구분
  생성은 XIV.3.6 범위입니다.

## 남은 위험 / 한계

- 바인딩 변경 전용 kind는 지시 목록에 없어 SetProperty로 분류됩니다. 바인딩 모델(VIII.6)이
  생기면 전용 분류가 필요한지 다시 판단합니다.
