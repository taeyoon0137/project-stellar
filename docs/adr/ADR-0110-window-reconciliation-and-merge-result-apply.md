# ADR-0110: 열린 창의 외부 변경 reconciliation과 Merge Composer 결과 적용

## Status

Accepted (2026-11-23). engine lane `merge-reconcile`의 결정입니다. 계획 XIII.3.4, XXIII.3.1~XXIII.3.4를 구현합니다.
ADR-0083 결정 2의 "XIII.3.4 semantic reconciliation이 연결되기 전까지 이 상태로 남고 자동 저장도 멈춘 채"를 이 ADR
결정 1·2가 대체합니다. ADR-0083 본문은 고치지 않습니다.

## Context

- 기술 아키텍처 §22.5는 로컬 변경이 있는 창에 외부 변경이 오면 `LoadedBase / CurrentMemory / ExternalDisk`를 비교해 겹치지
  않는 변경은 자동 재적용하고, 의미 충돌은 Semantic Conflict 경로로 보내도록 정합니다. 구현은 외부 writer(CLI·MCP·Git·다른
  프로세스)와 미저장 편집이 겹치면 `StaleResolution::ReconciliationPending`으로 멈추기만 했습니다(ADR-0083 결정 2).
- Semantic Merge 엔진(XIV.10), 독립 candidate C와 해결(XXIII.2, ADR-0081), Merge Composer source 해석·merge-base·snapshot
  materialize(XXIII.1)는 있었지만 C를 어디에 어떻게 쓰는지(XXIII.3)가 없었습니다.
- lane 정의는 runtime → semantic-merge 의존 간선을 ADR로 더하라고 했습니다. runtime은 이미 `stellar-ops`에 의존하고,
  ops는 semantic-merge에 의존합니다(ADR-0005 §8 허용 표).
- 사용자 방향(2026-11-23): 엔진·기능만 만들고 Swift UI·셸 FFI는 더하지 않습니다. 기능은 Rust 시험·CLI·MCP로 봅니다.

## Decision

### 1. 창 reconciliation은 세 문서 snapshot의 3-way merge 하나다 (XIII.3.4)

- 판정 함수는 `stellar_semantic_merge::apply::reconcile` 하나입니다. theirs = base면 `KeepOurs`, ours = base이거나 ours =
  theirs면 `TakeTheirs`, 그 밖은 `CandidateSnapshot::build`로 병합하고 conflict가 있으면 `Conflicted`, conflict 0이면
  `CandidateSnapshot::validate`(XXIII.2.4)를 통과해야 `Merged`, 아니면 `Blocked`입니다. 새 병합 규칙·검사는 만들지 않습니다.
- 문서 하나의 3-way는 `stellar_ops::merge_apply::document::merge_document`입니다. 세 입력 모두 "레코드 → semantic snapshot
  → 레코드" 왕복이 같아야 병합합니다(무손실 조건). 왕복이 다른 입력을 병합하면 snapshot에 없는 필드를 조용히 잃기 때문입니다.
  결과도 다시 snapshot으로 올린 값이 병합 snapshot과 같아야 합니다.
- LoadedBase는 창의 loaded generation(`ProjectWindowSession`이 마지막으로 읽거나 쓴 generation)에서 이 창 문서만 다시 읽습니다
  (`stellar_runtime::reconcile::base_records`). current pointer와 무관하게 그 manifest가 가리키는 content-addressed chunk만
  읽습니다. 메모리에 base 사본을 따로 두지 않습니다. 텍스트 원본에서 연 뒤 cache를 아직 맞춰 쓰지 않았으면(`FromText` +
  맞춤 쓰기 대기) loaded generation은 base가 아니므로 병합하지 않습니다(`BaseUnavailable`). chunk가 지워졌을 때도 같습니다.
- ExternalDisk는 열기·다시 불러오기와 같은 `read_disk_state`로 읽습니다(원본·cache 계획 그대로).
- 쓸 수 있는 결과는 clean 창 반영(ADR-0083 결정 3)과 같은 끝부분(`adopt_external`)으로 `ApplyExternalSnapshot` transaction
  **하나**를 history에 기록하고 base를 ExternalDisk로 바꿉니다. Undo 한 번이 병합 반영 전체를 되돌립니다. 결과가 디스크 문서와
  같으면 clean, 다르면 미저장 편집으로 남습니다. reconciliation 자체는 파일을 쓰지 않습니다.

### 2. 병합할 수 없으면 그 디스크 관찰값에 한해 두 선택지다

- 겹침·결과 위반·base 없음·문서 삭제·무손실 증명 실패면 문서·history를 바꾸지 않고 `ExternalConflict`(이유 code, conflict 목록,
  위반 수)를 그 디스크 관찰값(current manifest hash + 원본 digest)과 함께 기록합니다. 같은 관찰값이면 stale 다음 할 일은
  `StaleResolution::Choose`이고, `resolve_stale`의 UseCurrentWindow·UseLatestDisk를 쓸 수 있습니다. 사용자가 conflict를 보고
  명시적으로 고른 경우이므로 UseCurrentWindow가 외부 writer generation 위에 쓰는 것을 허용합니다(ADR-0083 결정 2의 "외부 writer
  generation을 두 선택지 경로로 덮어쓰지 않음"은 reconciliation을 시도하기 전까지로 좁힙니다).
- 디스크가 다시 바뀌면 관찰값이 달라 다시 `ReconciliationPending`이고 다음 반영이 다시 병합합니다. `ReconciliationPending`은
  이제 "아직 병합을 시도하지 않음"이라는 짧은 상태입니다.
- 앱 runtime: 감시 경로의 반영(`sync_external`)이 바로 병합합니다. 자동 저장 writer가 외부 writer stale을 먼저 만나면 그 창의
  반영을 미뤄 두어 쓰기 결과를 반영하는 자리(collect)에서 병합합니다. 병합 결과를 반영하면 stale 표시를 풀고 자동 저장 일정을
  새로 시작해 결과를 씁니다(자동 저장 재개). 셸 API·FFI는 더하지 않으며, 상세는 `ProjectWindowSession::external_conflict`로
  읽습니다.

### 3. runtime → semantic-merge 직접 간선은 두지 않는다

lane 정의의 제안과 달리 runtime은 `stellar-ops`(이미 허용된 간선)의 `merge_apply::document`를 부릅니다. semantic snapshot 투영
(`diff::snapshot_of`)·역투영·`ConflictEntry` 표시 변환이 ops에 있고, Merge Composer 적용도 같은 함수를 씁니다. 직접 간선을
더하면 runtime이 같은 투영·판정을 따로 조합하는 두 번째 경로가 생기고, xtask crate graph 정책과
`docs/architecture/crate-dependency-graph.md`도 바뀌어야 합니다. crate graph는 바뀌지 않습니다.

### 4. 결과 target은 세 가지이고 검증은 읽기만 한다 (XXIII.3.1)

`stellar_ops::merge_apply::target::ResultTarget`:

| target | 검증 |
|---|---|
| `CurrentWorktree` | Merge Composer 프로젝트가 열리고 같은 project id, 진행 중 merge·rebase·cherry-pick·revert 없음, unmerged 0 |
| `ExistingWorktree { path }` | `git worktree list`에 있음(bare·prunable 아님), 같은 저장소 상대 경로의 프로젝트에 위와 같은 검사 |
| `NewBranchWorktree { branch, path, start }` | branch 이름 형식(`is_valid_ref_body`)·미존재, 경로는 절대·미존재·부모 있음·기존 worktree 밖, 시작 commit 존재, 그 commit tree에 프로젝트 `stellar.yaml`·`design/current`가 있음 |

검증은 `rev-parse`·`worktree list`·`ls-tree`·`ls-files -u`만 실행하고 아무것도 만들지 않습니다. 기존 target은 검증 시점의 프로젝트
revision을 기록합니다.

### 5. 새 branch+worktree는 시스템 Git 명령 하나로 만든다 (XXIII.3.2)

`git worktree add -b <branch> -- <path> <start>`(git-porcelain `plan_add` + `execute_plan`, 확인이 필요 없는 Safe 수준) 하나입니다.
`.git` 아래 파일을 직접 만들거나 고치지 않습니다. 이 명령은 C 쓰기 조건과 source 재검사를 모두 통과한 뒤에만 실행합니다.
만든 뒤 프로젝트 확인이 실패해도 branch·worktree를 지우지 않습니다(지우기는 파괴적 명령이라 사용자가 결과를 보고 정합니다).

### 6. C는 target WorkingTree에 generation save만 한다 (XXIII.3.3)

- 저장 잠금(`LockPolicy`) → target revision 재비교 → C 문서 재직렬화 → 저장 파이프라인(V.5, optimistic base check) 순서입니다.
  `stellar apply` 쓰기 모드와 같은 저장 경로이고, 문서가 아닌 chunk(project metadata·컴포넌트·token·asset)는 target pointer를
  그대로 잇습니다. 결과가 target 문서와 같으면 쓰지 않습니다.
- `git commit`·`merge`·`add`·`reset`·`checkout`을 부르지 않습니다. target HEAD·index는 그대로이고 C는 작업 트리 변경으로만
  남습니다(merge commit 자동 생성 0).
- 지원 범위: 문서 의미(generation)만 병합합니다. Source A·B의 컴포넌트 원본(`design/components`) 집합 hash가 base와 다르면
  병합하지 않고 거부합니다(조용히 버리지 않음). 결과가 target에 metadata가 없는 이미지 asset을 새로 참조하면 쓰지 않습니다.
  공통 base가 하나가 아니면(2-way·criss-cross) 3-way 입력이 없으므로 거부합니다.

### 7. 적용 전에 source revision·OID와 내용 지문을 다시 검사한다 (XXIII.3.4)

C를 만들 때 Source A·B의 해석 결과(`ResolvedSource`: commit OID·snapshot 원천·worktree)와 내용 지문(path·mode·blob ID)을 기록합니다.
적용 직전에 같은 요청을 다시 해석해 해석 결과가 같은지 보고, index·작업 트리 source는 같은 materialize 함수로 지문을 다시 계산해
비교합니다. 다르면 `StaleSource`이고 아무것도 만들거나 쓰지 않습니다. target 쪽 변경은 결정 6의 잠금 안 revision 비교가
`StaleTarget`으로 막습니다.

## Alternatives

- **창 session이 base 문서 사본을 메모리에 둠**: 문서 크기만큼 메모리를 늘 씁니다. content-addressed chunk는 불변이라 필요할 때
  loaded generation에서 다시 읽는 편이 같은 결과를 냅니다. chunk가 지워진 경우만 병합하지 않습니다.
- **conflict면 계속 ReconciliationPending으로 둠**: 사용자가 벗어날 길이 창 닫기뿐입니다. lane 정의대로 두 선택지로 둡니다.
- **runtime → semantic-merge 직접 의존**: 결정 3. 두 번째 조합 경로와 crate graph 변경이 생깁니다.
- **Merge Composer 적용이 merge commit까지 만듦**: §27.2·XXIII.3.3이 금지합니다. 탈락.
- **기존 `serialize_merged_generation`(파일 목록 반환)으로 쓰기**: 잠금·base 검사 없는 파일 목록이고 문서가 아닌 chunk를 잇지
  않습니다. target 쓰기는 저장 파이프라인을 씁니다.
- **새 worktree 생성 실패 시 자동 삭제**: 파괴적 명령을 자동 실행하게 됩니다. 탈락.

## Consequences

- 외부 writer와 겹친 창은 겹치지 않는 변경이면 자동 병합되고 자동 저장이 다시 돕니다. 겹치면 두 선택지 알림이 한 번 나옵니다.
  XXIV.4.6(writer 종류 구분)과 XIX.7.2(agent lease 겹침)는 이 경로를 씁니다.
- 이전 시험 세 개(외부 writer + dirty → 대기)는 XIII.3.4 동작(자동 병합)으로 바꿨습니다.
- Merge Composer는 엔진 API로 C를 세 target 가운데 하나에 쓸 수 있습니다. CLI·MCP 명령과 UI는 아직 없습니다.

## Compatibility

- 파일 포맷·generation manifest·journal 형식·C ABI·FFI 헤더를 바꾸지 않습니다. 보호 경로 변경이 없습니다.
- 공개 Rust API 추가: `stellar_semantic_merge::apply::{reconcile, Reconciliation}`, `stellar_ops::merge_apply::{document,
  target, composer}`, `stellar_runtime::reconcile::{ExternalConflict, ReconcileBlock, ConflictEntry}`,
  `ProjectWindowSession::external_conflict`.
- 동작 변경: 외부 writer + dirty 창의 `sync_external`이 `Stale { ReconciliationPending }` 대신 `Applied` 또는
  `Stale { Choose }`를 돌려줍니다.

## Performance

- reconciliation은 base 문서 chunk 읽기, 세 문서의 snapshot 투영·왕복 검사, 3-way merge로 문서 크기에 비례하며 document thread에서
  실행합니다. 대형 문서 측정은 하지 않았습니다(후속).
- Merge Composer 재검사는 index·작업 트리 source를 다시 materialize하므로 그 source 크기에 비례합니다.

## Security

- target 검증·생성은 git-porcelain gate 실행기(신뢰하지 않은 프로젝트는 프로세스 0)와 고정 argv(경로 앞 `--`)만 씁니다.
- 새 worktree 경로는 절대 경로만 받고 `..`·`.` 구성 요소를 거부합니다. 기존 worktree 안에 만들지 않습니다.
- materialize한 source의 컴포넌트 원본은 외부 symlink 내용을 읽지 않는 `MetadataOnly`로 관측합니다.

## Migration

기존 데이터 migration은 없습니다.
