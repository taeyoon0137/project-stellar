# ADR-0080: 의미 단위 Git 엔진을 앱 없이 쓰고 commit 직전에 다시 검증

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `git-engine` lane이 정했습니다.

- ADR-0005 §8 crate 배치 표의 `stellar-ops` 허용 직접 의존에 `git-porcelain`을 더합니다.
- 셸 FFI(XIV.13.x)는 이 ADR 범위가 아닙니다. UI를 다시 설계할 때 새로 정합니다.

## Context

- 핵심 가치 1은 Git 협업입니다. 앱 없이도 의미 단위 stage·commit을 끝까지 쓸 수 있어야 합니다. AI 에이전트와 사람이 같은 엔진을 `stellar` CLI로 씁니다(ADR-0005).
- 기존 코드에는 다음이 이미 있었습니다.
  - git-porcelain: 실행기, trust gate, status 해석, snapshot provider, candidate tree, index 게시, identity, commit 실행.
  - semantic-diff: dependency graph, 선택 stage·unstage 후보.
  - runtime `staging`: 실제 index에서 후보를 준비하고 적용하는 파이프라인.
- 빠진 것은 다음과 같았습니다.
  - commit 직전 재검증(XIV.8.1), commit 뒤 재조회(XIV.8.4), trusted 전용 commit(XIV.8.5·XVI.7.3).
  - 실제 HEAD·upstream·remote·identity 상태(UX.5.1).
  - 원본 단위 포함과 필수 동반 변경 판정(UX.5.4).
  - CLI 노출.
- `stellar-cli`는 `stellar-ops`·diagnostics에만 의존할 수 있습니다. staging 파이프라인은 runtime에 있어 CLI가 쓸 수 없었습니다.
- 컴포넌트 정의는 `design/components/`의 텍스트 원본입니다(ADR-0059·ADR-0061). View 문서는 아직 generation(`design/current`, `design/generations/`, `design/objects/`)입니다. semantic snapshot은 generation 문서만 담습니다.

## Decision

1. **위치.** 엔진은 `stellar_ops::git`에 둡니다. `stellar-ops`가 `stellar-git-porcelain`에 직접 의존합니다.
   - runtime의 staging 파이프라인(`prepare_*`, `apply_stage_candidate`)을 이 모듈로 옮깁니다. `stellar_runtime::staging`은 같은 이름으로 재노출합니다.
   - 앱·CLI·테스트가 같은 구현을 씁니다.
2. **CLI 범위.** `stellar git status|stage|unstage|commit`만 둡니다.
   - `status <경로> [--cursor <c>] [--limit <n>]`: JSON, page 기본 1000·최대 10000.
   - `stage|unstage <경로> <대상>... [--with <대상>]...`
   - `commit <경로> -m <메시지>`
   - push는 의미 단위 가치가 없으므로 시스템 Git에 맡기고 CLI·엔진에 넣지 않습니다. 강제 push 경로도 없습니다.
3. **신뢰.** 모든 Git 실행은 `GatedGitRunner`를 거칩니다.
   - 신뢰하지 않은 프로젝트는 status·stage·commit 모두 프로세스를 만들지 않고 거부합니다(CLI 종료 코드 77, `not-trusted`).
   - CLI는 OS 사용자 데이터의 신뢰 등록(`TrustedProject::select`)으로 신뢰를 판정합니다. 신뢰를 부여하지 않습니다.
   - hook·filter는 시스템 Git이 실행합니다. Stellar는 hook 파일을 직접 실행하지 않습니다.
4. **대상 ID.** 텍스트 형식은 다음 셋입니다.
   - `change:<hex>`: `ChangeId` canonical bytes의 16진수. 값은 담지 않습니다. 오래된 ID는 거부합니다.
   - `generation`: View generation 묶음. `design/components/` 밖의 `design/` 전체입니다(UX.5.4). View 원본이 generation인 동안 모든 View는 이 묶음 하나입니다.
   - `file:<경로>`(UTF-8이 아니면 `file-hex:<hex>`): 그 밖의 Stellar 파일 하나. 컴포넌트 YAML, `stellar.yaml`, `stellar.lock.yaml`, asset이 해당합니다. non-Stellar 파일은 대상이 아닙니다.
   - generation 묶음과 변경 ID는 한 요청에 함께 쓸 수 없습니다.
   - 원본 단위 stage는 그 단위의 index 경로를 저장된 작업 트리 파일과 같게 만듭니다. unstage는 HEAD와 같게 만듭니다.
5. **필수 동반 변경은 자동으로 포함하지 않습니다.**
   - semantic 의존(새 parent, 참조 대상 추가, interface member 등)은 XIV.5.3의 `required_additions`를 씁니다.
   - 컴포넌트 정의 의존은 다음과 같이 판정합니다.
     - 후보 index의 인스턴스가 가리키는 정의가 후보 index 컴포넌트 원본에 없으면 끊긴 참조입니다.
     - 기준 index에 이미 끊겨 있던 참조는 제외합니다.
     - 원천(stage: 작업 트리, unstage: HEAD)에 그 정의 파일이 있으면 `file:` 단위가 필요합니다.
     - 원천에 정의 파일이 없고 변경 ID 없이 원본 단위만 고른 경우에는 `generation` 단위가 필요합니다. stage(작업 트리에서 정의와 인스턴스를 함께 지움)와 unstage가 같습니다. generation을 더해도 원천 인스턴스가 끊기면 해결할 수 없는 오류입니다.
     - 그 밖에는 해결할 수 없는 오류입니다.
     - 필요한 단위를 더한 뒤 다시 판정해 고정점까지 계산합니다.
   - 결과는 `CompanionRequired { changes, units, reasons }`입니다. index는 바꾸지 않습니다. 호출자가 고른 항목을 `approved`(`--with`)로 다시 보냅니다.
   - 필수가 아닌 승인은 오래된 승인으로 거부합니다.
6. **같은 의미면 원천 bytes를 씁니다.** 선택 결과의 semantic snapshot이 원천과 같으면 새 generation을 쓰지 않고 generation 묶음을 원천 bytes로 둡니다. 원천은 stage면 저장된 작업 트리, unstage면 HEAD입니다.
   - 같음은 저장 왕복이 무손실로 증명된 snapshot끼리의 비교입니다(`saved_semantic_snapshot`).
   - 이때 원천 bytes로 바꾸는 경로는 generation 저장 경로(`design/current`, `design/generations/`, `design/objects/`)뿐입니다. 고르지 않은 `design/` 아래 다른 파일은 포함하지 않습니다. 명시적인 `generation` 선택만 `design/` 전체를 포함합니다.
   - 그래서 모두 unstage하면 `git diff --cached`가 비고, 모두 stage하면 generation 파일이 작업 트리와 같아집니다.
7. **commit 순서.**
   1. trust를 확인합니다.
   2. staged snapshot을 전체 validator로 다시 검사합니다(XIV.8.1).
      - 검사 항목: conflict 없음, `stellar.yaml`, 저장소 loader 무결성, 문서 validator, 저장 무손실 왕복, semantic 무결성, 컴포넌트 원본 읽기.
      - 오류가 하나라도 있으면 `git commit`을 실행하지 않습니다.
      - 정의가 없는 인스턴스는 `stellar validate`와 같이 경고입니다. 인스턴스의 정의 참조는 문서 snapshot 밖 원본이므로 semantic `DanglingReference`에서 제외하고 이 경고로 봅니다.
   3. 검사한 index와 지금 index가 다르면 실행하지 않습니다.
   4. 시스템 Git `git commit --file=- --cleanup=verbatim`을 실행합니다. identity 누락은 구조화한 오류입니다. hook 실패는 우회하지 않고 Git 결과(종료 코드·stderr)를 그대로 돌려줍니다.
   5. 성공하면 HEAD·index·status를 다시 읽습니다. 결과에는 남은 staged 경로 수와 계산 revision을 담습니다. 새 HEAD tree가 검사한 index와 같은지(`tree_matches_validated`)도 담습니다. hook이 index를 바꿔 다르면 알리기만 하고 되돌리지 않습니다.
   6. 자동 push하지 않습니다.
8. **상태 조회는 worker에서 합니다(UX.5.1).**
   - runtime `submit_git_status`가 worker pool에서 `GitSession::status`를 실행합니다. 결과에는 요청 순번과 계산 revision이 붙습니다. 받는 쪽은 더 새 순번만 반영합니다.
   - 아직 Git 전용 worker class가 없으므로 `CacheGeneration`(Background)을 씁니다. class 추가는 UI 재설계 때 정합니다.
   - 계산 revision은 다음의 hash입니다. 같은 상태면 같은 값입니다.
     - HEAD commit
     - index 항목(경로·mode·OID, unmerged 포함)
     - 저장된 Stellar 작업 파일의 경로와 bytes. 내용 hash 이름인 `design/objects/`·`design/generations/`는 경로만 넣습니다.
     - raw status
   - page cursor는 revision 끝 16자와 offset입니다. 상태가 바뀌면 `StaleCursor`입니다.
   - status는 실제 HEAD(branch·detached·unborn), upstream(ahead·behind·gone), 변경 수, 진행 중 작업, identity 누락 항목, remote(credential을 지운 fetch URL)를 담습니다.
9. **linked worktree는 별도 project identity입니다.**
   - 세션은 worktree root에 고정합니다. Git dir·index는 그 worktree의 것을 씁니다. common Git dir은 공유합니다.
   - 신뢰 등록은 실제 경로 단위이므로 linked worktree도 따로 신뢰해야 합니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| CLI가 runtime에 의존 | ADR-0005 §8이 CLI 의존을 ops·diagnostics로 제한합니다. 앱 runtime(thread·renderer)을 headless 도구로 끌어옵니다. 미선택 |
| 파이프라인을 runtime에 두고 ops에 복사 | 같은 의미의 두 구현이 갈라집니다. 미선택 |
| `git write-tree`+`commit-tree`로 검증한 tree를 그대로 commit | pre-commit 등 hook을 우회합니다(XIV.8.5 위반). 미선택 |
| 필수 동반 변경 자동 포함 | 사용자가 고르지 않은 변경이 commit에 들어갑니다(XIV.5.3·UX 명세). 미선택 |
| CLI에 push 포함 | 의미 단위 가치가 없고 시스템 Git과 중복됩니다. 강제 push 위험이 생깁니다. 미선택 |
| 정의 없는 인스턴스를 commit 오류로 처리 | `stellar validate`와 판정이 달라지고, 기존 상태도 commit하지 못하게 됩니다. 새로 끊기는 참조는 stage 단계에서 막습니다. 미선택 |

## Consequences

- 장점
  - 앱 없이 `stellar git`으로 status→stage→commit을 끝까지 쓸 수 있습니다. AI 에이전트도 같은 경로를 씁니다.
  - invalid staged snapshot, untrusted 프로젝트, hook 실패에서는 commit이 생기지 않습니다.
  - 원본 단위와 semantic 단위를 같은 승인 규칙으로 다룹니다.
- 단점·한계
  - generation 원본에서는 속성 하나만 stage해도 chunk·manifest·`design/current` 세 파일이 바뀌고 diff는 바이너리입니다. "속성 줄만 보이는 diff"는 View 텍스트 원본 전환(ADR-0059) 뒤에 가능합니다.
  - token은 아직 저장 포맷이 없어(문서 레코드에 token 없음) 저장 프로젝트로 token 의존 staging을 재현할 수 없습니다. token·binding 의존 판정은 semantic-diff dependency graph(XIV.5.2)가 맡고 그 단위 테스트로만 검증됩니다.
  - commit은 시스템 Git이 index를 쓰므로 재검증과 `git commit` 사이의 짧은 경합 창이 남습니다. 직전 비교와 `tree_matches_validated`로 드러내지만 막지는 못합니다.
  - 상태 조회는 HEAD·index를 임시 디렉터리에 materialize하므로 큰 프로젝트에서 비용이 큽니다. 성능 예산 측정은 후속입니다.

## Compatibility

- 파일 포맷 변경은 없습니다.
- `stellar_runtime::staging`의 공개 함수·타입 이름은 그대로입니다(재노출). `PreparedStageCandidate`에는 비공개 필드가 늘었습니다. 테스트용 fault injection은 `test-support` feature의 `with_semantic_for_test`로 바꿨습니다.
- CLI에 `git` 명령이 추가됩니다. 결과 schema 식별자(`stellar-cli/result` v1)는 그대로입니다.
- FFI 변경은 없습니다.

## Performance

- status는 Git 명령 여러 개와 HEAD·index materialize, 프로젝트 load 세 번을 실행합니다. 앱은 worker에서만 부릅니다.
- `config/performance-budgets.yaml`은 바꾸지 않습니다. 큰 프로젝트 측정은 후속 작업입니다.

## Security

- 신뢰하지 않은 프로젝트에서 Git 프로세스 생성은 0입니다.
- remote URL은 credential을 지운 값만 출력합니다.
- 컴포넌트 원본은 `ExternalAccess::MetadataOnly`로 읽어 프로젝트 밖 symlink target 내용을 읽지 않습니다.
- 임시 materialize는 작업 트리 밖 `0700` 비공개 디렉터리에서 하고 drop 때 지웁니다.

## Migration

- 없음. 기존 runtime staging 호출자는 코드 변경 없이 같은 이름을 씁니다.

## 개정(2026-11-23, integration-followups)

1. **staged 재검증과 `stellar validate`의 문서 판정은 한 함수.** `stellar_ops::validate::validate_records`(문서 레코드와
   컴포넌트 정의 입력)를 공개하고 `validate_snapshot`과 `GitSession::validate_staged`가 모두 부릅니다. 그래서 문서
   validator 위반(`document-invalid` 오류), 미해결 참조(`unresolved-reference` 경고), 정의 없는 인스턴스
   (`missing-component-definition` 경고), 정의 원본 읽기 실패(`component-library-load` 오류) 규칙과 진단 문구가 같습니다.
   이 개정 전 staged 재검증은 미해결 참조 경고를 내지 않았고, 정의 없는 인스턴스를 문서와 무관한 문구로 냈습니다. staged
   재검증만의 검사(저장소 loader, 저장 왕복, semantic 무결성, 프로젝트 단위 컴포넌트 원본 `component-library`)는 그대로입니다.
   정의는 인스턴스가 있는 문서에서만 한 번 읽습니다.
2. **텍스트 원본 파일은 의미 단위 파일.** `SourceUnit::of_path`가 텍스트 원본 배치(ADR-0077 결정 1)를 generation 묶음에서
   뺍니다. `design/document.stellar.yaml`, `design/assets/catalog.stellar.yaml`, `design/tokens/<slug>.stellar.yaml`,
   View 파일 `design/pages/<page>/<view>.stellar.yaml`은 각각 파일 단위이고, View의 분할 Frame 파일
   (`design/pages/<page>/<view>/**.stellar.yaml`)은 서로 `$ref`로 기대므로 그 View 파일 단위에 묶습니다(단위 ID는 View 파일
   경로). generation 저장 경로와 배치에 없는 나머지 `design/` 경로는 지금처럼 `Generation`입니다. codec이 실제로 쓰는 모든
   경로가 파일 단위로 분류되는지 test가 확인합니다. 한계: 페이지 목록(`document.stellar.yaml`)과 View 파일 사이 동반
   판정(새 View 파일만 stage하는 경우)은 아직 없고, staged 재검증은 아직 generation만 읽어 텍스트 원본 정합을 보지 않습니다.
   둘 다 텍스트 원본이 기본 저장이 되는 V.10.7 전환 때 검사에 넣습니다.

## 개정(2026-11-23, cli-mcp-git-cleanup)

1. **문서 하나의 판정은 읽기 결과까지 한 함수.** `stellar_ops::validate::validate_loaded_document`(문서 읽기 결과와 컴포넌트
   정의 입력)를 공개하고 `stellar validate`와 `GitSession::validate_staged`가 모두 부릅니다. 읽기 실패의 `document-load`
   오류도 이 함수가 만듭니다. 두 경로는 문서를 어디서 읽는지(작업 트리의 current generation, staged index를 materialize한
   tree)만 다릅니다.
2. **참조 대상의 존재는 문서 판정만 봅니다.** 이 개정 전 staged 재검증은 semantic 무결성
   (`stellar_semantic_diff::validate::validate_snapshot`)의 `DanglingReference`를 인스턴스의 정의 참조만 빼고 모두
   `snapshot-invalid` 오류로 냈습니다. 그래서 `stellar validate`가 유효(미해결 참조 경고 III.2.4)로 판정한 tree의 commit을
   막았습니다. 같은 사실을 두 곳에서 다르게 판정한 것입니다. 이제 `DanglingReference`는 모두 semantic 검사에서 빼고, 문서
   판정의 `unresolved-reference`·`missing-component-definition` 경고만 남깁니다. semantic 검사는 구조(parent 존재·순환,
   형제 OrderKey, 소속 목록, 부모 없는 node, interface 멤버)만 봅니다. 같은 tree에서 두 경로의 문서 진단이 같은지 test
   (`git::tests::staged_judgment_matches_project_validation`)가 확인합니다. staged semantic snapshot은 문서만 담고 asset과
   컴포넌트 정의를 담지 않으므로, 이 개정 전에는 이미지 node가 있는 문서는 asset이 실제로 있어도 `IMAGE_ASSET`
   `DanglingReference`로 commit이 막혔습니다. 같은 test가 이미지 node도 넣어 이 경우를 확인합니다. 한계: 이미지 asset이
   실제로 있는지는 `stellar validate`와 staged 재검증 어느 쪽도 판정하지 않습니다. 넣을 때는 두 경로가 함께 쓰는
   `validate_loaded_document` 쪽에 넣습니다.
3. **staged 재검증만의 검사는 그대로.** 저장소 loader 무결성(`storage-invalid`), 저장 무손실 왕복(`storage-roundtrip`),
   프로젝트 단위 컴포넌트 원본 읽기(`component-library`)는 commit 전에만 보는 추가 검사입니다. 기존 component-library chunk와
   텍스트 원본이 함께 있는 경우는 `stellar validate`가 `component-library-load`로 보고, staged 재검증에서는 그 chunk를 문서
   레코드에서 다시 만들 수 없어 저장 왕복 검사가 `storage-roundtrip` 오류로 막습니다.
