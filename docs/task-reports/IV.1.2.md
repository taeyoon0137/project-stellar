# 작업 보고서: IV.1.2 제한 YAML 파서(금지 문법 거부)

- WBS: IV.1.2
- 작업명: yaml-rust2 event API 래핑, 금지 문법을 위치 포함 고유 오류로 거부
- 상태: 완료 (`[x]`)
- 선행: IV.1.1(`stellar.yaml` 데이터 구조·`YamlValue`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/yaml/loader.rs` (신규: `RestrictedReceiver`, `load_restricted`, `LoaderError(Kind)`, 회귀 테스트)
- `crates/project/src/yaml/mod.rs` (`loader` 모듈 공개·재노출)
- `crates/project/Cargo.toml` (`yaml-rust2 = "=0.11.0"` 추가)
- `config/dependencies.yaml` (yaml-rust2 purpose를 Phase IV 매니페스트로 확장)
- `docs/task-reports/IV.1.2.md`(신규), 계획서(IV.1.2 `[ ]`→`[x]`)

## 구현 요약

- `yaml-rust2`의 `MarkedEventReceiver` 이벤트를 직접 받아 제한 트리(`YamlValue`)를 만듭니다.
  이벤트 단계에서 `anchor_id != 0`, `tag.is_some()`, `Event::Alias`, `DocumentStart` 카운트,
  `<<` 스칼라 키, 비스칼라 키를 검사해 거부합니다.
- 거부 항목과 고유 오류 코드: `anchor`, `alias`, `custom-tag`, `merge-key`, `non-scalar-key`,
  `multiple-documents`, `empty-document`, `parse`. 각 오류는 line/column 위치를 포함합니다.
- 스칼라는 암시적 타입 추론 없이 문자열로 보존합니다(IV.1.4 대비). `~`·`null`·빈값·따옴표
  스칼라 모두 원문 문자열로 보존됨을 확인했습니다.

## 적대적 검증(ultracode)

`yaml-rust2`의 실제 이벤트 동작이 불확실한 경계 사례를, 6개 lens(anchor/alias/tag/merge-key/
multi-doc/structural)로 적대적 입력 94종을 생성한 뒤 실제 파서로 검증했습니다.

- **이미 올바름으로 확인**: core schema 태그(`!!str`/`!!int`/`!!null`/`!!seq`/`!!map`)·verbatim
  태그·flow 항목 태그 모두 `custom-tag`로 거부. `key: ~`/`null`/빈값/따옴표는 문자열 보존
  (null 타입 누수 없음). `...` document-end 후 새 doc은 다중 document로 거부. `<<`가 값·항목
  이면 문자열 허용, 키면 거부. 단독 `%YAML`/`%TAG` directive와 선행 `---`는 단일 doc로 허용.
- **발견·수정한 gap**: 복합(비스칼라) 매핑 키(`? [a,b] : v`, `? {x:1} : v`)가 수용되던 문제를
  발견해 `non-scalar-key` 거부를 추가했습니다(제한 매니페스트는 스칼라 키만 허용). 컬렉션이
  값으로 오는 정상 경우(중첩 매핑/시퀀스)는 영향 없음을 확인했습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 종료 코드 |
|---|---|
| `cargo test -p stellar-project` (23 unit) | 0 |
| `cargo fmt -p stellar-project -- --check` | 0 |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 0 |
| `cargo run -p xtask -- validate-dependencies` | 0 |

| 금지 문법 fixture | 오류 코드 |
|---|---|
| anchor(스칼라/매핑/시퀀스/flow/빈노드/중첩) | `anchor` |
| alias(정의/미정의/flow/키) | `alias`(미정의는 `parse` 가능) |
| custom·core·verbatim·flow tag | `custom-tag` |
| merge key(block/flow/중복/alias값) | `merge-key` |
| 복합 매핑 키 | `non-scalar-key` |
| 다중 document(`---`·`...`) | `multiple-documents` |
| 빈/주석 only | `empty-document` |

## 차이(작업 지시와 다른 점)

- 지시의 금지 목록(다중 문서·custom tag·anchor·alias·merge key)에 더해, 적대적 검증에서
  드러난 **비스칼라 매핑 키**를 추가로 거부했습니다(제한 매니페스트의 "스칼라 키" 가정 보강).
- `yaml-rust2` 사용 범위를 `config/dependencies.yaml`에서 Phase IV 매니페스트로 확장했습니다
  (이미 승인된 crate·버전; purpose 갱신, validator 통과).

## 남은 위험 / 한계

- 중복 mapping key 거부는 IV.1.3에서 추가합니다(현재는 마지막 값 덮어쓰기 없이 모든 쌍을
  보존하므로, 중복은 IV.1.3이 위치 정보와 함께 거부).
- directive(`%YAML`/`%TAG`)는 사용되지 않으면 허용합니다. `%TAG`로 정의한 handle을 실제로
  사용하면 tag 검사가 `custom-tag`로 거부합니다.
- 따옴표 `"<<"` 키도 `merge-key`로 거부합니다(방어적; 매니페스트에서 `<<` 리터럴 키는 불필요).
