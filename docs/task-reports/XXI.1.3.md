# 작업 보고서: XXI.1.3 서명 gate 분리와 unsigned local plugin 개발 모드 한정

- WBS: XXI.1.3
- 작업명: plugin signature/registry는 별도 gate로 두고 unsigned local plugin은 개발 mode에서만 허용
- 상태: 완료 (`[x]`)
- 선행: XXI.1.1
- 결정: [ADR-0097](../adr/ADR-0097-plugin-host-core.md) 결정 4

## 필수 실행 환경

- `DEV-MAC` (Rust, 로컬 검증).

## 변경 파일 목록

- `crates/plugin-host/src/admission.rs`(신규) — `LoadPolicy`, `SignatureGate`, `NoSignatureRegistry`, `admit`, `AdmittedPlugin`.
- `crates/plugin-host/src/lib.rs` — module 등록.
- `crates/plugin-host/tests/admission_gate.rs`(신규).

## 구현 요약

- 서명 판정은 `SignatureGate::verify(package) -> Verified | Unsigned | Invalid`로 분리했습니다. registry가 아직 없으므로
  `NoSignatureRegistry`는 `plugin.sig`가 없으면 unsigned, 있으면 확인할 수 없어 invalid입니다.
- `admit`: 검증됨은 모든 모드에서 허용, unsigned는 development에서만 허용(`UnsignedDevelopment` 표시), invalid는 항상 거부합니다.
- `LoadPolicy::default()`는 release입니다. `AdmittedPlugin`은 `admit`으로만 만들 수 있어 Host session이 gate를 건너뛸 수 없습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-plugin-host --test admission_gate` | 5 passed |
| `cargo clippy -p stellar-plugin-host --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| release 모드 unsigned load 거부(`unsigned-plugin-in-release`) | 통과 |
| development 모드 unsigned local plugin 허용 | 통과 |
| 확인할 수 없는 서명은 두 모드 모두 거부 | 통과 |
| digest를 확인한 registry gate는 release에서 허용, digest 불일치는 거부 | 통과 |

## 남은 위험 / 한계

- 실제 서명 형식과 registry는 정하지 않았습니다(별도 gate 자리만 둠). 배포 전 registry를 정해야 release에서 플러그인을 쓸 수 있습니다.
- 개발자 모드를 켜는 사용자 설정은 앱 UI 재설계 때 연결합니다.
