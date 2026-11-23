# 작업 보고서: IX.11.19 registry CSS 매핑과 Web capability matrix

- WBS: IX.11.19
- 작업명: registry CSS 매핑에서 Web profile capability 상태를 유도하는 초기 matrix
- 상태: 부분 완료 (`[~]`) — 유도 함수와 대조 test는 끝났습니다. `CapabilityTable`에는 등록하지 않았습니다. 그 표는
  Web·Apple·Android 계단을 모두 요구하고, 이 행은 Apple·Android 항목을 만들지 않기 때문입니다.
- 결정: ADR-0025 결정 1.3, 속성 모델 14장
- 선행: IX.11.2, IX.11.3, IX.11.18, IX.10.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust). capability 조건 확인에만 로컬 Google Chrome 157.0.8103.79(headless)를 썼습니다.

## 변경 파일 목록

- `crates/target-profile/src/css_matrix.rs`(신규), `crates/target-profile/src/lib.rs`(모듈 등록).
- `tests/integration/tests/web_capability_matrix.rs`(신규).

## 구현 요약

- target-profile은 문서 registry를 읽지 못합니다(crate 그래프). 그래서 matrix는 유도 함수
  `web_capability(RegistryInput)`입니다.
- 판정 순서는 다음과 같습니다.
  1. 14장 확장(E1·E2·E5·E7·E9·E10)
  2. capability 조건
  3. 대응표(`exact`·`converted` → Native, `approximated` → Approximated)
  4. `stellar-only`: 출력 규칙이 있으면 Approximated, 없으면 Unsupported
- Deprecated는 나오지 않습니다.

## 확인 결과 기록

- `clipping` FrameBounds의 `converted`(capability) 판정: 기준 browser에서 다음을 확인했습니다.
  - `CSS.supports("overflow-clip-margin", "content-box")` = true
  - `CSS.supports("overflow-clip-margin", "padding-box 4px")` = true
  - 그래서 Native입니다.
- E8(`line-height: normal`)은 registry 항목이 아직 없어 matrix에 없습니다(IX.11.11 이후).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-target-profile` | 통과. |
| `cargo test -p stellar-integration-tests --test web_capability_matrix` | 통과. registry 60항목과 layout longhand 전부에 Web 상태가 있습니다. 표 판정 항목은 대응표 값과 같습니다. override 표의 이름은 모두 registry에 있습니다. E1·E2·E9는 Approximated입니다. |

## 차이(작업 지시와 다른 점)

- "IX.10.5 registry에 등록" 대신 유도 함수입니다(위 상태 설명).

## 남은 위험 / 한계

- stellar-only 출력 규칙 표(`STELLAR_ONLY_OUTPUTS`)는 이 crate의 손 표입니다. 출력기가 생기면 그 규칙과 대조해야
  합니다.
