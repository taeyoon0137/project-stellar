# ADR-0120: 시작 복구 probe·폐기 표식, Git 종단 시나리오, 대화형 터미널 상속 환경

## Status

Accepted (2026-11-23). engine lane `recovery-git-terminal`의 결정입니다. 계획 XIII.8.3, XIII.8.4, XIII.8.7, XIV.14.1,
XIV.14.3, XIV.14.4, XVIII.2.5, XVIII.4.6의 엔진 부분을 정합니다. Swift UI와 셸 FFI는 사용자 방향(2026-11-23)에 따라
다루지 않습니다. ADR-0083(창 session·복구 체크포인트), ADR-0023(대화형 터미널 환경), ADR-0080(Git 원본 단위),
ADR-0111(종단 시나리오)을 따릅니다.

## Context

- 복구 체크포인트(XIII.8.1·XIII.8.2)와 복원·폐기 경로(`stellar_runtime::app::recovery`)는 이미 있었지만 세 가지가
  계획 행과 달랐습니다.
  1. 제안 후보를 창 세션 UUID 문자열의 역순으로 골랐습니다. UUID 순서는 창을 연 순서라 "최신 snapshot"이 아닙니다.
  2. 셸이 받는 결과가 창 ID와 hash뿐이었고, probe와 Restore·Discard가 runtime이 복구본을 쓰는 저장소
     (`AppRuntime::enable_recovery`)가 아니라 OS 기본 위치를 따로 계산했습니다.
  3. 정상 저장·닫기·Discard의 삭제가 실패하면 기준 세대가 current와 같은 복구본이 다음 시작에 다시 제안됐습니다.
     XIII.8.7은 "삭제 실패는 다음 시작에서 stale 판정으로 무시"를 요구합니다. 판정도 `stellar_project`의
     `probe_recovery_snapshots`와 runtime이 각자 조합했습니다.
- 대화형 터미널 spawn은 XVIII.1.1의 `env_clear` + 상속 허용 목록 9개였습니다. ADR-0023은 대화형 세션이 앱 프로세스
  환경을 그대로 물려받고 식별 변수 4개와 `SHELL`, profile 명시 지정만 얹도록 정했고 XVIII.2.5가 그 구현입니다.
  `stellar-terminal`의 소비자는 앱 내부뿐이고(지금은 없음), 에이전트 CLI 허용 목록 환경은 `stellar-agent-host`의
  `ChildEnv`가 따로 맡습니다.
- XVIII.4.6은 창 종료 시 터미널을 복원하지 않고, recovery snapshot에 출력·프로세스를 넣지 않는 검사를 요구합니다.
- Git 종단 시나리오(XIV.14.x)는 XIV.14.1 harness(로컬 bare 원격, 네트워크 없음, 고정 시각)를 씁니다. XIV.14.4의 branch
  conflict 해결과 XIV.14.3의 token 의존 staging은 시나리오가 없었습니다. 문서 레코드(`DocumentRecords`)에는 token
  참조가 없고, 텍스트 원본 token set 파일(`design/tokens/*.stellar.yaml`)은 독립 원본 단위입니다(ADR-0080 개정).

## Decision

### 1. 시작 복구 probe는 runtime 저장소에서 최신 복구본 하나를 요약해 돌려준다

- `AppRuntime::recovery_probe(session) -> Result<Option<RecoveryProbe>, String>`을 둡니다. `RecoveryProbe`는 선택을 적용할
  후보(`RecoveryCandidate`: 창 ID + 파일 bytes hash), 미저장 revision, 열린 View 탭 수, 파일 기록 시각입니다. 셸은 앱을
  시작해 창을 연 뒤 이 결과를 보이고, Restore·Discard를 `AppRuntime::recover_checkpoint`로 넘깁니다. Later는 아무것도
  부르지 않으며 복구본은 다음 시작까지 남습니다.
- probe와 `recover_checkpoint`는 runtime의 복구본 저장소 root(`enable_recovery`로 켠 위치, 시험이 바꾼 위치)를 씁니다.
  root가 없으면 OS 사용자 데이터 기본 위치이며, 셸 FFI가 쓰는 `ProjectSummary::recovery_candidate`도 같은 위치입니다.
  `recovery_candidate`는 probe 결과의 후보만 돌려주는 얇은 함수입니다.
- "최신"은 복구본 파일의 마지막 기록 시각입니다(`RecoverySnapshotStore::list_sessions_newest_first`). 시각이 같거나 읽을 수
  없으면 세션 이름 내림차순이고, 시각을 읽지 못한 파일은 가장 오래된 것으로 봅니다. 다른 살아 있는 앱·창이 잠근
  복구본은 건너뛰고 다음 후보를 봅니다(기존 규칙).
- 저장된 복구본 하나의 판정은 `stellar_project::recovery_probe::judge_stored_snapshot` 하나입니다(폐기 표식 → payload
  해석 → `assess_snapshot`). `probe_recovery_snapshots`와 runtime이 같은 함수를 씁니다. runtime은 여기에 "저장한 적 없는
  기준(`base_manifest_hash` 없음)은 이 창 세대에 복원할 수 없으므로 제안하지 않음"만 더합니다.
- 복원은 XIII.8.4대로 새 in-memory 상태이고 디스크 current를 바로 쓰지 않습니다. 두 선택 모두 프로젝트 폴더 bytes를
  바꾸지 않습니다.

### 2. 정상 저장·닫기·폐기의 삭제는 폐기 표식을 먼저 남긴다

- `stellar_project::recovery_shutdown::discard_snapshot`: 지울 복구본 bytes의 content hash를 같은 폴더의
  `<창 UUID>.discarded`에 쓰고, 복구본을 지운 뒤, 표식을 지웁니다. 복구본이 없으면 남은 표식만 지웁니다. 표식 기록이
  실패해도 삭제는 시도하고, 삭제가 실패하면 오류를 돌려주되 표식은 남습니다.
- 판정: 표식의 hash와 bytes가 같은 복구본은 `StaleReason::Discarded`로 제안하지 않습니다. 같은 창이 뒤에 새 bytes를
  기록하면 hash가 달라 다시 제안 대상이므로, 표식과 새 복구본의 쓰기 순서에 기대지 않습니다.
- 체크포인트 worker의 Clear(정상 저장·닫기·저수준 close 폐기), 복원 뒤 이전 창 복구본 정리, 사용자 Discard가 모두 이
  함수를 씁니다. V.9.4 `end_window_session_clean`도 이 함수로 지운 뒤 정상 종료 marker를 씁니다.
- 표식 파일은 스냅샷 목록(`list_sessions`)에 나타나지 않습니다(확장자가 다름). 파일 이름은 창 UUID뿐이라 V.9.1 경로
  정책(프로젝트 이름·경로 텍스트 비노출)을 지킵니다.

### 3. 대화형 터미널 child는 앱 환경을 상속하고 변경 목록만 받는다

- `stellar_terminal::env::EnvironmentPolicy`는 profile의 명시 지정만 담습니다. 상속 허용 목록(`DEFAULT_INHERITED_ENV`,
  `inherit`, `with_defaults`, `resolve`)은 지웁니다. 허용 목록 환경은 에이전트 CLI 실행기(`stellar-agent-host`)에만
  있습니다(ADR-0023 결정 9).
- spawn 입력 `TerminalIdentity`(`TERM`·`COLORTERM`·`TERM_PROGRAM`·`TERM_PROGRAM_VERSION`)를 `LaunchEnvironment`가 받습니다.
  `TerminalIdentity::stellar(version)`은 XVIII.4.1 상수(`TERM_NAME`, `COLORTERM`)와 제품 식별자 `Stellar`를 씁니다.
  `COLORTERM`이 `None`인 identity는 상속값을 지웁니다.
- `interactive_env(identity, shell, profile)`이 이름 순 변경 목록(`EnvChange::Set`·`Remove`)을 만듭니다. 적용 순서는 식별
  변수 → 기본 셸 profile이면 실행하는 셸 경로의 `SHELL` → profile 명시 지정이고 뒤의 것이 이깁니다. argv profile은
  `SHELL`을 바꾸지 않습니다. 그 밖의 변수는 더하지 않습니다(`STELLAR_SET_VARIABLES`).
- `LaunchRequest`는 최종 환경 대신 변경 목록(`env_changes`)을 담고, PTY 실행기는 `env_clear` 없이 상속 환경에 변경만
  적용합니다. 그래서 child가 보는 상속 부분은 OS가 앱 프로세스 환경에서 그대로 복사한 값이고, Stellar가 환경을 미리
  읽거나 가공하지 않습니다.

### 4. 터미널은 복원하지 않으며 recovery payload는 터미널 내용을 담을 수 없다

- 창 종료(registry drop)는 세션 프로세스를 끝내고(XVIII.2.4), 같은 창의 새 registry는 빈 상태입니다. child를 만드는
  입력은 신뢰 gate를 지난 `TerminalRegistry::start`뿐이고 pid로 다시 붙는 API가 없습니다.
- crash 검사는 앱 역할 helper 프로세스가 PTY 세션을 연 채 `SIGKILL`로 끝나는 fixture입니다. PTY master가 닫혀 세션
  프로세스가 끝나고, 다시 시작한 registry는 비어 있습니다.
- recovery payload는 필드 목록을 구조 분해 시험으로 고정하고, payload를 만들고 읽는 crate(`stellar-project`,
  `stellar-runtime`)가 `stellar-terminal`에 의존하지 않음을 시험합니다. 복구 문서 chunk는 runtime이 문서 직렬화 결과와
  같은지 다시 확인하므로 다른 내용이 끼어들 수 없습니다.

### 5. Git 종단 시나리오는 harness와 시스템 Git, ops conflict 세션을 그대로 쓴다

- XIV.14.4: harness의 work·peer clone이 두 branch를 저장 파이프라인으로 편집·push하고, work가 `origin/theirs`를 시스템
  Git `git merge`로 병합합니다(merge driver 미등록, `design/current` 텍스트 conflict). 해결은 `stellar merge-driver
  resolve`와 같은 `ConflictSession` + `GatedMergeGit`, 적용은 `apply_session`(쓰기 + `git add`)입니다. 판정은 unmerged
  entries 0(`git ls-files -u`), MERGE_HEAD = theirs, HEAD = merge 전 ours, `git status`의 "still merging", 작업 트리 =
  index, 자동 병합한 속성 보존, validator 오류 0입니다. 시나리오는 generation 저장 프로젝트를 씁니다. 텍스트 원본 View
  파일 conflict는 conflict 세션이 의미 병합하지 않는 경로라(ADR-0081 단점, XVI.11.3 보고서) 이 시나리오에 넣지 않았고,
  ops conflict 세션에 텍스트 원본 codec이 붙을 때 같은 시나리오를 더합니다.
- XIV.14.1: harness를 crate 밖 소비자처럼 `test-support` feature로 쓰는 시험(`crates/git-porcelain/tests`)을 더해, 같은
  scenario가 플랫폼과 무관하게 같은 ID를 만들 조건(일반 파일 mode `100644`만, LF만, ASCII 경로, 위치 무관)을 고정합니다.
  Windows에서 같은 golden을 확인하는 일은 ADR-0006에 따라 Windows 재개 때 합니다.
- XIV.14.3의 token 의존 staging은 만들 수 없습니다. 문서 레코드에 token 참조가 없어 View가 token set에 기대는 저장
  프로젝트가 생기지 않고, ops staging의 동반 단위 계산(`companion_closure`)도 컴포넌트 정의만 봅니다. token 참조가
  문서 형식에 들어오면 동반 단위 계산과 시나리오를 함께 더합니다.

## Alternatives

- **복구본 payload에 기록 시각을 넣기**: payload 형식(V.9.2) 버전을 올려야 하고, 파일 수정 시각으로 충분합니다. 채택하지
  않습니다.
- **창 세션 ID를 UUIDv7 시각으로 정렬**: 창을 연 시각이지 마지막 기록 시각이 아닙니다. 채택하지 않습니다.
- **폐기 표식에 hash 없이 "폐기함"만 기록**: 같은 창이 뒤에 새 복구본을 쓰면 표식과 쓰기 순서에 따라 새 복구본까지
  무시하거나 폐기한 내용을 제안할 수 있습니다. hash로 bytes를 짚습니다.
- **정상 종료 marker(V.9.4)로 stale 판정**: marker는 세션 시작 때 지워지고 복구본과 짝이 없어, 삭제 실패와 새 기록을
  구분하지 못합니다. 채택하지 않습니다.
- **`LaunchRequest`에 앱 환경 전체를 담고 `env_clear` 유지**: 결과는 같지만 Stellar가 환경을 읽어 다시 만드는 경로가
  남고, XVIII.2.5 검증 열(대화형 spawn 경로의 `env_clear` 사용 0)과 다릅니다. 채택하지 않습니다.
- **`EnvironmentPolicy`의 허용 목록을 남겨 두기**: 대화형 세션에서 쓰이지 않는 규칙이 남아 이중 경로가 됩니다. 에이전트
  CLI 허용 목록은 agent host에 이미 있습니다. 지웁니다.

## Consequences

- 셸은 시작 복구 prompt에 revision·탭 수·기록 시각을 보일 수 있고, Restore·Discard·Later 모두 같은 저장소를 봅니다.
- 삭제가 실패한 복구본은 디스크에 남지만 제안하지 않습니다. 다음 폐기가 성공하면 표식도 정리됩니다. 정리되지 않은
  복구본 파일의 GC는 이 ADR 범위 밖입니다.
- 대화형 터미널은 사용자 계정 환경(`SSH_AUTH_SOCK` 등)을 그대로 받습니다. 앱을 개발 도구에서 실행하면 그 도구의 변수도
  전달됩니다(ADR-0023 Consequences와 같음).
- `stellar-terminal`의 공개 API가 바뀝니다: `LaunchEnvironment.env_lookup` → `identity`, `LaunchRequest::env` →
  `env_changes`, `EnvironmentPolicy`의 상속 관련 함수 삭제, `registry::process_env` 삭제. 이 crate를 쓰는 제품 코드는
  아직 없습니다.
- XIV.14.4는 generation 저장 프로젝트로만 판정합니다. 텍스트 원본 프로젝트의 branch conflict는 ops 후속입니다.

## Compatibility

- 파일 포맷: recovery payload 형식(V.9.2)은 그대로입니다. 복구본 폴더에 `<창 UUID>.discarded` 표식 파일이 생길 수
  있습니다(앱 데이터, 프로젝트·Git 밖). 이전 버전은 이 파일을 모르고 무시합니다.
- 공개 FFI: 바뀌지 않습니다. `stellar_window_recovery_probe`는 같은 후보를 돌려주며, 이제 최신 기록 순서와 폐기 표식을
  따릅니다.
- Rust API: 위 Consequences의 `stellar-terminal` 변경, `stellar_project::recovery_probe::RecoveryProposal`에 `hash` 필드
  추가, `StaleReason::Discarded` 추가, `stellar_runtime::app::recovery::RecoveryProbe`·`AppRuntime::recovery_probe` 추가.
- `stellar-git-porcelain`은 자기 자신을 `test-support` feature로 dev-dependency에 둡니다. resolver 2라 일반 빌드에는
  feature가 켜지지 않습니다.

## Performance

- probe는 복구본마다 metadata를 한 번 더 읽어 정렬합니다(창 수만큼, 보통 한 자리). 폐기 표식 확인은 복구본 bytes의
  hash 한 번입니다. 둘 다 셸 worker·복구 worker에서 실행하며 UI·document thread 예산에 들어가지 않습니다.
- 대화형 터미널 spawn은 환경을 지우고 다시 만드는 단계가 없어 비용이 늘지 않습니다.

## Security

- 복구본·표식은 앱 데이터 폴더(권한 0700)에만 있고 파일 이름에 프로젝트 이름·경로가 없습니다.
- 대화형 셸 환경은 사용자가 일반 터미널 앱을 여는 것과 같은 신뢰 수준이고 신뢰된 프로젝트 게이트를 유지합니다
  (ADR-0023). Stellar는 내부 상태·연결 정보·토큰을 환경에 넣지 않으며, 시험은 Stellar가 더한 변수가 식별 변수 4개와
  `SHELL`뿐임을 실제 PTY에서 확인합니다.
- 터미널 출력·프로세스·환경 값은 recovery snapshot에 들어가지 않습니다(결정 4).

## Migration

- 저장 데이터 마이그레이션은 없습니다. 기존 복구본은 그대로 읽히고, 표식이 없으면 이전과 같은 판정입니다.
- 코드: `stellar-terminal`을 쓰는 코드는 `LaunchEnvironment`에 `TerminalIdentity`를 넘기고, 최종 환경이 필요하면
  `apply_env_changes`로 계산합니다.
