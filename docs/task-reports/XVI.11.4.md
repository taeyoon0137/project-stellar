# 작업 보고서: XVI.11.4 외부 library v1·v2 동시 사용→update→override orphan E2E

## WBS

- WBS: XVI.11.4
- 작업명: 외부 library v1·v2 동시 사용→update→override orphan E2E
- 상태: 부분 완료 (`[~]`) — 다중 버전 경고·update 후보·삭제 property까지 제품 경로로 확인했고, override orphan 진단 생산자(XV.7.3)와 적용(XV.7.4)이 제품에 없습니다.
- 선행: XV.8.4
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_library_versions_update.rs`: 시나리오

코드 변경은 테스트뿐입니다(제품 코드 변경 없음).

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-integration-tests --locked` | 0 — 전체 통합 테스트 통과(e2e 6개 파일 포함) |
| `cargo clippy -p stellar-integration-tests --tests --locked` | 0 — 경고 0 |
| `cargo xtask validate-adr --base 682c7b0c` | lane 결과 보고에 적음 |
| `STELLAR_CHECK_BASE=682c7b0c cargo xtask check-core` | lane 결과 보고에 적음 |

## 환경과 증거

- 필수 실행 환경: DEV-MAC
- 환경별 증거: Apple Silicon macOS 로컬 실행, debug build, 시스템 Git. 앱 UI 없이 앱 runtime(`AppRuntime`)과 headless ops만 씁니다.
  테스트 HOME·전역 Git 설정은 테스트 프로세스 임시 폴더로 격리합니다(ADR-0111 결정 2).

## 테스트

| 테스트 | 결과 |
|---|---|
| `two_versions_warning_update_and_removed_override_property` | 통과 — 실제 Git 저장소 C(v1·v2)·A(v1→C v1, v2→C v2)·B(→C v2)와 사용 프로젝트 선언(실제 `stellar.yaml` 파서) → bare mirror fetch·`ls-remote`·commit 매니페스트로 해석: C 두 commit 격리, 경고 경로 `a > c`·`b > c`, 추가 용량 하한(cache 없음 2개), telemetry에 URL 없음 → `prepare_library_update`(A tag v2 재해석): 현재 A v1·후보 A v2, lock 불변 → resource diff `removed_properties` = Card.badge → A v2 선언으로 다시 해석하면 C v2 하나, 경고 없음 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 의존성 crate trait의 제품 구현이 없어 테스트가 얇은 연결을 둡니다: `GitCommandRunner`(git-porcelain `GitRunner`), `RepositoryKeyer`(저장 SHA-256), commit별 export 표(`UpdateSnapshotSource`). library 저장소 파일에서 export를 읽는 공급자가 없기 때문입니다(ADR-0111 결정 7).
- orphan 진단은 테스트에서 따로 만들지 않았습니다(이중 판정 금지). 그 입력인 삭제 property 목록까지 확인합니다.

## 남은 위험

- XV.7.3(후보 library와 instance override 대조 진단), XV.7.4(lock·snapshot 적용), XV.8.4(전체 fixture)와 의존성 trait 제품 구현이 들어오면 테스트 연결을 제품 구현으로 바꾸고 orphan 진단 기대값을 더해야 합니다.
