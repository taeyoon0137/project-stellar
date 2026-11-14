# XII.4.1 macOS CanvasHost 연결의 선행 검증 현황

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
  layer 연결까지 포함합니다. 독립적인 창 fixture로 먼저 backend 검증을 수행할 수 있습니다.

## 재현 명령

- `grep -n "^| XI\.8\.2 \|^| XII\.4\.1 " docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md` → XI.8.2 `[~]`, XII.4.1 선행 `XI.8.2, XII.2.2`.
- `sed -n '/## 상태 이력/,$p' docs/task-reports/XI.8.2.md` → `[~]` 정정 사유(실제 창 증거 없음).

## 필요한 결정과 환경

기존 XI.8.2 검증 기준을 따라, 테스트용 실제 NSWindow에 CAMetalLayer를 붙여 최소화·가림·복원 시 backend 동작을 먼저 확인합니다.
이 검증은 제품의 CanvasHost 구현에 의존하지 않습니다. 근거를 확보하면 XII.4.1에서 같은 동작을 실제 CanvasHost와 연결합니다.

실행 환경은 `DEV-MAC`(실제 GPU, 화면에 띄운 창)이면 충분합니다.

## 변경되지 않은 파일

- 코드 변경 없음. `apps/macos/StellarMac/CanvasView.swift`(layer 없음, drawable 크기만 전달)와 `crates/renderer-metal`은 그대로입니다.
- 이 lane에서 준비된 재료: 창 controller가 소유하는 창 session handle(XII.2.2), 창별 active view·viewport(XII.2.5), 창 session이
  로드한 작업 문서. 캔버스에 문서를 그리는 데 필요한 document → scene 입력 변환(binding·layout 평가 결과를 `SceneBuilder` 입력으로
  옮기는 단계)은 아직 어느 행에도 구현되어 있지 않습니다.

## 재개 조건

- 실제 창에서 XI.8.2 검증을 마치고 XII.4.1의 선행이 모두 `[x]`(또는 Windows 때문에 `[~]`)가 되면, `CanvasView`를
  CAMetalLayer-backed view로 바꿔 `MetalBackend::with_layer`에 넘기고 backing scale·drawable 크기 변경을 backend resize로 전달하며,
  resize·display scale change 렌더 검사를 smoke에 추가합니다. 캔버스 drawable은 safe area 안에 둡니다(열린 결정 O2).
