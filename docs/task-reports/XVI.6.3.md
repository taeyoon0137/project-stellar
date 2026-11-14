# 작업 보고서: XVI.6.3 document loader/validator fuzz target

- WBS: XVI.6.3
- 작업명: document loader/validator fuzz target 추가(invalid refs/cycles 처리, panic 0)
- 상태: 완료 (`[x]`)
- 선행: VI.10.3(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, debug 빌드 `cargo test` + release 장기 실행).

## 변경 파일 목록

- `crates/document/src/test_support/loader_fuzz.rs`(신규, 테스트 전용) — 결정론적 fuzz harness(원본 생성, byte·레코드 변형,
  대상 실행·불변식 검사, 통계)와 `#[test]` 3개.
- `crates/document/src/test_support/mod.rs` — 모듈 등록.
- `docs/task-reports/XVI.6.3.md`(신규), 계획서(XVI.6.3 `[ ]`→`[x]`).

## 구현 요약

XVI.6.1·XVI.6.2와 같은 방식(고정 seed SplitMix64, `#[test]`, 새 의존성·nightly 없음)입니다. document crate는 통합 테스트에서
`test_support`(random valid document 생성기)를 쓰려면 자기 자신을 dev-dependency로 걸어야 해서, harness를 `test_support`
모듈(프로덕션 빌드 제외)에 두고 unit test로 실행합니다.

- **원본**: random valid document(VI.10.5) 12개의 레코드와 정의·node·View subtree bytes, IX.9.1 컴포넌트 library·contract
  표·drilling 모음 bytes, IX.9.3 View variant model bytes. campaign은 원본 전체(수용 경로 보장) 뒤에 변형 입력을 붙입니다.
- **byte 변형**(입력당 1~8회, 64 KiB 상한): bit 뒤집기, 삽입, 구간 삭제, `u32` 경계값·NaN·∞·비 UTF-8 token 덮어쓰기·삽입,
  구간 복제, 다른 원본 splice, 절단, 경계 byte, 구간 교환. 20%는 다른 decoder에 넣어 형식 혼동 입력을 만듭니다.
- **레코드 변형**(입력당 1~3회): 자기 부모, 다른 node를 부모로(cycle), 2-cycle, 없는 부모, extra root, node 삭제(dangling),
  레코드 중복·NIL id, 형제 OrderKey 중복, View root를 없는 id·비 Frame으로, 63·64·65·200·5,000단계 chain, modifier stack
  소유자 바꾸기·중복, 자식 있는 node를 Text로, 레코드 순서 섞기.
- **대상**: `DocumentDefinition::decode`, `decode_node_record`, `SubtreeRecords::decode` → 단일 View 레코드 →
  `validate_document` → `DocumentModel::from_records`, `ComponentLibrary::decode`, `DrillingSet::decode`,
  `ControlContractTable::decode`, `ViewVariantModel::decode`, 레코드 변형은 validator·loader 직접.
- **판정**: panic(`catch_unwind`), hang(8 MiB stack worker thread + 60 s 진행 없음 watchdog), 느린 입력(입력당 3 s), 불변식 —
  validator가 통과시킨 레코드는 loader가 반드시 모델을 만들고(`LoadModelError::Model` = 버그 신호), 그 `to_records`는 재검증
  통과·canonical 정렬 입력과 동일·View subtree 분할/재결합 동일, validator 거부 시 loader도 거부, 수용 bytes 재인코딩 round-trip
  (canonical decoder는 bytes 동일).
- 모든 byte 대상의 수용·거부 경로 실행, 레코드 통과·거부 모두 발생, `ParentCycle`·`MissingParent`·`DepthExceeded`·`DuplicateId`
  위반이 실제로 생성·거부됐는지 확인합니다. 반복 수 `STELLAR_DOCUMENT_FUZZ_ITERATIONS`(기본 1,500), seed
  `STELLAR_DOCUMENT_FUZZ_SEED`.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-document` | 완료 |
| `cargo test -p stellar-document` | unit 351 passed, 통합 7+1+1, doc 2 passed, 0 failed |
| `STELLAR_DOCUMENT_FUZZ_ITERATIONS=50000 cargo test --release -p stellar-document --lib loader_and_validator_fuzz` (seed 기본값·1·2·3, 각 5만 입력) | 4회 모두 passed, 발견 0 |
| `cargo clippy -p stellar-document --all-targets -- -D warnings` | 경고 0 |
| `cargo clippy -p stellar-document --features test-support -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-document -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| campaign(원본 + 1,500 변형): panic·hang·느린 입력·불변식 위반 0, 7개 byte 대상 수용·거부 모두 실행, 레코드 통과·거부 모두 발생, cycle·dangling·깊이·중복 위반 생성(`loader_and_validator_fuzz_campaign_has_zero_panics`) | 통과 |
| 같은 seed 같은 입력열, 다른 seed 다른 입력열(`campaign_inputs_are_deterministic`) | 통과 |
| 고정 입력: self cycle → `ParentCycle`, 2-cycle 거부, 없는 부모 → `MissingParent`, 없는 View root → `MissingRootFrame`, 64단계 통과·65단계와 2만 단계 → `DepthExceeded`(stack overflow 없음), 없는 stack 소유자 → `ModifierOwnerMissing`, id 중복 → `DuplicateId`, 모두 loader 불변식 유지(`handcrafted_invalid_refs_and_cycles_are_rejected_without_panic`) | 통과 |

## 차이(작업 지시와 다른 점)

- harness 위치: XVI.6.1·6.2는 `crates/<crate>/tests/`이지만, 이 작업은 `test_support`의 random document 생성기를 써야 해서
  `crates/document/src/test_support/loader_fuzz.rs`(unit test)에 두었습니다. 발견 입력은 파일로 저장하지 않고 실패 메시지(입력 앞
  64 bytes hex·레코드 요약)로 보고합니다. seed corpus는 파일 대신 생성기로 만듭니다.
- 과도한 할당 계측(전역 할당자)은 unit test 바이너리 전체에 영향을 주므로 넣지 않았습니다(이 작업 완료 기준에는 없음).

## 남은 위험 / 한계

- fuzz 발견은 0건이라 제품 코드 수정은 없습니다. 디버그 빌드에서 4만 node 문서의 validate+load가 약 1~2 s(선형 증가 확인)라
  campaign의 chain 길이는 5,000으로 제한하고 2만 단계는 고정 테스트에서만 다룹니다.
- 장기 nightly 실행(XVI.6.7류)과 corpus 파일화는 후속입니다.
