# ADR-0093: 에이전트 권한을 세션 메모리에 두고 편집 lease를 change set 판정과 파일 잠금 조정자로 구현한다

## Status

Accepted (2026-11-23). 엔진 우선 방향(2026-11-23, UI는 나중에 다시 설계)에 따라 `agent-lease` lane이 정했습니다.

- 아키텍처 §31.3(MCP 권한)·§31.4(일반 편집과 동일한 Agent Command)와 ADR-0005 §3·§4를 구현 수준에서 구체화합니다. ADR-0005의 결정은 바꾸지 않습니다.
- 대상 행: XIX.3.2\~XIX.3.4, XIX.4.1\~XIX.4.7, XIX.5.2, XIX.5.3의 엔진 부분. XIX.3.1(권한 어휘)과 XIX.5.1(closed enum proposal)은 ADR-0085가 이미 정했고 그대로 씁니다.
- 보호 경로(core-types, storage, ffi, schemas, 성능 예산)는 바꾸지 않습니다. 셸 FFI와 Swift UI는 범위가 아닙니다.

## Context

- §31.4는 에이전트가 대상 node를 선언하고, 사용자가 범위를 승인하면 그 subtree를 독점 잠그고, 잠긴 subtree는 같은 프로젝트의 모든 창·프로세스에서 사용자 편집을 막는다고 정합니다. 잠금은 프로젝트 데이터나 Git에 저장하지 않고, 다른 worktree는 잠금을 공유하지 않습니다.
- 기존 D8 잠금(`stellar_commands::lock`, 노드의 `locked` 필드)은 저장되는 문서 속성이라 이 lease와 다릅니다. 사용자 편집 판정은 앱 여러 곳(`edit_permission`)과 CLI apply(`NodeLock::resolve` + 레코드 비교)에서 이 모듈의 같은 함수를 씁니다.
- command 실행은 `DocumentState::execute_transaction` 하나로 지나가고, 결과 `ChangeSet`이 바뀐 node·추가·삭제·속성 경로를 의미 단위로 기록합니다(VII.1.4).
- 저장 잠금(XXIV.4.5, `stellar_ops::lock`)이 이미 OS 사용자 데이터 영역의 `flock(2)` 파일 잠금과 프로젝트 identity key(project id ‖ canonical target)를 씁니다.
- crate graph: runtime은 mcp-core에 닿을 수 없고(ADR-0005 §8), CLI(stellar-cli)는 ops까지만 닿습니다. 세 쪽(앱 runtime, CLI, MCP 브리지)이 모두 닿는 공통 crate는 commands입니다.

## Decision

1. **lease 판정은 commands crate에 하나만 둡니다** (`stellar_commands::lock::lease`).
   - `Lease`는 lease id(= 에이전트 편집 세션 id, `AgentProvenance::edit_session`), subtree 잠금 목록(`SubtreeLock`: 문서, root, 잠금 시점 조상 목록), 전역 resource 잠금 목록(`ExternalResource`: token·component-definition·asset + id)입니다.
   - node가 속한 lease는 자신부터 조상을 따라 올라가 lease root를 찾아 정합니다. 그래서 lease 뒤에 추가된 자손도 자동으로 잠깁니다(XIX.4.2). 자손 목록을 미리 저장하지 않습니다.
   - 판정 함수는 `LeaseTable::judge(적용 전 소속, 적용 후 모델, ChangeSet, 주체)` 하나입니다. 사용자 편집 거부(XIX.4.3)와 에이전트 proposal 범위 검사(XIX.5.2)가 이 함수를 같이 씁니다. command 종류별로 대상을 따로 계산하지 않습니다(이중 판정 금지).
   - 규칙: 바뀐·추가·삭제 node마다 적용 전 소속(잠긴 subtree의 node 집합, 비용은 잠긴 subtree 크기)과 적용 후 소속(조상 탐색)을 봅니다. 사용자는 어느 쪽이든 lease 안이면 거부합니다. 부모·순서 속성이 바뀐 node가 lease root의 진조상이면(상위 이동) 거부합니다. 상위 삭제는 lease root 자체가 삭제 집합에 들어가 거부됩니다. 에이전트는 존재하는 쪽 모두 자기 lease 안이어야 하고, 자기 lease root 삭제는 거부합니다. 조회·선택은 command가 아니라 막지 않습니다.
   - 판정 주체는 transaction 출처입니다. `User`는 사용자, `FutureAgent(provenance)`는 그 lease입니다. `System`(외부 변경 반영)·`Migration`은 거부하지 않습니다. 외부 변경이 lease와 겹치면 ADR-0005 §4.2에 따라 세션을 끝내야 하므로, 겹친 lease를 `touched_leases`가 같은 소속 계산으로 알려 줍니다.
2. **판정을 거친 실행은 `execute_leased` 하나입니다.** 사용자 편집인데 문서에 lease가 없으면 판정 없이 그대로 실행합니다(빠른 경로). 판정할 때는 같은 실행 경로로 적용한 뒤 change set을 판정하고, 거부하면 역 transaction으로 되돌리고 revision을 실행 전 값으로 돌립니다.
   - 이 방식을 고른 이유: commands의 실행 loop를 복제하지 않고(실행 이중 경로 금지), 문서 사본을 만들지 않아 허용되는 편집의 비용이 판정 비용(잠긴 subtree 크기 + 바뀐 node 수 × 깊이)만 늡니다. 거부 경로만 역 transaction 비용을 냅니다.
   - 거부된 실행은 change set·역 transaction을 밖에 내보내지 않으므로 관찰되지 않습니다. 되돌리기가 실패하면 `Restore` 오류로 내부 불일치를 알립니다.
3. **전역 resource 잠금**(XIX.4.4): token, component definition, asset은 문서 node가 아니므로 lease 요청에 명시해야 합니다. instance node를 잠가도 그 정의를 자동으로 잠그지 않습니다. 편집 경로는 `LeaseTable::check_external(resource, 주체)`로 판정합니다. 사용자는 어느 lease도 쥐지 않은 resource만, 에이전트는 자기 lease가 명시적으로 잠근 resource만 바꿉니다.
4. **겹침**(XIX.4.6): 같은 문서에서 root가 같거나 한쪽 root가 다른 쪽의 조상이면 겹칩니다(`SubtreeLock::overlaps`). 조상 목록을 잠금에 적어 두므로 문서 모델이 없는 다른 프로세스도 같은 판정을 합니다. 한 요청 안의 대상끼리 겹치거나 resource가 중복이면 요청 형식 오류입니다.
5. **프로세스 간 공유는 파일 잠금 조정자입니다**(XIX.4.5, `stellar_commands::lock::coordinator::ProjectLockCoordinator`).
   - 위치: `<저장 잠금 디렉터리>/agent-leases/<project identity key>/`. key는 저장 잠금과 같은 `stellar_ops::lock::lock_key`라 symlink로 연 같은 프로젝트는 같은 공간, 다른 worktree는 다른 공간입니다. 프로젝트 폴더와 Git 밖이고 디렉터리는 `0700`, symlink는 따라가지 않습니다.
   - lease 하나 = `lease-<id>.txt`, gesture 하나 = `gesture-<id>.txt`. 쥔 쪽이 파일을 연 채 `flock(2)` 배타 잠금을 유지합니다. 조회는 잠기지 않은 항목(쥔 프로세스가 끝남, 비정상 종료 포함)을 지우고, 잠긴 항목만 읽습니다. macOS `flock(2)`은 open file description 단위라 같은 프로세스의 다른 창이 따로 연 파일과도 충돌합니다.
   - 조회·획득·해제는 `coordinator.lock` 배타 잠금 안에서만 합니다. 디렉터리 하나를 읽고 파일 하나를 쓰는 짧은 구간이라 기다립니다.
   - 항목 형식은 줄 단위 텍스트입니다(첫 줄 `stellar-agent-lease 1`). 살아 있는 항목이 형식에 맞지 않으면 실패로 닫습니다(획득 거부).
6. **gesture 대기**(XIX.4.7): 사용자 상호작용 미리보기(drag 등)는 조정자에 gesture 항목으로 표시합니다. gesture와 겹치는 lease 요청은 `Conflict::Gesture`로 거부하고, 요청 관리자(`stellar_mcp::lease::AgentEditLeases`)는 그 요청을 대기 상태로 두었다가 gesture가 끝난 뒤 `retry_waiting`에서 획득합니다. 반대로 lease 안 node에서 gesture를 시작하는 것은 거부합니다.
7. **`BeginAgentEdit`**(XIX.4.1): tool 이름은 `document.begin_agent_edit`, attached 전용, 권한 Propose(문서를 바꾸지 않는 대상 선언·잠금 요청)입니다. 인자는 `document`, `targets`(1\~1,000개 StableId), `externals`(선택, `{kind, id}`), `description`(1\~2,000자)입니다. 빈 대상은 `empty-targets`, 문서에 없는 node는 `missing-node`(not-found), 중복·겹침·빈 설명은 invalid-arguments입니다. 제출하면 승인 대기이고 아직 잠그지 않습니다. 사용자가 승인하면 그때 lease를 획득합니다.
8. **권한 저장**(XIX.3.2): `stellar_mcp::permission::PermissionStore`는 key(project id, canonical target, 에이전트 세션 id)별 상태를 메모리에만 둡니다. 파일·직렬화 경로가 없어 앱을 다시 시작하면 권한이 없는 상태입니다.
9. **첫 연결 승인과 backend 정책**(XIX.3.3): attached 연결은 신뢰된 프로젝트라도 첫 연결 승인 전에는 Read를 포함한 모든 권한이 대기(`AwaitingConnection`)입니다. 승인하면 Read만 허용됩니다. headless는 신뢰된 고정 프로젝트에서 상태를 바꾸지 않는 권한(`Permission::is_non_mutating`, Read·Propose)만 허용하고 나머지는 `NotOffered`입니다. headless `tools/list`의 tool이 요구하는 권한이 모두 이 정책의 허용 집합 안에 있는지 test가 확인합니다. 신뢰되지 않은 프로젝트는 어느 backend에서든 모두 거부합니다.
10. **독립 확인**(XIX.3.4): Read를 뺀 권한은 권한마다 확인 요청 id와 상태를 따로 가집니다. 한 응답은 그 요청의 권한 하나만 바꾸고 첫 응답만 유효합니다. Propose·Apply·Terminal·FileDelete 승인은 세션 동안 유지하고, GitCommit·GitPush 승인은 실행 한 번에만 씁니다(§31.3 개별 확인). 응답 함수는 앱 사용자 UI만 부릅니다(ADR-0005 §3 변경 불가 경로).
11. **승인 전 미리보기**(XIX.5.3): `stellar_commands::lock::lease::preview`가 문서 레코드로 사본을 만들어 적용하고 lease 판정을 붙입니다. 원본 상태는 읽기만 하므로 문서와 revision이 바뀌지 않습니다. `stellar_mcp::attached::edit::preview`는 그 결과에 ops의 `diff_records`(semantic diff)와 `validate_records`(`stellar validate`와 같은 진단)를 붙입니다. 범위 밖 proposal도 미리보기는 만들고 `scope`가 위반입니다. 승인된 적용(`apply_approved`)은 `execute_leased`로 같은 판정을 다시 합니다.

## Alternatives

- **command 종류별 대상 계산(정적 판정)**: Command enum마다 바꿀 node를 따로 계산해야 하고 실제 적용 결과와 어긋날 수 있습니다(이중 판정). 채택하지 않습니다.
- **문서 사본에 적용해 판정한 뒤 사본을 상태로 교체**: 허용되는 사용자 편집마다 O(문서) 사본 비용이 생기고, 사본은 runtime 핸들을 다시 할당해 앱 cache 핸들과 어긋납니다. 미리보기에만 사본을 씁니다.
- **실행 loop를 lock 모듈에 복제해 guard 안에서 판정 후 rollback**: 실행 경로가 둘이 되어 `run_requests`가 바뀔 때 어긋납니다. 채택하지 않습니다.
- **local IPC(앱 프로세스가 coordinator 서버)로 lease 공유**: 앱이 없을 때 CLI가 lease를 확인할 수 없고, 앱 instance가 여럿이면 서버를 정해야 합니다. 파일 잠금은 이미 저장 잠금이 쓰는 방식이고 프로세스 종료 시 OS가 풀어 줍니다.
- **lease 정보를 프로젝트 폴더에 둠**: §31.4(잠금은 프로젝트 데이터·Git에 저장하지 않음)와 맞지 않습니다.
- **permission 상태를 파일에 저장**: XIX.3.2(재시작 후 유지하지 않음)와 ADR-0005 §3(변경 불가 경로)에 맞지 않습니다.

## Consequences

- 사용자 편집·에이전트 편집·CLI가 같은 `LeaseTable::judge`로 판정하게 됩니다. 실제 연결은 다음 작업입니다.
  - 앱 runtime의 사용자 편집 경로를 `execute_leased`로 바꾸고, 창마다 `ProjectLockCoordinator`에서 lease 목록을 읽는 일(UI 재설계와 함께, XII 계열).
  - `stellar apply`(ops::apply)가 쓰기 전에 조정자의 lease 목록으로 같은 판정을 하는 일(ops 소유).
  - `document.begin_agent_edit` 정의를 agent-protocol catalog 목록과 golden에 넣는 일(agent-protocol 소유). 지금은 `stellar_mcp::lease::BEGIN_AGENT_EDIT`에 정의만 있습니다.
- attached proposal의 `base_revision`(저장 generation 기준)과 창 세션 revision의 대조는 앱이 합니다. `set-property`·`insert-component-instance` 입력은 attached 경로에서 아직 거부합니다(`decode_commands` 규칙).
- lease 판정 비용은 lease가 있는 문서의 편집에서만 생깁니다. 잠긴 subtree가 매우 크면 판정 전 소속 기록 비용이 커집니다. §6.3 단일 점유 상한과의 관계는 실측 후 정합니다(ADR-0005 "lease 대상 계산과 batch 적용은 나눠 처리").

## Compatibility

- 파일 포맷·FFI·schema 변경은 없습니다. lease 항목 파일은 프로젝트 밖 임시 상태이며 형식 version 1로 시작합니다. version이 다른 살아 있는 항목은 손상으로 보고 획득을 거부합니다(fail closed).
- 공개 API 추가: `stellar_commands::lock::{lease, coordinator}`, `stellar_mcp::{permission, lease, attached}`.

## Performance

- lease가 없는 문서의 사용자 편집은 판정 없이 기존 경로입니다.
- lease가 있으면 실행 전 잠긴 subtree 순회(O(잠긴 node 수))와 실행 후 바뀐 node별 조상 탐색(O(바뀐 node 수 × 깊이))이 더해집니다.
- 조정자 조회는 디렉터리 하나 읽기와 항목 수만큼의 파일 열기·`flock` 확인입니다. 인터랙션 frame 경로에서 부르지 않고, 편집 commit이나 lease 요청 때 부릅니다.

## Security

- 권한·lease·gesture 상태는 프로젝트 파일과 Git 밖에 있고, 에이전트 경로(MCP)로는 응답·승인을 쓸 수 없습니다(ADR-0005 §3).
- 조정 디렉터리는 소유자 전용이고 symlink를 따라가지 않습니다. project key는 소문자 16진수만 받아 경로 조작을 막습니다.
- 손상된 살아 있는 항목은 획득을 거부합니다. 오래된 항목은 쥔 프로세스가 없을 때만 지웁니다.

## Migration

- 기존 데이터 마이그레이션은 없습니다. 기존 D8 잠금(`locked` 필드)과 판정 함수는 그대로입니다.
