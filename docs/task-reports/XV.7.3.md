# 작업 보고서: XV.7.3 후보 library 적용 전 external refs·instance override 검증

## WBS

- WBS: XV.7.3
- 작업명: candidate library를 적용하기 전 사용 project의 external refs와 instance overrides를 검증
- 상태: 완료 (`[x]`)
- 선행: IX.4.6(`[x]`), XV.5.2(`[x]`)
- 결정: ADR-0119 결정 5

## 변경 파일

- `crates/dependencies/src/candidate_check.rs`(신규) — `check_candidate_usages`, `ExternalInstanceUsage`, `OverrideUsage`,
  `ExternalTokenUsage`, `CandidateIssue`, `OrphanCause`, `CandidateDiagnostic`, `CandidateCheck`.
- `crates/dependencies/src/library_update.rs`(수정) — `LibraryUpdateCandidate::check_usages`.
- `crates/dependencies/src/read_only.rs`(수정) — `override_type_accepted`(사전 검사와 후보 검증이 함께 쓰는 타입 표기 규칙).
- `crates/dependencies/src/lib.rs`(수정) — 모듈 등록.
- `crates/project/src/library_refs.rs`(신규) — 문서에서 외부 instance 사용 수집(`external_instance_usages`),
  `check_update_candidate`.
- `tests/integration/tests/library_update_orphans.rs`(신규) — 실제 Git library update 후보 종단 시험.
- `docs/task-reports/XV.7.3.md`(신규).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 101 passed, 0 failed; doctest 1 passed |
| `cargo test -p stellar-integration-tests --test library_update_orphans` | 1 passed |
| `cargo test -p stellar-integration-tests --test e2e_library_versions_update` | 1 passed(기존 update 시나리오 회귀 없음) |
| `cargo clippy -p stellar-dependencies -p stellar-project -p stellar-integration-tests --all-targets -- -D warnings` | 경고 0 |
| `cargo xtask validate-adr --base 0c226a06` | 종료 코드 0 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: 없음(DEV-MAC 로컬 시험)

## 테스트

| 검사 | 결과 |
|---|---|
| **removed property override가 orphan diagnostic**: DS v1 → v2 후보(제품 `prepare_library_update` + 제품 `CacheUpdateSource` 연결: `SystemGitCommands`·`ProjectLibraryValidator`·`SourceExports`)에서 사용 프로젝트 Card instance의 `badge` override는 `OrphanOverride(PropertyRemoved)`, Number로 바뀐 `tone` override는 `PropertyRetyped`, 사라진 Notice instance는 `ComponentRemoved`(`removed_property_override_is_orphan_diagnostic_before_candidate_is_applied`) | 통과 |
| 검증은 문서 semantic snapshot과 lock을 바꾸지 않음, 진단에 URL 원문 없음, 현재 버전을 후보로 대면 진단 없음 | 통과 |
| 단위: 비공개 property·`Optional<Number>`가 `Number` 값을 받음·사라진 component·library 변경·token 사라짐/타입 변경·다른 의존성 사용 무시(`candidate_check::tests` 2건) | 통과 |

## 성능

- 해당 없음(사용 수 × override 수에 비례하는 snapshot 조회).

## 차이

- override 판정은 사전 검사(XV.5.3)의 `check_library_edit`을 그대로 쓰고 거부 사유를 orphan 원인으로 옮깁니다. resource diff(XV.7.2)로
  따로 판정하지 않습니다(이중 판정 금지).
- 문서가 외부 참조를 저장하지 않으므로 instance의 외부 component는 `classify_component` 규칙(ADR-0119 결정 4)으로 정합니다. 외부 token
  참조는 문서에 없어 사용 수집은 instance만 하고, token 판정 API는 의존성 crate에 둡니다.
- nested 경로 override는 외부 component의 직접 property가 아니므로 사용에 넣지 않습니다(IX.4.2).

## 남은 위험

- 사용자 승인 뒤 lock·in-memory snapshot 원자 갱신과 override 재적용(XV.7.4)은 남았습니다. 이 검증 결과가 그 승인 화면의 입력입니다.
- 두 직접 의존성이 같은 component를 export하는 모호한 instance는 후보 판정에서 빠지고 따로 돌려줍니다.
