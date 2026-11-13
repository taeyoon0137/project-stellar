# 작업 보고서: VI.10.4 테스트 전용 canonical semantic snapshot JSON

- WBS: VI.10.4
- 작업명: canonical semantic snapshot JSON을 테스트 전용으로 생성(프로덕션 저장 미사용)
- 상태: 완료 (`[x]`, ADR-0006 규칙 4에 따라 macOS 증거)
- 선행: VI.10.3

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). Windows 비교는 ADR-0006으로 보류.

## 변경 파일 목록

- `crates/document/src/test_support/mod.rs`(신규) — 테스트 전용 모듈(`#[doc(hidden)]`).
- `crates/document/src/test_support/json.rs`(신규) — 결정론적 JSON 값·pretty 출력기.
- `crates/document/src/test_support/semantic_snapshot.rs`(신규) — `semantic_snapshot`,
  `semantic_snapshot_json`, `model_snapshot_json`, `SNAPSHOT_FORMAT`·`SNAPSHOT_VERSION`과 테스트.
- `crates/document/src/test_support/fixtures/semantic_snapshot_basic.json`(신규, 11,444 bytes).
- `crates/document/src/lib.rs` — `#[doc(hidden)] pub mod test_support;`.
- `docs/task-reports/VI.10.4.md`(신규), 계획서(VI.10.4 `[ ]`→`[x]`).

## 구현 요약

- **테스트 전용 경계**: 다른 crate 통합 테스트(XVI.5.5 reload 비교 등)도 쓸 수 있게 `cfg(test)`가
  아닌 숨김 공개 모듈로 두고, 이 crate의 비테스트 코드(각 파일 첫 `#[cfg(test)]` 앞)가
  `test_support`를 참조하지 않음을 소스 검사 테스트로 고정했습니다. 저장 형식은 chunk model
  (VI.10.2)뿐입니다. 새 의존성은 없습니다.
- **JSON 규칙**: object key는 `BTreeMap` 사전순, 출력은 ASCII만(제어 문자·`"`·`\` escape, U+007F
  이상은 `\uXXXX`, BMP 밖은 surrogate pair), 2칸 들여쓰기·LF·마지막 줄바꿈 1개. 숫자는 정수 또는
  f32 Rust `Display`(최단 왕복 10진, 지수 없음), `-0`은 `0`. 저장소 `.gitattributes`
  (`* text=auto eol=lf`)로 fixture는 체크아웃 플랫폼과 무관하게 LF입니다.
- **snapshot 내용**: 문서 정의(페이지·View 순서), node(StableId 순: kind, parent, order key hex,
  visible, locked, 이름, 종류별 payload 저장값), modifier stack(소유 node 순, 항목은 stack 순서).
  runtime 핸들·캐시는 포함하지 않습니다. 모든 enum은 wildcard 없는 match로 이름을 붙여 variant
  추가 시 컴파일이 실패합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1 cargo test -p stellar-document test_support` | fixture 최초 생성(7 passed) |
| `cargo test -p stellar-document` | 186 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 고정 fixture 문서(모든 node 종류·Shape 4종·modifier 12종·다국어/emoji/escape 문자) snapshot이 체크인 fixture와 byte-identical, ASCII(`fixture_snapshot_is_byte_identical`) | 통과 |
| 레코드 순서 뒤집기·모델 재구축(새 runtime 핸들) 후에도 snapshot 동일(`snapshot_is_independent_of_record_order_and_runtime_history`) | 통과 |
| 이름·표시 여부·order key·modifier enabled 변경이 snapshot에 반영(`snapshot_reflects_every_semantic_change`) | 통과 |
| 프로덕션 코드의 test_support 참조 0(`production_code_does_not_use_test_support`) | 통과 |
| key 정렬·escape·float 표기 단위 테스트(`json::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- "양 플랫폼 fixture diff byte-identical"은 ADR-0006 규칙 4를 적용했습니다: snapshot bytes는
  정수 연산, 저장된 f32 값의 Rust core 최단 왕복 10진 변환, 고정 escape 규칙만으로 정해지며
  OS API·libm 초월함수를 쓰지 않으므로 macOS 증거와 고정 golden fixture로 `[x]` 처리했습니다.

## 남은 위험 / 한계

- Windows 재개 시 같은 fixture 테스트를 Windows CI에서 실행해 byte-identical을 재확인해야 합니다.
- fixture 갱신은 `STELLAR_UPDATE_SNAPSHOT_FIXTURES=1`로만 합니다. 의도하지 않은 갱신을 막으려면
  리뷰에서 fixture diff를 확인해야 합니다.

## 후속 수정 (리뷰 minor: test_support의 프로덕션 빌드 포함)

- **결함**: `test_support`가 숨김 공개 모듈이라 프로덕션 빌드에도 컴파일됐고, "프로덕션 미사용" 소스 검사는 이 crate의
  `src`만 봤습니다.
- **수정**: `crates/document/Cargo.toml`에 기본값이 꺼진 `test-support` feature를 추가하고 모듈을
  `#[cfg(any(test, feature = "test-support"))]`로 gate했습니다. 이를 쓰는 `crates/project` 통합 테스트는
  `[dev-dependencies]`에서만 feature를 켭니다. `cargo tree -e normal,features`에서 stellar-project의 일반 edge는
  `default` feature만, dev edge만 `test-support`를 켭니다.
- **검사 확장**: `workspace_production_code_does_not_use_test_support`가 root `Cargo.toml`의 모든 member(crates/*,
  tools/*)의 비테스트 소스에서 `stellar_document::…test_support`(use group 포함) 참조가 없는지, 어떤 manifest도
  `[dependencies]`·`[build-dependencies]`·`[features]`에서 `test-support`를 켜지 않는지 검사합니다(scanner positive
  control 포함). lib.rs 선언의 gate 형태와 feature가 default가 아님도 고정합니다.

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-document` | unit 244 포함 전부 passed, 0 failed |
| `cargo test -p stellar-project` | unit 168, `document_residency` 3, `document_round_trip` 4, `document_serialize` 4, `restricted_yaml_fuzz` 4 passed, 0 failed |
| `cargo clippy -p stellar-document -p stellar-project --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -p stellar-project -- --check` | 통과 |
| `cargo check --workspace` | 통과(프로덕션 빌드에서 test_support 없이 컴파일) |
| `cargo tree -p stellar-project -e normal,features -i stellar-document` | `default` feature만 |
