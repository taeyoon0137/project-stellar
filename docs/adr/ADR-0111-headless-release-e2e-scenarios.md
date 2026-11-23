# ADR-0111: 초기 릴리스 종단 시나리오를 앱 UI 없이 앱 runtime·headless 경로로 실행한다

## Status

Accepted (2026-11-23). engine lane `headless-e2e`의 결정입니다. 계획 XVI.11.1\~XVI.11.6의 E2E를 셸 UI 없이 만드는 방법과,
지금 엔진에 없어 시나리오가 닿지 못한 부분을 기록합니다.

## Context

- 계획 XVI.11.1\~XVI.11.6은 초기 릴리스 종단 시나리오(새 프로젝트·저장·다시 열기, 부분 stage·commit·push, branch 충돌
  해결, 외부 library 두 버전·update, 자동 저장 중 비정상 종료·복구, 두 창 stale 저장)를 요구합니다.
- 사용자 방향(2026-11-23): 지금은 엔진·기능만 만들고 Swift UI·셸 FFI를 더하지 않습니다. macOS 셸은 ADR-0098로 빈 창
  껍데기만 남았습니다. 기능은 Rust 테스트·CLI·MCP로 검증합니다.
- 셸이 쓰던 진입점은 그대로 `stellar_runtime::app::AppRuntime`에 있습니다: 프로젝트 생성·열기, 창 열기·닫기, Frame·Text·Image
  배치, 속성 패널 편집, Save Now, 자동 저장, stale 알림·두 선택지, 복구 체크포인트·복구. headless 편집·Git은 `stellar_ops`
  (CLI·MCP가 in-process로 부르는 같은 구현)에 있습니다.
- 이 lane이 고칠 수 있는 파일은 `tests/integration/tests/e2e_*`뿐입니다(제품 코드 변경 없음).

## Decision

### 1. 시나리오는 셸과 같은 앱 runtime 진입점과 headless ops만 쓴다

- 테스트 helper `e2e_common::App`은 `AppRuntime`에 요청을 보내고 그 요청의 완료 event를 기다린 뒤 상태 snapshot을 읽습니다.
  셸이 C ABI로 하던 순서(요청 → 완료 event → snapshot)와 같습니다. 요청 없는 표시 변경 event(request 0)는 건너뜁니다.
- 문서 편집은 runtime 요청(`insert_canvas_frame`, `begin_canvas_text`/`commit_canvas_text`, `place_png`,
  `edit_selection_property`)으로만 합니다. CSS 값 텍스트는 속성 문법 parser(`inspector::parse_input`) 하나로 해석합니다.
- headless 단계는 CLI·MCP와 같은 ops 함수입니다: `apply`, `component_source::{create, resize, query}`, `git::GitSession`
  (status·stage·validate_staged·commit), `candidate::session::ConflictSession`·`candidate::apply::apply_session`.
- 테스트 안에서 제품 판정·해석·직렬화를 따로 구현하지 않습니다. 결과 확인은 제품 읽기 경로(창 session 다시 열기, ops snapshot,
  텍스트 원본 `semantic_key`, Git 명령 출력)로 합니다.

### 2. 사용자 데이터·Git 환경은 테스트 프로세스의 임시 HOME으로 격리한다

- 각 테스트 binary는 처음에 `HOME`·`XDG_CONFIG_HOME`·`GIT_CONFIG_GLOBAL`을 임시 폴더로 바꾸고 `GIT_CONFIG_NOSYSTEM=1`을
  둡니다(한 번만, `OnceLock`). 저장 잠금, 신뢰 저장소, 복구 체크포인트, library cache가 사용자 실제 데이터 폴더에 닿지 않고,
  Git은 그 HOME의 전역 설정(고정 identity)만 읽습니다.
- 앱 runtime의 프로젝트 생성 Git 실행기와 ops `system_git_runner`가 이 환경 변수를 그대로 전달하므로 제품 실행기를 바꾸지 않고
  격리됩니다. 신뢰 판정이 필요한 Git 엔진 호출은 신뢰 gate(`GatedGitRunner::new(.., GitTrust::Trusted)`)를 테스트가 직접
  둡니다(OS 신뢰 등록 UI는 범위 밖).

### 3. XVI.11.1의 Component·Token은 제품 경로로 만들고, View Variant는 저장 형식이 없어 다루지 않는다

- Component 정의는 `stellar component-source create`와 같은 `component_source::create`로, Component 사용은 `stellar apply`의
  `InsertComponentInstance`로 만듭니다. Token set은 생성 command가 없으므로 텍스트 원본 `design/tokens/<set>.stellar.yaml`을
  codec 공개 encoder(`source_v4::token::encode`)로 씁니다. 사용자·AI가 Git 원본 파일을 직접 고치는 경로와 같은 bytes입니다.
- "semantic snapshot 동일"은 세 겹으로 확인합니다: 문서 semantic snapshot JSON, 텍스트 원본 전체 의미(`TextProject::semantic_key`:
  문서 레코드·컴포넌트 library bytes·token set·이미지 record), 다시 연 창의 Save Now가 쓸 것이 없음.
- View 변수·Variant(IX.7)는 메모리 command만 있고 문서·텍스트 원본·chunk 어디에도 영속 형식이 없습니다. 그래서
  저장·다시 열기 시나리오에 넣을 수 없고 XVI.11.1은 부분 완료입니다. "양 플랫폼"은 ADR-0006에 따라 macOS만입니다.

### 4. XVI.11.2의 GitHub 연결은 URL 검증 단위와 local bare remote로 나눈다

- URL 형식 판정(`classify_github_url`)과 연결 전 거부(GitHub 아님·형식 오류·이름 규칙)는 Git을 실행하지 않는 단위로 따로 봅니다.
- 연결·push는 제품 함수(`connect_empty_github_remote`, `push_and_refresh`)를 GitHub URL 그대로 끝까지 실행합니다. 테스트 HOME의
  전역 Git 설정 `url.<bare 경로>.insteadOf = https://github.com/<owner>/<repo>.git`이 전송만 빈 local bare 저장소로 바꿉니다.
  저장소 config에는 GitHub URL이 남고, `ls-remote` 빈 저장소 확인·이력 있는 저장소 거부도 실제로 실행됩니다. 네트워크는
  쓰지 않습니다. Git이 보고하는 remote URL(`remote -v`)은 바꾼 뒤 값이라 fixture에서는 bare 경로로 보입니다.

### 5. XVI.11.3은 generation 문서와 컴포넌트 원본 conflict로 실행하고, 텍스트 원본 View 파일 conflict는 남은 일로 둔다

- 두 branch를 시스템 Git으로 만들고 `git merge`(merge driver 미등록)로 텍스트 conflict를 낸 뒤 conflict 세션에서 해결하고
  `apply_session`으로 쓰고 `git add`합니다. 판정: unmerged 0, MERGE_HEAD 유지, commit·continue 없음, 결과 validator·staged 재검증
  통과, 서로 다른 속성 변경 자동 병합.
- 앱 창으로 편집한 프로젝트는 텍스트 원본(ADR-0088)이 Git 원본이라 두 branch가 같은 View 파일
  (`design/pages/<page>/<view>.stellar.yaml`)을 바꿉니다. conflict 세션은 그 경로를 "의미 병합하지 않는 Stellar 저장 경로"로
  제외하므로(ADR-0081 단점, 실측 확인) unmerged가 남습니다. 그래서 시나리오는 CLI로 편집한(텍스트 원본이 없는) 프로젝트를 쓰고
  XVI.11.3은 부분 완료입니다. View·문서 텍스트 원본 codec을 conflict 세션과 merge driver dispatch에 더하는 일이 남았습니다.

### 6. XVI.11.5의 비정상 종료는 실제 프로세스 중단이다

- 테스트 binary가 자기 자신을 자식 프로세스로 다시 실행합니다(`--exact crash_child_process`, 부모 HOME을 환경 변수로 넘김).
  자식은 자동 저장과 복구 체크포인트를 켠 앱 runtime을 돌리다 `std::process::abort()`로 끝나므로 창 닫기·runtime drop 같은
  정상 종료 정리가 한 번도 실행되지 않습니다. 부모는 SIGABRT 종료를 확인합니다.
- 결정적 경로: 자동 저장 debounce를 길게 두고 앱 비활성화 flush로 첫 편집을 자동 저장 writer가 쓰게 한 뒤, 둘째 편집은 체크포인트
  파일이 저장소에 생긴 것을 확인하고 중단합니다(살아 있는 창의 체크포인트는 그 창이 잠가 후보 조회가 건너뛰므로 저장소 파일을
  직접 봅니다). 쓰기 중 중단: 체크포인트 뒤 flush를 접수하고 바로 중단합니다. 쓰기가 끝났으면 디스크가 최신이고 복구 제안이 없어야
  하며, 끝나지 않았으면 복구 후보가 둘째 편집을 돌려줘야 합니다(어느 쪽이든 편집을 잃지 않음).

### 7. XVI.11.4는 제품 의존성 엔진이 닿는 곳까지 실행하고, 빠진 생산자를 테스트에서 만들지 않는다

- 실제 Git 저장소(C v1·v2, A→C v1, B→C v2)를 bare mirror fetch·`ls-remote`·commit `stellar.yaml` 읽기로 해석하고
  (`MirrorCache`·`GitSelectorResolver`·`GitManifestSource`·`resolve_dependency_graph`), 다중 버전 경고(`duplicate_version_reports`),
  update 후보(`prepare_library_update`)와 resource diff의 삭제 property(`removed_properties`)까지 확인합니다.
- 제품에 없는 것: (1) 후보 library와 사용 프로젝트 instance override를 대조해 orphan 진단을 내는 생산자(XV.7.3 미착수),
  (2) 승인 뒤 lock·in-memory snapshot 적용(XV.7.4), (3) 의존성 trait(`GitCommandRunner`·`RepositoryKeyer`·`LibraryExportSource`·
  `LibraryProjectValidator`)의 제품 구현. 테스트는 (3)을 얇은 연결(git-porcelain `GitRunner`, 저장 hash keyer, commit별 export 표)로
  두고, (1)의 진단을 테스트 안에서 따로 판정하지 않습니다(이중 판정 금지). XVI.11.4는 부분 완료입니다.

### 8. XVI.11.6은 같은 앱의 두 창으로 두 선택지를 각각 확인한다

- 같은 프로젝트 session의 두 창이 서로 다른 편집을 하고 첫 창이 저장하면, 둘째 창 Save Now는 `SaveStale`이고 디스크는 그대로이며,
  외부 변화 반영 뒤 stale 알림이 둘째 창에 한 번만 나옵니다(같은 앱 창이 쓴 generation이므로 두 선택지).
- UseCurrentWindow: 디스크 = 둘째 창 snapshot(첫 창 편집 덮임), history 유지, cache generation도 같은 문서.
  UseLatestDisk: 둘째 창 편집·history 폐기, 디스크 bytes 불변. 두 경우 모두 HEAD·index 불변이고 저장은 작업 트리 변경입니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| 셸 UI 자동화(XCUITest)로 E2E | 셸이 ADR-0098로 지워졌고 사용자 방향이 엔진 우선입니다. 미선택 |
| 창 session(`ProjectWindowSession`)만 직접 호출 | document thread·완료 event·자동 저장·복구 worker가 빠져 셸 경로와 다릅니다. 결과 확인(다시 열기)에만 씁니다 |
| 실제 GitHub 저장소 사용 | 네트워크·자격 증명이 필요하고 재현되지 않습니다. 계획 문구도 local bare remote를 요구합니다. 미선택 |
| 비정상 종료를 `std::mem::forget(runtime)`으로 흉내 | 같은 프로세스 thread가 계속 살아 있어 체크포인트·잠금 상태가 실제 중단과 다릅니다. 미선택 |
| 빠진 orphan 진단·export 공급자를 테스트에서 구현 | 제품과 다른 두 번째 판정이 생기고 제품 공백을 가립니다. 미선택 |
| Variant를 메모리 command로만 확인하고 XVI.11.1을 완료로 표시 | 저장·다시 열기가 시나리오의 핵심이라 완료 근거가 되지 않습니다. 미선택 |

## Consequences

- 초기 릴리스 시나리오 여섯 개가 셸 없이 CI에서 실행됩니다(시스템 Git이 없으면 건너뜀을 출력).
- 시나리오가 드러낸 제품 공백(결정 3·5·7)이 행 상태 부분 완료로 남습니다: View Variant 영속 형식, 텍스트 원본 View 파일의 semantic
  conflict 해결, 외부 library override orphan 진단·적용과 의존성 trait의 제품 구현.
- 두 E2E(XVI.11.2·XVI.11.3)는 계획 XIV.14.4(branch conflict 통합 실행)와 XIV.9.x의 증거로도 쓸 수 있습니다. 행 판정은 통합 때 합니다.

## Compatibility

- 제품 코드·파일 포맷·공개 API·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로를 바꾸지 않습니다.
- `tests/integration`(제품 crate가 의존하지 않는 테스트 crate)에 dev 의존 `stellar-runtime`·`stellar-ops`·`stellar-dependencies`를
  더했습니다(crate graph 검사 제외 대상).

## Performance

- 성능 예산을 바꾸지 않습니다. 시나리오 시간(DEV-MAC, debug, 다른 작업과 동시 실행)은 파일당 수 초\~수십 초이며 branch 충돌
  시나리오가 가장 깁니다(Git 명령·generation materialize).

## Security

- 테스트는 사용자 HOME·전역 Git 설정·신뢰 저장소를 읽거나 쓰지 않습니다(결정 2). 원격은 로컬 파일 전송만 씁니다.
- 신뢰 gate를 테스트가 직접 Trusted로 두는 것은 테스트 전용이며 제품 신뢰 판정 경로(`os_git_trust`)를 바꾸지 않습니다.

## Migration

- 없음.
