# ADR-0001: Rust 도구체인 baseline 고정

## Status

Accepted

## Context

Project Stellar는 macOS와 Windows에서 결정론적인 빌드·테스트 결과를 요구합니다.
부유(floating) toolchain은 플랫폼·시점에 따라 결과가 달라져 결정성과 재현성을 해칩니다.
Phase I(I.1.6)에서 `rust-toolchain.toml`과 `config/versions.yaml`에 Rust `1.96.0`을 고정했고,
이를 제품 코어의 공식 baseline으로 승격합니다.

## Decision

- 공식 baseline·MSRV(minimum supported Rust version)는 Rust `1.96.0`입니다.
- 단일 출처는 `rust-toolchain.toml`의 `channel`이며, `config/versions.yaml`의 `rust.exact`가 이와
  정확히 일치해야 합니다(`cargo xtask validate-versions`가 강제).
- profile은 `minimal`, component는 `rustfmt`, `clippy`로 고정합니다.
- 버전은 자동으로 변경하지 않습니다. 업그레이드는 전용 WBS(아래 Migration)로만 수행합니다.

## Alternatives

- 부유 stable(`channel = "stable"`): 시점에 따라 버전이 바뀌어 결정성 위반 — 미선택.
- 개발자별 rustup default 사용: 환경마다 달라 재현 불가 — 미선택.

## Consequences

- 모든 환경(DEV-MAC, CI-MAC, CI-WIN, LAB-WIN)이 동일 release·commit-hash를 사용합니다.
- toolchain 변경은 추적 가능한 단일 WBS로 한정되어 의도치 않은 drift가 차단됩니다.
- 새 Rust 기능 채택은 baseline 업그레이드 WBS 이후에만 가능합니다.

## Compatibility

- 파일 포맷·공개 API·FFI에 직접 영향 없음. 단, 새 언어 기능 사용은 baseline 버전에 의해 제한됩니다.

## Performance

- 고정 toolchain은 컴파일러 최적화 동작을 안정화해 성능 측정의 비교 가능성을 보장합니다.

## Security

- 고정 버전은 알려진 취약점 대응 시 명시적 업그레이드 WBS를 통해서만 갱신되어, 변경 이력이 감사 가능합니다.

## Migration

toolchain 업그레이드 절차(전용 WBS):

1. 새 release를 `rust-toolchain.toml`의 `channel`과 `config/versions.yaml`의 `rust.exact`에 동시에 기록합니다.
2. 본 ADR을 새 ADR로 supersede 하거나 갱신해 변경 사유를 남깁니다.
3. `CI-MAC`·`CI-WIN`에서 `rustc -Vv`의 release와 commit-hash가 동일함을 재검증합니다.
4. 전체 workspace `fmt`·`clippy`·`test`가 새 버전에서 통과함을 확인합니다.
