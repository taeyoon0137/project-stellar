# ADR-0020: 에이전트 도구 계층 외부 의존성 승인

## Status

Accepted (2026-11-14). 제품 소유자가 2026-11-14에 잘 관리되는 MIT·Apache 라이선스 외부 crate를 필요할 때 추가하도록 위임한 범위에서 정했습니다(ADR-0019 Decision 4의 "외부 crate가 필요하면 `config/dependencies.yaml`과 별도 ADR로 추가").

## Context

- ADR-0005 Consequences는 JSON 직렬화, JSON-RPC 프레이밍, IPC 상대 인증용 OS binding, 필요하면 표시 서식 해석을 승인 목록에 등록하라고 요구합니다. 계획서 XXIX.1.2가 이 등록을 맡습니다.
- 계획서 1.2 규칙 14에 따라 외부 의존성은 `config/dependencies.yaml` 승인 항목만 허용합니다.
- 이 결정 전 `validate-dependencies`는 `xtask`의 직접 의존만 검사했습니다. 제품 crate(예: `stellar-project`의 `yaml-rust2`)의 외부 의존은 검사 범위 밖이었습니다.
- 승인 목록은 쓰이지 않는 approved 항목을 `unused-approval`로 거부합니다. 도입 작업보다 먼저 승인하려면 도입 예정임을 적을 방법이 필요합니다.
- 현재 L1 `tools/stellar-cli`와 L2 골격 `tools/stellar-mcp`는 외부 crate 없이 strict JSON reader·writer를 각각 따로 두고 있고(약 770행씩 거의 같은 복사본), macOS 시스템 호출은 `extern "C"`로 직접 선언합니다. XIX.9(agent-protocol·MCP library), XXIV.4(`stellar` JSON 입출력), XXIX.2(구독 CLI 구조화 출력 디코드)가 더해지면 같은 종류의 코드가 더 늘어납니다.

## Decision

1. **승인 항목**: 다음 crate를 approved로 등록합니다. 버전·라이선스·공개일은 2026-11-14 crates.io에서 확인했습니다.

   | crate | 버전 | 라이선스 | 공개일 | 용도 | 도입 예정 행(`planned_for`) |
   |---|---|---|---|---|---|
   | `serde` | 1.0.229 | MIT OR Apache-2.0 | 2026-07-18 | 에이전트 도구 계층 메시지·출력 타입 직렬화 derive | XIX.9.1, XIX.9.2, XXIV.4.3, XXIV.4.4, XXIX.2.10 |
   | `serde_json` | 1.0.154 | MIT OR Apache-2.0 | 2026-10-11 | MCP JSON-RPC 메시지, `stellar` JSON 입출력, 구독 CLI 구조화 출력 해석 | XIX.9.1, XIX.9.2, XXIV.4.3, XXIV.4.4, XXIX.2.11 |
   | `libc` | 0.2.191 | MIT OR Apache-2.0 | 2026-10-16 | macOS IPC 상대 인증용 시스템 호출 binding | XIX.1.6, XIX.1.7 |
   | `security-framework` | 3.7.0 | MIT OR Apache-2.0 | 2026-02-20 | IPC 상대와 구독 CLI 실행 파일의 코드 서명 신원 확인(Security.framework binding) | XIX.1.6, XIX.1.7, XXIX.2.3 |

2. **JSON-RPC 프레이밍**: 별도 crate를 승인하지 않습니다. MCP stdio transport는 줄바꿈으로 구분한 JSON-RPC 메시지이고, 로컬 IPC도 같은 방식을 쓸 수 있으므로 표준 라이브러리 줄 단위 읽기와 크기 상한, `serde_json`으로 구현합니다.
3. **표시 서식 해석**: 지금은 승인하지 않습니다. 채팅 보기의 서식 범위는 XXIX.4.3이 정합니다(ADR-0005 §4.4). 해석기가 필요하면 그 작업에서 승인 항목과 ADR을 추가합니다.
4. **사용 범위**: 기계 검사가 아니라 이 ADR과 리뷰로 지킵니다.
   - `serde`·`serde_json`은 에이전트 도구 계층(L1 `stellar` JSON 입출력 포함)과 그 protocol 타입에만 씁니다. 프로젝트 파일 포맷, storage canonical encoding, content hash 입력에는 쓰지 않습니다.
   - `libc`·`security-framework`는 macOS IPC 상대 인증과 실행 파일 신원 확인을 맡는 crate에서만 씁니다. 사용할 OS API와 인증 기준은 XIX.1.6, XIX.1.7, XXIX.2.3이 Apple 공식 문서를 근거로 정합니다(ADR-0005 §5, §4.3). 코어 문서·저장 crate에서는 쓰지 않습니다.
5. **검사 범위**: `cargo xtask validate-dependencies`는 workspace 모든 member의 외부 직접 의존(normal·build·dev, 모든 target, 모든 feature)을 검사합니다. 목록은 `cargo tree --workspace --all-features -e normal,build,dev --target all --depth 1 --prefix depth --no-dedupe` 출력에서 workspace member 자신(이름·버전·출처가 같은 package)을 뺀 깊이 1 package입니다. `--all-features`가 없으면 기본 feature로 켜지지 않는 optional 의존이 목록에서 빠지므로 필수입니다(2026-11-14 보완, 인자는 `tools/xtask/src/dependencies.rs`의 `CARGO_TREE_ARGS`). 출력 형식이 예상과 다르면 실패합니다.
6. **도입 예정 표기(`planned_for`)**: 승인 항목의 선택 필드입니다. 값은 쉼표로 구분한 WBS ID이고 approved 항목에만 쓸 수 있습니다.
   - 어떤 member도 쓰지 않는 approved 항목은 `planned_for`가 있을 때만 통과합니다.
   - `planned_for` 행이 계획서에 없으면 `unknown-planned-for`로 실패합니다.
   - `planned_for` 행이 모두 `[x]`인데도 쓰이지 않으면 `stale-planned-approval`로 실패합니다. 그 crate를 쓰지 않기로 한 작업은 항목을 지우거나 도입 행을 고칩니다.
   - 쓰이기 시작한 항목의 `planned_for`는 남겨 두어도 됩니다. 병렬 작업이 승인 목록을 함께 고치지 않게 하기 위해서입니다.
7. **버전과 feature**: 도입 작업은 manifest에서 `=` exact 버전으로 고정합니다(`yaml-rust2` 관례). 승인 버전과 다른 버전을 쓰면 같은 변경에서 `version` 필드를 갱신합니다. default feature는 필요한 것만 켭니다. `security-framework`의 TLS 관련 기능은 쓰지 않습니다.

## Alternatives

- **계속 직접 구현**: 외부 의존이 없지만, 보안에 민감한 JSON parser를 도구마다 복사해 유지해야 하고 XIX.9·XXIV.4·XXIX.2에서 복사본이 더 늘어납니다. 채택하지 않습니다. 기존 `tools/stellar-cli`·`tools/stellar-mcp` 구현을 교체할지와 시점은 XIX.9.1 이후 구현 작업이 정합니다.
- **JSON-RPC framework crate**(async runtime·network transport 포함): Stellar는 stdio 줄 단위 메시지와 로컬 IPC만 쓰고 network port를 열지 않습니다(ADR-0005 §5). 필요 이상의 runtime과 transport를 들여오므로 채택하지 않습니다.
- **`approval: pending`으로 등록**: pending은 아직 승인하지 않았다는 뜻이고, 도입 작업마다 승인 목록을 고쳐야 해 병렬 작업끼리 충돌합니다. 채택하지 않습니다.
- **crate별 허용 범위 필드**(어느 member가 쓸 수 있는지까지 기계 검사): 경계는 더 좁아지지만 도입 crate 이름과 배치가 XIX.9.1·XXIX.2.1에서야 정해집니다. 이번에는 채택하지 않고 Decision 4를 리뷰로 지킵니다.
- **다른 macOS binding**: 실제 인증 방법은 XIX.1.6이 정합니다. 승인한 crate로 부족해 다른 binding이 필요하면 그 작업에서 승인 항목과 ADR을 추가합니다.

## Consequences

- 긍정: XIX.9, XXIV.4, XXIX 구현 작업이 승인 목록을 고치지 않고 의존을 추가할 수 있습니다.
- 긍정: 제품 crate와 도구 crate의 외부 의존이 모두 승인 검사 범위에 들어갑니다.
- 부정: 승인한 crate를 끝내 쓰지 않으면 도입 행이 모두 완료되는 시점에 `validate-dependencies`가 실패하므로, 그 작업이 항목을 정리해야 합니다.
- 부정: Decision 4의 사용 범위는 기계 검사하지 않습니다.
- 확인 필요: `validate-dependencies`는 직접 의존만 검사합니다. 도입 작업은 추가되는 전이 의존의 라이선스를 `Cargo.lock` 기준으로 확인해 보고서에 기록합니다.

## Compatibility

파일 포맷, 공개 FFI, 저장 구조에 영향이 없습니다. `config/dependencies.yaml`에 선택 필드 `planned_for`가 추가되며, 기존 항목은 그대로 유효합니다.

## Performance

핵심 입력·렌더링 경로에 영향이 없습니다. `serde` derive를 쓰는 crate는 proc-macro 의존 때문에 컴파일 시간이 늘어납니다. `validate-dependencies`는 `cargo tree`를 한 번 실행합니다.

## Security

- 승인은 crate 단위 공급망 신뢰입니다. 버전은 exact로 고정하고 `Cargo.lock` checksum으로 확인합니다.
- 신뢰하지 않는 JSON 입력은 framing 단계에서 크기 상한을 먼저 적용합니다. `serde_json`의 기본 재귀 깊이 제한을 없애는 `unbounded_depth` feature는 켜지 않습니다. 중복 key 처리 같은 strictness 요구는 구현 작업이 정해 보고서에 기록합니다.
- `libc`·`security-framework` 호출은 unsafe FFI 경계입니다. 호출부는 해당 crate의 좁은 모듈에 모으고, 판정할 수 없으면 실패로 처리합니다(fail closed).
- 이번 승인 목록에는 network 기능을 쓰는 crate가 없습니다.

## Migration

없습니다. 도입 작업이 manifest에 의존을 추가하는 시점부터 적용됩니다.
