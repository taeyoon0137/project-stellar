# macOS CanvasHost와 Metal 연결의 선행 검증

XI.8.2의 실제 창 검증은 제품의 CanvasHost 없이 독립적으로 실행할 수 있습니다. 시험용 NSWindow에 CAMetalLayer를 붙이고 제품의 `MetalBackend::with_layer`로 프레임을 그립니다.

```sh
cargo run -p stellar-renderer-metal --example window_surface
```

2026-11-14 DEV-MAC에서 창을 최소화하자 세 번의 프레임이 `EmptySurface`로 건너뛰어졌고, 복원 후 제출이 재개됐습니다. 크기 변경과 숨김·재표시를 포함해 GPU 제출 5회가 완료됐습니다. 이 실행에서 앱은 활성 상태가 되지 않았고 다른 창으로 덮었을 때의 가림도 관측되지 않아 검사의 종료 코드는 1이었습니다. [실행 기록](../task-reports/evidence/XI.8.2-window-surface.txt)

로그인한 데스크톱에서 활성 상태와 다른 창으로 가려진 상태를 다시 확인합니다. XII.4.1에는 실제 CanvasView의 레이어 연결, 크기 및 디스플레이 배율 전달, release 시 수명 검증이 남았습니다. 기존 캔버스는 셸의 safe area 안에 배치합니다. [XI.8.2 검증 보고서](../task-reports/XI.8.2.md)
