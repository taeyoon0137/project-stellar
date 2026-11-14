# 차단 보고서: XII.3.7 transient preview와 앱 입력 경로 연결

## WBS

- WBS: XII.3.7
- 상태: `[!]`
- 선행: VII.4.2 `[x]`, XI.2.1 `[ ]`
- 작업명: interaction preview를 Scene builder에 주입하고 pointer up에서만 CommandTransaction 확정

## 실패 선행조건

최신 origin과 이 lane의 기준점 `eac4531b` 모두 XI.2.1(layout/binding/modifier invalidation →
scene dirty set 변환)이 `[ ]`입니다. preview가 바꾸는 scene dirty 범위와 revision 계약을 이 선행 없이
완료로 표시하지 않습니다. 이전 `0c464875`·`289c3793`의 차단 근거를 현재 코드와 계획에 대조했습니다.
WBS 설명·완료 기준·선행은 유지하고 XII.3.7 상태만 `[!]`로 갱신합니다.

## 재현 명령

- `rg -n '^\| (XI\.2\.1|VII\.4\.2|XII\.3\.[567]) ' docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`
  — XI.2.1 `[ ]`, VII.4.2 `[x]`, XII.3.5 `[~]`, XII.3.6 `[x]`, XII.3.7 `[!]`.
- `rg -n 'pop_received|submit_hit_test|InteractionController' crates/ffi/src crates/runtime/src apps/macos/StellarMac`
  — 정의·Rust 테스트 외에 정상 앱에서 상태 기계와 hit-test 요청을 구동하는 연결이 없습니다.
- `rg -n 'update_preview|execute_transaction' crates/history/src/history.rs`
  — 기존 preview는 document transaction을 적용합니다. transient scene 주입과는 다릅니다.

## 필요한 결정과 환경

`DEV-MAC`에서 XI.2.1의 dirty 변환을 먼저 구현·검증한 뒤 runtime과 scene의 연결 위치를 정해야 합니다.
현재 runtime은 scene에 직접 의존하지 않고 FFI는 core-types/runtime 경계만 사용합니다. crate graph를
임의로 바꾸지 않았습니다. 입력의 project/window별 대상과 정상 프로젝트의 DocumentThread 소유권도
연결 시 명시해야 합니다. 전역 입력 대기열을 별도 테스트 thread로 비우는 방식은 정상 앱 증거가 아닙니다.

## 변경되지 않은 파일

이 차단 기록에서는 제품 코드를 바꾸지 않았습니다. 앞선 XII.3.5·XII.3.6 통합으로 준비된 범위는 다음과 같습니다.

- `f360ec50`: FFI 수신 receipt, 병합 보존, DocumentThread trace API. 앱의 자동 전달은 미연결.
- `7b5cf84b`: 도구별 상태 기계, 취소 결과, pointer-up 실패 뒤 Idle 복구, 선·점 handle 판정.
- 기존 `History`의 gesture API와 `WindowHistory::cancel_preview`의 선택 복원은 유지했습니다.
  이 lane은 history 구현을 중복 변경하거나 기존 완료 행을 되돌리지 않았습니다.

## 재개 조건

1. XI.2.1 완료 후 `InteractionPreview`를 scene builder의 transient 입력에 반영하고,
   drag 500 update 동안 project revision 증가 0을 확인합니다.
2. pointer up에서 `InteractionOutcome`을 CommandTransaction 하나로 확정하고 history 1건을 확인합니다.
   cancel 또는 실패한 pointer up은 preview를 폐기하고 camera·선택을 복원하며 history를 추가하지 않아야 합니다.
3. 정상 프로젝트 창의 입력을 receipt와 함께 해당 DocumentThread에 보내 실제 `hit_test_canvas` 결과로
   `InteractionController`를 구동합니다. AppKit 입력→T0→실제 hit test→상태 전이를 앱 검사로 증명하고
   Rust fixture만으로 XII.3.5를 `[x]`로 올리지 않습니다.
4. 현행 캔버스 명세의 수식키·도구 전환·영역 드래그 줌·CSS 속성 적용은 해당 XII.10 행과 함께 검증합니다.
   도구 상태 기계 기반 구현을 최종 UX 완료로 확대하지 않습니다.

이 lane은 전체 workspace 검증과 push를 수행하지 않습니다. 통합 담당이 다른 lane과 합친 뒤
최종 검증·push를 수행합니다. 다른 checkout과 기존 dirty 작업 트리는 수정하지 않았습니다.

## 통합 검증 기록

- `cargo xtask generate-ledger`, `sync-plan --check`, `validate-task-status`,
  `validate-task-report docs/task-reports/XII.3.7-blocked.md`: 모두 종료 0.
- `cargo xtask validate-adr`는 기준점 이후 실제 변경 경로 전체로 실행해 종료 0.
  `validate-crate-graph`, 관련 crate `cargo fmt -- --check`, FFI 헤더 drift 검사도 종료 0.
- 기준점과 현재 계획의 상태 칸을 제외한 전체 내용이 같고, 기존 완료 행의 하향 변경이 없음을 검사했습니다.
  ADR 파일 28개 번호에 중복이 없고 ADR-0029는 입력 trace 한 주제뿐입니다.
- macOS smoke는 FFI 통합 뒤 종료 0이었습니다. Rust test/clippy의 범위와 결과는 XII.3.5·XII.3.6
  보고서를 따릅니다. metadata 추출 생략 경고 1건, scene 전용 성능 benchmark 1개 ignored가 남습니다.
