# 작업 보고서: X.8.3 명시적 font fallback chain 모델

- WBS: X.8.3
- 작업명: explicit fallback chain 모델 정의 — missing face를 임의 시스템 font로 바꾸지 않음
- 상태: 완료 (`[x]`)
- 선행: X.8.2(`FontRegistry`, 이 lane에서 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증. `planning/task-status.yaml`의 required_environments는 `DEV-MAC`만 지정).

## 변경 파일 목록

- `crates/text/src/fallback.rs`(신규) — `FontFallbackChain`, `FallbackChainError`, `ResolvedFallbackChain`, `FallbackSelection`, 단위 테스트.
- `crates/text/src/lib.rs` — `pub mod fallback;` 선언.
- `docs/task-reports/X.8.3.md`(신규), 계획서(X.8.3 `[ ]`→`[x]`).

## 구현 요약

- **`FontFallbackChain`**: 비어 있지 않은 `FontFaceRef` 목록이며 첫 항목이 primary입니다. 기록 순서를 그대로
  보존합니다. 같은 face instance(file hash + face index + variation axes, 출처 무관)가 두 번 나오면
  `DuplicateFace { first, duplicate }`로 거부합니다. registry가 출처와 무관하게 dedup하므로 두 번째 항목은 언제나
  첫 항목과 같은 결과가 되기 때문입니다. 같은 face라도 axes가 다르면 다른 instance로 허용합니다.
- **canonical binary**: `u32 LE 항목 수` + 항목별 `FontFaceRef` canonical bytes(X.8.1). decode는 빈 chain·중복·
  잘린 입력·잔여 bytes를 거부하고, 큰 항목 수는 남은 입력 길이까지만 예약합니다.
- **`resolve(&registry)`**: 각 항목의 등록 여부만 계산합니다. `available()`·`missing()`은 체인 순서이고, 체인 밖
  face는 registry에 있어도 결과에 나타나지 않습니다.
- **`select(accepts)`**: 사용 가능 항목을 체인 순서로 검사해 조건(이후 X.9.5의 glyph coverage)을 처음 만족하는 항목의
  위치와 handle을 돌려줍니다. 없으면 `FallbackSelection::NoFace`이며 임의 시스템 font로 대체하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`, Apple Silicon arm64)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-text` | 종료 코드 0 |
| `cargo test -p stellar-text` | 종료 코드 0 (25 unit passed, 0 failed) |
| `cargo clippy -p stellar-text --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-text -- --check` | 종료 코드 0 |

| 검사 | 결과 |
|---|---|
| **fallback order fixture**(`fallback_order_fixture`): A(누락)→B→C→D(누락)→E에서 available `[(1,B),(2,C),(4,E)]`, missing `[(0,A),(3,D)]`, primary 누락 시 B 선택, 조건 {C,E,Z}에서 C 선택, 조건 {E}에서 E 선택 | 통과 |
| registry에 있고 조건도 만족하는 체인 밖 Z는 선택되지 않음, 전부 누락 체인은 `NoFace`(`missing_face_is_never_replaced_by_unlisted_system_font`) | 통과 |
| 전부 사용 가능 체인은 primary 선택 | 통과 |
| 체인 순서 encode/decode round-trip, 단일 항목 golden bytes | 통과 |
| 빈 chain 거부, 출처만 다른 같은 face 중복 거부, axes가 다른 같은 face 허용(체인 순서상 첫 항목 선택) | 통과 |
| decode의 빈 chain·truncation·trailing·`u32::MAX` 개수·중복 항목 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 선택 조건은 closure로 받습니다. 실제 glyph coverage(cmap) 판정은 폰트 데이터와 shaping wrapper가 필요해 X.9.5에서
  연결합니다. 이 작업은 순서·대체 금지 규칙만 고정합니다.

## 남은 위험 / 한계

- 체인 길이 상한은 두지 않았습니다. 문서 단위 상한이 필요하면 문서 limit 정책(config/document-limits.yaml)과 함께
  별도 작업에서 정합니다.
- 체인을 문서 style·typography에 저장하는 연결(어느 style이 어떤 chain을 쓰는지)은 `TypedValue::Typography` 정식화와
  함께 후속 작업 범위입니다.
