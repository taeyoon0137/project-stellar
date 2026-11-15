# 작업 보고서: XII.3.6 도구 상태 기계와 취소 복구

- WBS: XII.3.6
- 상태: `[x]` — 도구별 closed enum과 cancel/complete fixture 범위
- 선행: XI.3.5 `[x]`, Phase VII. VII.6.3 fuzz 검증은 기존 `[~]`를 유지하며 완료로 바꾸지 않았습니다.

## 필수 실행 환경

`DEV-MAC`. `/tmp/stellar-interaction-scene-test.log`와
`/tmp/stellar-interaction-scene-clippy.log`에 실행 결과를 남겼습니다.

## 변경 파일 목록

- `crates/scene/src/interaction.rs`: 여섯 도구 상태 기계, preview/outcome/cancellation, 수치 계산.
- `crates/scene/src/interaction/tests.rs`: complete/cancel, 실패 뒤 복구, 선·점 상자 회귀 검사.
- `crates/scene/src/lib.rs`: 모듈 등록.
- `crates/scene/src/transient.rs`, `crates/scene/src/transient/tests.rs`: 조절 가능한 축 판정과 handle 일치 검사.
- `docs/task-reports/XI.3.5.md`: 길이 0인 축의 handle 제외 규칙 보충. 기존 완료 상태 유지.
- 구현 계획, 이 보고서, 생성 원장 `planning/task-status.yaml`.

## 통합 근거와 구현 요약

기준점 `eac4531b`의 scene 코드는 대상 range 기준점 `3892c5e3`와 같았습니다.
`5e572e35`의 상태 기계, `b59a8623`의 모듈 문서 수정, `227c7b3a`의 pointer-up 실패 복구,
`ea96cf2c`의 길이 0인 축 resize 수정을 선별 통합했습니다. 커밋 전체를 cherry-pick하거나
다른 checkout의 작업 트리를 옮기지 않았습니다.

- `InteractionState`는 Idle과 Pan·Zoom·Select·Drag·Resize·Rotate의 닫힌 집합입니다.
  각 도구는 Pressed와 진행 상태를 가지며, 이동은 시작점 기준 preview만 계산합니다.
- pointer up은 최종 위치를 반영한 outcome 하나를 반환하고 Idle로 돌아갑니다.
  계산이 실패해도 `Cancelled`로 종료하며 실패 원인과 Pan·Zoom의 시작 camera를 돌려줍니다.
  다음 pointer down은 별도 cancel 없이 시작할 수 있습니다.
- 명시적 cancel은 outcome 없이 Idle로 돌아갑니다. targets와 preview는 비워지고
  Pan·Zoom만 복원할 camera를 반환합니다. 문서·history를 변경하는 API는 없습니다.
- `ResizeAxes`를 handle 생성과 resize 시작이 함께 사용합니다. 길이 0인 축은 배율 1로 고정하고,
  조절할 축이 없는 handle은 만들지 않습니다. 수평·수직선은 길이가 있는 축으로 조절할 수 있습니다.

## 실행한 명령 / 검증

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-scene -- --check` | 종료 0 |
| `cargo clippy -p stellar-scene --all-targets -- -D warnings` | 종료 0, 경고 없음 |
| `cargo xtask validate-task-report docs/task-reports/XII.3.6.md` | 종료 0 |
| `cargo xtask validate-crate-graph` | 종료 0 |
| `git diff --check` | 종료 0 |
| `cargo xtask generate-ledger` | `7b5cf84b` 커밋 후 종료 0, 1,145행 재생성 |
| `cargo xtask sync-plan --check` / `validate-task-status` / `verify-task XII.3.6` | 모두 종료 0 |
| `cargo xtask validate-adr <기준점 이후 전체 변경 경로>` | 종료 0 |
| `cargo test -p stellar-scene` | 종료 0. unit 117, integration 1, doctest 2 통과. 전용 성능 benchmark 1개 ignored |

16개 interaction 테스트가 여섯 도구의 진행→완료·취소, threshold, 선택하지 않은 node의 drag 전환,
마지막 위치 반영, 줌 clamp, 회전 누적, 뒤집기·회전된 resize, 잘못된 누름·이동, 뗌 실패 후 복구를 검사합니다.
최종 검사 로그는 `/tmp/stellar-interaction-final-checks.log`입니다.
transient layer 검사는 사각형·수평선·수직선·점·평행 축 상자의 handle과 상태 기계가 일치하는지 확인합니다.

## 차이(작업 지시와 다른 점)

정상 프로젝트 창은 아직 `InteractionController`를 만들거나 호출하지 않습니다. 공통 InputEvent는
ffi에, 이 상태 기계는 scene에 있으므로 입력 분류·hit test 결과를 넘기는 adapter가 필요합니다.
`PressTarget::from_hit` fixture는 실제 transient hit를 연결하지만 정상 앱의 편집 경로 증거는 아닙니다.
문서 저장·revision·history·선택 복구를 검증한 것으로 표시하지 않습니다(XII.3.7 미완료).

현재 origin에는 `docs/design/ui/canvas_editing.md`와 XII.10 계획이 있습니다. 원 보고서의
"캔버스 명세가 아직 없다"는 설명은 채택하지 않았습니다. 현재 수평 드래그 줌은 잠정 계산이며
현행 명세의 영역 드래그 확대(XII.10.37)를 충족하지 않습니다. 수식키·일시 도구 전환은
XII.10.1·XII.10.2, node별 크기 제약·CSS longhand 적용은 XII.10.9·XII.10.13의 미구현 범위입니다.
기존 WBS 행의 설명·완료 기준·선행은 유지하고 XII.3.6 상태만 갱신합니다.

## 남은 위험 / 한계

- 정상 앱에서 gesture 취소가 camera·선택·preview·history를 복구하는 연결은 아직 없습니다.
  XII.3.7의 선행 XI.2.1은 `[ ]`이고 runtime→scene 직접 의존도 허용되어 있지 않습니다.
- marquee는 world AABB이며 회전 camera의 화면 사각형과 다릅니다. 다중 선택의 부모 layout·잠금·
  node별 제약은 적용하지 않습니다. 최소 축 길이 기준은 없으며 극단 좌표는 오류로 거부될 수 있습니다.
- `atan2`·`sin_cos`·`exp2` 등 결과는 허용 오차 1e-4로 검증했습니다. Windows 동등성이나
  기준 장비의 입력→Present 성능을 주장하지 않습니다. 전용 benchmark와 전체 workspace 검증은 통합 담당 범위입니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U035 수용.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- 검증 기준(cancel/complete fixture): `interaction::tests` 16개 통과.
- 판정: `[x]` 유지.
