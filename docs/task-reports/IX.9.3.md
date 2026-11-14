# 작업 보고서: IX.9.3 compact/error/dark nullable Variant 조합 fixture

- WBS: IX.9.3
- 작업명: compact/error/dark nullable variant 조합 fixture
- 상태: 완료 (`[x]`)
- 선행: IX.7.6(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/document/src/test_support/variant_fixtures.rs`(신규, 테스트 전용) — checkout View fixture(`checkout_fixture`:
  변수 3개·Variant 6개·base 속성 7개·조합 case 10개), `resolution_snapshot`(결정론적 JSON), 검증 테스트 4개.
- `crates/document/src/test_support/fixtures/variant_combinations.json`(신규) — case별 resolve 결과 golden.
- `crates/document/src/test_support/mod.rs` — 모듈 등록.
- `docs/task-reports/IX.9.3.md`(신규), 계획서(IX.9.3 `[ ]`→`[x]`).

## 구현 요약

- **변수**: `device`(compact|regular), `paymentState`(idle|processing|error|completed, nullable — 실제 null은
  `ExplicitNull`), `dark`(Boolean).
- **Variant**: compact, error, dark, noPayment(payment = ExplicitNull), compactError, compactErrorDark. 모두 IX.7.8
  command로 만들고 orphan 진단 0건입니다.
- **case 10개**: default, compact, error, dark, compact+error, compact+dark(정의되지 않은 조합), error+dark(같은 구체성
  충돌), compact+error+dark(구체성 3이 충돌 해소), compact+processing(일치 조합 없음), payment=null(ExplicitNull).
- **snapshot**: case마다 일치 Variant 순서, 속성별 값·출처(base/Variant 이름), 충돌(속성·구체성·Variant)을 ASCII JSON으로
  출력해 golden과 byte 비교합니다(`STELLAR_UPDATE_SNAPSHOT_FIXTURES=1`로 갱신, VI.10.4와 같은 방식). IX.9.4의 플랫폼 간
  비교 입력입니다.
- `test_support` 모듈이라 프로덕션 빌드에 포함되지 않습니다(`cfg(test)`/`test-support` feature).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1 cargo test -p stellar-document --lib variant_fixtures` | golden 최초 생성, 4 passed |
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 344 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 변수 3·Variant 6, orphan 진단 0, 모든 case 할당 유효, model round-trip(`fixture_model_is_valid_and_complete`) | 통과 |
| 예상 resolution: compact+error에서 compactError가 banner.height 덮어씀, compact+dark는 일반 Variant만·충돌 없음, compact+error+dark에서 compactErrorDark가 card.fill 결정, compact+processing은 compact만, ExplicitNull은 noPayment만 일치(`expected_override_resolution_per_combination`) | 통과 |
| ambiguity 진단: error+dark에서 card.fill 충돌 1건(구체성 1, [error, dark]), 충돌 속성은 base 유지·나머지 적용, 다른 case 충돌 0(`ambiguity_diagnostic_for_same_specificity_conflict`) | 통과 |
| snapshot 결정론·ASCII·golden byte 일치(`resolution_snapshot_matches_fixture_bytes`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. golden은 코드로 생성한 뒤 명시적 기대값 테스트(위 2·3행)로 내용을 확인했습니다.

## 남은 위험 / 한계

- 플랫폼 간(macOS·Windows) byte 비교는 IX.9.4 범위이며 Windows는 ADR-0006에 따라 보류입니다.
