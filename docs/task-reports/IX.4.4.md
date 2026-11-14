# 작업 보고서: IX.4.4 DrillingPlan preview와 결정론적 이름 suffix

- WBS: IX.4.4
- 작업명: DrillingPlan preview에 생성될 property 이름, type, default, component 경로 포함, 이름 충돌은 deterministic
  suffix 제안
- 상태: 완료 (`[x]`)
- 선행: IX.4.3(이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/component/drilling_plan.rs` — `NewProperty::name`(제안 이름), `DrillingPlan::preview`,
  `DrillingPreview`, `PreviewProperty`, `PreviewLink`, `ComponentPathSegment`, `propose_name`.
- `docs/task-reports/IX.4.4.md`(신규), 계획서(IX.4.4 `[ ]`→`[x]`).

## 구현 요약

- **제안 이름**: 계획 계산 시 새 property마다 그 컴포넌트의 기존 이름 **집합**과 비교해 겹치면 `"{기준} {n}"`(n = 2부터
  첫 빈 번호)를 붙입니다. 이름 상한(256 bytes)을 넘으면 기준 이름을 UTF-8 문자 경계에서 줄입니다. 집합 비교라 선언
  순서와 무관합니다.
- **preview**: 새 property마다 컴포넌트 경로(top → 그 컴포넌트, 컴포넌트 ref + 표시 이름), 인스턴스 경로, 제안 이름,
  suffix 여부(`renamed`), 타입, default 값. 새 link마다 owner·인스턴스·source 이름·대상 컴포넌트·대상 이름(새 property는
  제안 이름, 기존 property는 현재 이름).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 332 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 기존 이름 선언 순서만 다른 두 library에서 plan·preview 동일, 충돌 suffix `label 2`·`label 3`, 컴포넌트 경로·인스턴스 경로·타입·default·link 이름 정확, 충돌 없으면 기준 이름(`preview_lists_names_types_defaults_and_paths_deterministically`) | 통과 |
| 255 bytes 이름 충돌 시 suffix 포함 256 bytes 이하·이름 규칙 통과(`proposed_names_respect_length_limit`) | 통과 |
| IX.4.3 계획 테스트(1·2·10단계, 재사용, 거부) 회귀 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 사용자가 preview에서 이름을 직접 바꾸는 경로는 아직 없습니다. 적용(IX.4.5)은 제안 이름을 그대로 씁니다.
