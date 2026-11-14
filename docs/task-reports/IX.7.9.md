# 작업 보고서: IX.7.9 ViewPresentation 런타임 타입

- WBS: IX.7.9
- 작업명: 캔버스에 같은 View를 여러 variable assignment로 배치하는 `ViewPresentation` 런타임 타입(definition 복제 안 함)
- 상태: 완료 (`[x]`)
- 선행: IX.7.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/view_variant/presentation.rs`(신규) — `ViewPresentation`, `PresentationCanvas`, `PresentationId`,
  `PresentationError`.
- `crates/document/src/view_variant/mod.rs` — 모듈 등록·문서.
- `docs/task-reports/IX.7.9.md`(신규), 계획서(IX.7.9 `[ ]`→`[x]`).

## 구현 요약

- **ViewPresentation**: 공유 정의(`Arc<ViewVariantModel>`, IX.7.8 변수 + Variant), 변수 할당(`Assignment`), 캔버스 위치
  (`Vec2`), 런타임 id. `resolve()`가 공유 정의의 Variant에 할당을 적용합니다(IX.7.4~IX.7.6 resolver). 정의를 복제하지
  않으며 `shares_definition_with`로 같은 객체 공유를 확인할 수 있습니다.
- **PresentationCanvas**: View마다 정의 하나를 보관하고 배치(place·set_assignment·move_to·remove)를 관리합니다. 배치 시
  할당을 View 변수로 검증합니다. 정의 교체(`update_definition`)는 그 View의 모든 배치를 새 정의 하나로 다시 묶고 새
  정의에서 무효가 된 할당의 배치 id를 돌려줍니다.
- 런타임 전용이며 저장 형식이 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 340 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 서로 다른 할당 100개 배치 → 정의 `Arc` strong count 101(캔버스 1 + 배치 100), 모든 배치 `ptr_eq`, 할당별 resolve 결과(compact+error 3개 일치, regular+processing 0개)와 위치 정확(`hundred_presentations_share_one_definition`) | 통과 |
| 정의 교체 후 배치 2개가 새 정의 하나 공유(strong count 3), 삭제 변수 쓰는 할당 무효 보고, 할당 검증·이동·삭제·없는 View 거부(`definition_update_rebinds_all_presentations_to_one_new_definition`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 배치별 scene 결과(base + effective override)의 materialize와 캐시는 렌더·layout 계층 몫입니다. 여기서는 resolve 결과까지
  제공합니다.
