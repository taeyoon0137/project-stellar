# 작업 보고서: XVI.11.2 객체 속성 부분 stage→commit→빈 GitHub fixture remote 연결→push E2E

## WBS

- WBS: XVI.11.2
- 작업명: 객체 속성 부분 stage→commit→빈 GitHub fixture remote 연결→push E2E
- 상태: 완료 (`[x]`) 원장 판정은 통합 때 `generate-ledger`로 정합니다.
- 선행: Phase XIV
- 결정: [ADR-0111](../adr/ADR-0111-headless-release-e2e-scenarios.md)
- lane: `headless-e2e`

## 변경 파일

- `tests/integration/tests/e2e_partial_stage_push.rs`: 시나리오와 URL 검증 단위

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
| `github_url_validation_is_a_pure_unit_without_git` | 통과 — HTTPS·SCP·SSH URL 형식과 자격 증명·http·port·query·추가 경로·owner·repo·SSH 사용자 거부, 비 GitHub 분류(Git 실행 없음) |
| `partial_property_stage_commit_connect_empty_github_remote_and_push` | 통과 — 앱 속성 패널로 같은 Frame의 `width`·`background-color` 편집 → semantic 상태에 두 속성 변경 → `width` 변경 ID 하나만 stage(동반 변경 없음, staged = 1) → 재검증 통과 commit(작업 트리 불변, HEAD에는 width만) → GitHub 아님·http·이름 규칙 거부, 이력 있는 저장소 `RemoteHasHistory`, 빈 저장소 `origin` 연결(config에는 GitHub URL), 같은 이름 재연결 거부 → push `New refs/heads/main`, upstream ahead·behind 0, bare 저장소 ref는 main 하나 = local HEAD, tree 동일 |

## 성능

- 해당 없음(성능 예산을 바꾸지 않음).

## 차이

- 실제 GitHub 대신 테스트 HOME 전역 설정 `url.<bare>.insteadOf`로 GitHub URL의 전송만 local bare 저장소로 바꿨습니다(ADR-0111 결정 4). 상태의 remote URL은 Git이 보고한 값이라 fixture에서는 bare 경로입니다.

## 남은 위험

- 신뢰 gate는 테스트가 Trusted로 둡니다. OS 신뢰 등록 UI 경로는 이 시나리오 밖입니다.
