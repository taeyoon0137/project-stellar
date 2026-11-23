# ADR-0098: macOS 셸 UI를 지우고 빈 창 껍데기만 남김

## Status

Accepted (2026-11-23). 제품 책임자 지시(2026-11-23)를 기록합니다. 지시 내용은 다음과 같습니다. "UI 관련
작업했던건 껍데기만 남기고 다 지워." 같은 날 앞선 지시("UI는 처음부터 재설계가 필요해보임")를 실행합니다.

## Context

- 앞선 셸 구현(시작 창, 프로젝트 창, 사이드바, 인스펙터, 도구·메뉴, Git 패널, 캔버스 view, 네이티브 텍스트 필드, 입력
  변환)은 재설계 대상이라 유지할 이유가 없습니다.
- 엔진(Rust crate)과 셸 FFI(`crates/ffi`, `stellar_ffi.h`)는 UI가 아니므로 그대로 둡니다.

## Decision

1. `apps/macos/Stellar`에는 다음만 남깁니다.
   - 진입점(`main.swift`), 앱 delegate(메뉴 Quit 하나와 제목만 있는 빈 창 하나)
   - 코어 runtime 수명(`RuntimeHost`, `LifecycleCheck`)과 bridging header
   - 앱 업데이트(`UpdateChecker`, `UpdateCheck`, ADR-0017)
   - `stellar` 명령 설치기(`StellarCommandInstaller`)와 그 fixture 검사(`CommandInstallCheck`). 화면이 아닌 기능이라
     남기고, 설치 화면만 지웁니다.
2. 지운 것: `UI/` 전체, 캔버스 view·scene·PNG·텍스트 입력, Swift FFI wrapper(`CoreRuntime*`), 네이티브 텍스트 필드,
   설정 화면, 입력·텍스트 입력·영역별 UI·셸 통합 검사, `scripts/macos/qa.sh`, `scripts/macos/canvas-move-check.py`,
   UI 설계 문서(`docs/design/ui/`, `docs/design/ux/`, `docs/design/macos-shell-design.md`), 실사용 QA 문서(`docs/qa/`).
   필요하면 Git 기록에서 되살릴 수 있습니다.
3. `scripts/macos/smoke.sh`는 빌드, `--smoke`, `--lifecycle-check`, `--update-check`, `--command-install-check`만
   실행합니다.
4. 셸 구현이 결과물이던 계획 행은 다시 착수 전(`[ ]`)으로 돌립니다. 코어 쪽 결과가 남는 행은 그대로 둡니다.
5. 셸 FFI는 바꾸지 않습니다. 새 UI가 FFI를 어떻게 쓸지는 재설계 때 정합니다.

## Alternatives

- 기존 UI를 비활성 상태로 남김: 재설계 때 기준을 흐리고 빌드·검사 비용만 남아 버렸습니다.
- 앱 target까지 지움: 빌드·서명·업데이트·FFI 연결 검사가 함께 사라져 버렸습니다.

## Consequences

- 앱을 실행하면 빈 창 하나만 뜹니다. 프로젝트 열기·편집은 CLI·MCP로만 할 수 있습니다.
- 이 결정 이전 ADR(ADR-0028, ADR-0073 등)의 셸 화면 관련 내용과 작업 보고서의 셸 검사 기록은 역사 기록으로 남고,
  현재 구현과 다릅니다.

## Compatibility

파일 포맷·FFI·CLI·MCP 영향 없음.

## Performance

영향 없음.

## Security

설치기의 동의 전 파일 변경 금지와 최소 권한 규칙은 그대로 검사합니다.

## Migration

없음.
