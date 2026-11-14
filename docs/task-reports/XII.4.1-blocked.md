# 차단 보고서: XII.4.1 macOS CanvasHost CAMetalLayer를 Metal backend에 연결

## WBS

- WBS: XII.4.1
- 작업명: macOS CanvasHost의 CAMetalLayer를 Metal backend에 연결하고 scale/size lifecycle을 전달합니다.
- 상태: 현행 계획 `[ ]` 유지. 구현 미통합·선행 검증 순환을 이 보고서에 기록합니다.
- 선행: XI.8.2 (`[~]`, Windows와 무관한 부분 완료), XII.2.2 (창 session 소유 경로 통합, 세션 통합 보고서 참고)

## 실패 선행조건

- **XI.8.2가 `[x]`가 아님**: 현재 기준 commit(eac4531b)의 XI.8.2는 `[~]`입니다. 사유는 Windows가 아니라 검증 열 "minimized/occluded
  window crash 없음"을 실제 NSWindow가 아닌 재현(0 크기 resize, `#[cfg(test)]` nil drawable 주입)으로만 확인한 것입니다
  (`docs/task-reports/XI.8.2.md` 상태 이력). 이 lane 규칙은 명시 행 ID 선행을 엄격히 적용하고 Windows 때문에 `[~]`인 행만
  충족으로 보므로, XI.8.2는 충족이 아닙니다. 그래서 코드를 바꾸지 않았습니다.
- **선행 순환**: XI.8.2의 남은 증거(실제 NSWindow를 최소화·가렸을 때 CanvasHost에 붙은 layer로 crash 없이 frame skip)는 CanvasHost
  layer를 Metal backend에 연결하는 이 행이 있어야 얻을 수 있습니다. 그런데 이 행이 XI.8.2를 선행으로 두므로, 계획을 바꾸지 않으면
  두 행 모두 진행할 수 없습니다. XI.8.2 보고서도 같은 결정이 필요하다고 적었습니다.

## 재현 명령

- `grep -n "^| XI\.8\.2 \|^| XII\.4\.1 " docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` → XI.8.2 `[~]`, XII.4.1 선행 `XI.8.2, XII.2.2`.
- `sed -n '/## 상태 이력/,$p' docs/task-reports/XI.8.2.md` → `[~]` 정정 사유(실제 창 증거 없음).

## 필요한 결정과 환경

다음 중 하나를 정하는 계획 변경 결정이 필요합니다(계획 3장: `[~]`인 XI.8.2는 고치지 않고, 미착수 XII.4.1은 결정 변경에 맞춰
고칠 수 있음).

1. **증거 이동**: XI.8.2의 남은 실제 창 증거를 XII.4.1 검증 열로 옮기고("resize, display scale change, 최소화·가림 창 렌더 통과"),
   XII.4.1의 선행에서 XI.8.2를 XI.8.1로 바꿉니다. XI.8.2는 XII.4.1 증거로 닫는 후속 기록 행을 둡니다.
2. **XI.8.2 단독 완료**: `renderer-metal`에 실제 NSWindow를 만들어(테스트 전용 harness, main thread) CAMetalLayer를 붙이고 최소화·
   가림 상태에서 frame skip·crash 없음을 확인하는 검증을 XI.8.2 후속으로 먼저 수행합니다. 그 뒤 이 행을 시작합니다.

실행 환경은 `DEV-MAC`(실제 GPU, 화면에 띄운 창)이면 충분합니다.

## 변경되지 않은 파일

- 코드 변경 없음. `apps/macos/StellarMac/CanvasView.swift`(layer 없음, drawable 크기만 전달)와 `crates/renderer-metal`은 그대로입니다.
- 이 lane에서 준비된 재료: 창 controller가 소유하는 창 session handle(XII.2.2), 창별 active view·viewport(XII.2.5), 창 session이
  로드한 작업 문서. 캔버스에 문서를 그리는 데 필요한 document → scene 입력 변환(binding·layout 평가 결과를 `SceneBuilder` 입력으로
  옮기는 단계)은 아직 어느 행에도 구현되어 있지 않습니다.

## 재개 조건

- 위 결정 1 또는 2가 계획에 반영되고 XII.4.1의 선행이 모두 `[x]`(또는 Windows 때문에 `[~]`)가 되면, `CanvasView`를
  CAMetalLayer-backed view로 바꿔 `MetalBackend::with_layer`에 넘기고 backing scale·drawable 크기 변경을 backend resize로 전달하며,
  resize·display scale change 렌더 검사를 smoke에 추가합니다. 캔버스 drawable은 safe area 안에 둡니다(열린 결정 O2).
