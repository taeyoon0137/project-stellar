# 작업 보고서: X.8.2 FontRegistry 인터페이스와 hash+index dedup

- WBS: X.8.2
- 작업명: `FontRegistry` 인터페이스 정의, 같은 face를 file hash + face index로 dedup
- 상태: 완료 (`[x]`)
- 선행: X.8.1(`FontFaceRef`, 이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. `planning/task-status.yaml`의 required_environments는 `DEV-MAC`만 지정).

## 변경 파일 목록

- `crates/text/src/registry.rs`(신규) — `FontFaceKey`, `FontHandle`, `RegisteredFace`, `FontRegistryError`, `FontRegistry` trait, `InMemoryFontRegistry`, 단위 테스트.
- `crates/text/src/lib.rs` — `pub mod registry;` 선언.
- `docs/task-reports/X.8.2.md`(신규), 계획서(X.8.2 `[ ]`→`[x]`).

## 구현 요약

- **`FontFaceKey`**: face 정체성 = `FontFileHash` + face index입니다. 같은 bytes의 파일은 출처·경로와 무관하게 같은
  face입니다. variation axes는 instance 설정이라 key에 넣지 않습니다(`From<&FontFaceRef>`).
- **`FontRegistry` trait**: `register(source, key)`, `lookup(key)`, `face(handle)`, `len`/`is_empty`, 기본 구현
  `resolve(&FontFaceRef)`를 둡니다. 같은 key를 다시 등록하면 기존 handle을 돌려주고, 다른 출처에서 온 등록은 출처
  집합에만 추가합니다. 플랫폼 font discovery(XIII.6.2)는 이 trait로 face를 등록합니다.
- **`InMemoryFontRegistry`**: 결정론적 기본 구현입니다. handle은 최초 등록 순서대로 0부터 발급하고, key 조회는
  `BTreeMap`, 순회(`iter`)는 handle 순서라 실행마다 결과가 같습니다. 새 face가 `max_faces`를 넘으면
  `CapacityExceeded`를 반환하고 상태를 바꾸지 않습니다. 이미 있는 face 재등록은 한도와 무관하게 성공합니다.
- **`FontHandle`**: 저장하지 않는 런타임 handle입니다. registry는 폰트 bytes를 읽거나 hash를 계산하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`, Apple Silicon arm64)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 종료 코드 0 |
| `cargo test -p stellar-text` | 종료 코드 0 (19 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **duplicate register returns same handle**(`duplicate_register_returns_same_handle`: 같은 key 재등록 → 같은 handle, face 수 1) | 통과 |
| 다른 출처에서 같은 hash+index 등록 → 같은 handle, 출처 집합 `[Project, System]` 기록 | 통과 |
| index 또는 hash가 다르면 다른 handle, 발급 순서 0·1·2와 `iter` 순서 고정 | 통과 |
| `resolve`가 variation axes·요청 출처와 무관하게 같은 face handle, 다른 face index는 `None` | 통과 |
| 미등록 key·미발급 handle 조회 `None`, `&mut dyn FontRegistry` 경유 dedup | 통과 |
| 용량 한도 초과 `CapacityExceeded`, 한도 도달 후 기존 face 재등록 허용 | 통과 |

## 차이(작업 지시와 다른 점)

- registry 기록에 폰트 데이터(파일 bytes·경로)를 넣지 않았습니다. 데이터 접근은 shaping wrapper(X.9.1)와 discovery
  adapter(XIII.6.2)가 정하는 범위라 이 작업에서 공용 필드를 선점하지 않았습니다.
- 용량 오류 경로를 검증하려고 `with_max_faces`를 공개 생성자로 두었습니다. 기본 `new()`는 `u32` handle 공간 전체를
  씁니다.

## 남은 위험 / 한계

- 같은 key가 서로 다른 실제 bytes를 가리키는 경우(호출자가 hash를 잘못 계산한 경우)를 registry가 탐지하지 않습니다.
  hash 계산·검증은 폰트 import·discovery 계층 책임입니다.
- handle은 등록 순서에 따라 달라지므로 저장·플랫폼 간 비교에 쓰면 안 됩니다. 저장에는 `FontFaceRef`를 씁니다.
