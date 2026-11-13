# ADR-0006: Windows 지원 보류(macOS 우선)

## Status

Accepted (2026-11-14, 제품 소유자 결정)

## Context

기술 아키텍처 §2의 목표 2는 "macOS와 Windows에서 공통 코어를 사용하되 각 운영체제의 네이티브
애플리케이션 셸을 제공합니다"이고, §5.1은 Windows 11 `x64`를 초기 지원 대상으로 둡니다. 구현 계획은 이를 위해 Windows 셸(C++/WinRT, WinUI 3), Direct3D 12
backend, Windows 파일 감시·글꼴·복구 adapter, MSIX 패키징·서명·업데이트, Windows ARM64,
`CI-WIN`·`LAB-WIN`·`REF-WIN` 검증 환경을 포함합니다.

제품 소유자는 2026-11-14에 "당분간 Windows 지원 계획이 없다"고 결정했습니다. 현재 개발·검증
환경은 macOS뿐이며, Windows 전용 작업은 착수해도 필수 환경 증거를 만들 수 없습니다.

## Decision

1. **지원 대상**: 당분간 Stellar는 macOS(Apple Silicon, macOS 26 이상)만 지원합니다.
2. **코어는 플랫폼 중립 유지**: Rust 코어의 OS 의존 코드는 지금처럼 셸·adapter 경계에만 둡니다.
   이미 있는 Windows 지향 코드와 crate(예: `renderer-d3d12` 골격, Git 설치 winget adapter)는
   삭제하지 않고 빌드·테스트가 계속 통과하는 상태로 유지합니다.
3. **Windows 전용 WBS 행은 `[!]`**(사용자 결정으로 차단)로 표기합니다. 대상:
   II.6.1~II.6.4, II.7.2, II.7.3, XI.9.1~XI.9.7, XII.2.3, XII.3.3, XII.4.2, XII.5.2, XII.5.5,
   XII.7.4, XII.8.2, XII.9.2, XIII.2.3, XIII.6.3, XIII.8.6, XIV.13.6, XVI.2.4, XVI.10.1~XVI.10.5,
   XVIII.3.1~XVIII.3.4, XVIII.4.4, XX.5.2, XXI.2.2, XXVII.1.1~XXVII.1.3, XXVII.2.1~XXVII.2.3.
4. **양 플랫폼 비교가 들어 있는 행**(예: "양 플랫폼 동일", "macOS·Windows CI 비교")은 macOS
   부분을 구현합니다. 판정 규칙:
   - 결과가 IEEE-754 기본 연산(사칙연산·`sqrt`)과 정수 연산만으로 정해지고 OS API나 libm
     초월함수에 의존하지 않으며 golden 값이 고정되면, macOS 증거로 `[x]`를 줄 수 있습니다
     (구성상 플랫폼 간 동일).
   - OS 글꼴·shaping, GPU 출력, libm 초월함수처럼 플랫폼 구현에 의존하는 결과는 macOS 부분이
     끝나도 `[~]`로 두고, 보고서의 "남은 위험"에 Windows 재개 시 필요한 검증을 적습니다.
5. **검증 환경**: `CI-WIN`·`LAB-WIN`·`REF-WIN` 요구와 gate의 Windows 조건은 보류합니다.
   macOS 환경(`DEV-MAC`·`CI-MAC`·`REF-MAC`) 요구는 그대로 유지합니다.

## Alternatives

- **macOS·Windows 병행 유지**: 검증 환경과 개발 자원이 없어 Windows 행이 계속 미완료로 남고
  gate 진행이 막힙니다. 채택하지 않습니다.
- **Windows 코드·crate 삭제**: 범위는 단순해지지만 재개 비용이 커지고 광범위한 구조 변경이
  됩니다. 채택하지 않습니다.
- **Windows 행 삭제**: 계획서는 목적이 사라진 행만 삭제하고 폐기 표기는 두지 않습니다(계획 변경
  규칙, 2026-11-14 개정). Windows 행은 재개 시 다시 필요하므로 삭제 대상이 아니며, `[!]`가
  "사용자 결정으로 차단"을 정확히 표현합니다.

## Consequences

- 긍정: 범위가 macOS로 좁아져 진행 속도와 검증 가능성이 높아집니다.
- 부정: 결정론 원칙(§3.1)의 교차 플랫폼 검증에 공백이 생깁니다. 4번의 판정 규칙과 golden
  고정으로 완화하지만, Windows 재개 시 `[~]` 행과 golden을 Windows에서 다시 검증해야 합니다.
- 부정: 유지하는 Windows 지향 골격 코드의 관리 비용이 남습니다.

## Compatibility

파일 포맷·저장 구조·공개 FFI(C ABI)에 영향이 없습니다. FFI는 계속 C ABI 경계로 유지해 재개 시
Windows 셸이 같은 헤더를 사용할 수 있습니다.

## Performance

macOS 성능 예산과 기준 하드웨어(M1 16GB)는 그대로입니다. Windows 기준 기기 측정은 보류합니다.

## Security

Windows 전용 보안 항목(MSIX 서명, Windows 프로세스 신원 검사 등)은 보류합니다. macOS 서명·공증과
macOS 보안 항목은 그대로 진행합니다.

## Migration

Windows를 재개하면 이 ADR을 `Superseded by ADR-YYYY`로 바꾸고, 3번 목록의 `[!]` 행을 `[ ]`로
되돌린 뒤, 4번 규칙으로 `[~]`가 된 행에 Windows 증거를 추가해 `[x]`로 확정합니다.
