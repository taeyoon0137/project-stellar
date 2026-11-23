# ADR-0081: Semantic Merge 결과 검증·해결 모델과 Git merge driver

## Status

Accepted (2026-11-23). 제품 책임자가 엔진 lane "semantic-merge"의 설계 판단을 위임했습니다. 이 ADR은 Phase XIV.10.7\~XIV.12,
XXIII.2, XXIV.2, XVI.6.6, XXVIII.2.3의 결정을 한 곳에 모읍니다. 기술 아키텍처 §27과 ADR-0059·ADR-0061을 바꾸지 않고
보완합니다.

## Context

- Semantic Merge 엔진(XIV.10.1\~XIV.10.6)은 base·ours·theirs snapshot을 `StableId + PropertyPath` 단위로 병합하지만,
  결과가 snapshot 무결성(validator, XIV.5.3)을 지키는지 확인하지 않았습니다. 서로 다른 대상의 변경이 합쳐져 의미가 깨지는
  경우(Property 삭제 대 사용 추가, token 타입 대 값)도 자동 병합했습니다.
- `stellar semantic-merge`는 세 작업 트리를 비교해 보고만 하는 읽기 전용 명령이고, 텍스트 원본의 바로 이웃한 두 줄을
  양쪽 브랜치가 바꾸면 Git 텍스트 충돌이 남았습니다(ADR-0061 Consequences). 핵심 가치 1(Git 협업)은 터미널의
  `git merge`만으로 의미 단위 병합이 되기를 요구합니다.
- 제약:
  - `crates/ops`와 `stellar-cli`의 허용 crate graph(ADR-0005 §8, `cargo xtask validate-crate-graph`)에는
    git-porcelain이 없습니다. 이 lane은 graph 정책 파일을 소유하지 않습니다.
  - 텍스트 원본 쓰기 형식 v4(ADR-0077 lane)와 다중 파일 journal(ADR-0078 lane)은 같은 시점에 다른 lane에서 만들어지고
    있어 이 lane의 기준 커밋에는 없습니다.
  - 셸 FFI·Swift UI는 만들지 않습니다(2026-11-23 사용자 방향). 기능은 Rust 테스트·CLI로 검증합니다.

## Decision

1. **전용 conflict(XIV.10.7).** 다음 세 경우는 서로 다른 대상이어도 따로 받아들이지 않습니다.
   - `token-type-vs-value`(code 8): 한쪽이 token `TOKEN_TYPE`, 다른 쪽이 같은 token의 `TOKEN_VALUE`·`TOKEN_EXPRESSION`을
     바꿨고 두 결과의 (타입, 값, expression)이 다르면 token 객체 전체가 conflict이며 candidate는 base token을 유지합니다.
   - `component-property-removed-vs-override`(code 7)·`drilling-chain-break`(code 9): 한쪽이 컴포넌트
     `COMPONENT_PROPERTIES` 목록에서 Property를 빼고, 그 Property를 목록에 둔 다른 쪽이 그 Property를 새로 사용하면 목록
     삭제를 보류하고 목록 속성 대상 conflict로 둡니다. 사용이 scene 인스턴스 override면 7, 정의 tree 안 nested 인스턴스
     override(drilling link)이거나 값 bytes가 그 Property StableId를 담으면(binding chain) 9입니다. 사용 변경은 목록이
     base로 남으므로 받아들입니다. drilling binding 자체는 아직 영속 레코드·snapshot에 없으므로(IX.4) snapshot에 보이는
     link만 판정합니다.
2. **결과 검증(XIV.10.8).** 병합 결과는 `introduced_violations(merged, [base, ours, theirs])`가 비어야 성공입니다. 새 위반이
   있으면 위반에 관련된 받아들인 변경(위반 종류별 단위: 객체 추가·삭제, 위치, 속성 경로, interface 목록)을 되돌려 그 단위를
   base 상태로 두고 `invalid-result`(code 10) conflict로 기록한 뒤 다시 적용합니다. 되돌릴 때마다 받아들인 변경이 줄어
   유한 번에 끝납니다. 되돌릴 변경이 없는 위반은 위반 객체 대상 `invalid-result`로 남겨 clean이 아니게 합니다. 어떤 경우에도
   위반이 있는 결과를 clean으로 돌려주지 않습니다.
3. **결정론적 순서(XIV.10.9).** conflict는 `(StableId, 대상)` 순서(대상은 객체 < 위치 < 속성 경로), 같은 키는 kind code
   순으로 정렬하고 같은 `(StableId, 대상)`은 하나만 남깁니다(code가 작은 종류). 변경 목록은 semantic-diff canonical 순서이고
   snapshot은 StableId map입니다. 그래서 같은 세 snapshot은 객체 삽입 순서와 무관하게 같은 `MergeResult` bytes를 냅니다.
4. **해결 모델(XIV.12.1\~XIV.12.3, XXIII.2).**
   - `Resolution`은 닫힌 enum `ChooseBase`·`ChooseOurs`·`ChooseTheirs`·`CustomValue(SemanticValue)`·`Delete`입니다. 계획서
     XIV.12.1의 `KeepModified`는 따로 두지 않습니다. 삭제 대 수정 conflict에서 수정 쪽을 고르는 것과 의미가 같아 같은 결과를
     두 이름으로 만들지 않습니다.
   - 허용 action은 대상별입니다. 속성 대상은 세 side 선택·`CustomValue`, 그리고 어느 side든 속성이 없으면 `Delete`. 객체
     대상은 세 side 선택과 `Delete`(`CustomValue` 없음). 위치 대상은 세 side 선택만. side 선택은 그 side 상태가 있어야 하고
     (객체 대상의 `Missing`은 삭제로 허용), 허용하지 않은 action은 `ResolutionError::NotAllowed`로 거부합니다.
   - 해결은 `CandidateSnapshot C`(메모리 값)에만 적용합니다. C는 입력 세 snapshot을 복사해 가지며 A·B를 바꾸지 않습니다.
     C는 객체·속성별 출처(`Base`·`A`·`B`·`Auto`·`Custom`)를 기록합니다. resolution·선택 command는 적용 결과와 함께 역
     command를 돌려줍니다.
   - 쓰기는 `ValidatedCandidate`로만 합니다. unresolved conflict가 0이고 C가 결과 검증(결정 2와 같은 기준)을 통과할 때만
     만들 수 있습니다.
5. **conflict 세션(XIV.11.3).** Git 작업 트리의 unmerged 경로를 Stellar 단위로 나눕니다: 컴포넌트 원본 파일
   `design/components/*.stellar.yaml`은 파일 하나가 단위, `design/current`는 generation 문서 단위, 그 밖은 raw conflict
   (XIV.11.4), 그 밖의 `stellar.yaml`·`design/` 경로는 이유와 함께 의미 병합에서 제외합니다. 세 stage를 snapshot으로 올려
   단위별로 병합하고, semantic conflict마다 원래 raw 경로를, raw 경로마다 그 단위의 객체 집합을 함께 보관합니다. generation
   단위는 세 stage pointer가 가리키는 세대의 manifest·chunk를 작업 트리에서 비공개 사본으로 열어 무손실 검사 뒤 씁니다. headless
   진입점은 `stellar merge-driver resolve <프로젝트> [--resolve <번호>=<action>]... [--apply]`이며, `--apply`가 없으면 C에만
   적용한 결과를 보여 주고 아무것도 쓰지 않습니다.
6. **WorkingTree 쓰기(XIV.12.4\~XIV.12.6).**
   - 모든 단위의 C가 검증된 뒤 쓰기는 한 번만 합니다. 쓰기 전에 unmerged 목록과 대상 파일 hash가 세션을 연 때와 같은지
     확인하고(stale이면 쓰지 않음), 파일마다 임시 파일 뒤 rename으로 씁니다. 컴포넌트 원본은 codec v4 encoder 결과, generation은
     기존 project save API 경로(문서 레코드 → chunk 직렬화 → save plan, ours 세대 기준)로 만든 chunk·manifest·
     `design/current`입니다. 그래서 텍스트 기본 저장 전환(V.10.7) 전후 모두 같은 저장 API를 거칩니다.
   - 쓰기 도중이나 `git add` 실패 시 이미 쓴 파일을 원래 bytes로 되돌리고(새로 만든 파일은 지움) unresolved 상태와 복구
     안내를 돌려줍니다. 다중 파일 journal(ADR-0078)이 들어오면 이 되돌리기를 그 journal로 바꿉니다.
   - `merge --continue`·`rebase --continue`·`cherry-pick --continue`·commit은 실행하지 않습니다. Git 포트에는 그런 메서드가
     없습니다. 결과에는 진행 중 작업 종류와 남은 unmerged 경로만 담습니다.
   - `resolve`(Git 읽기 포함)와 `git add`는 신뢰한 프로젝트에서만 실행합니다(기술 아키텍처 §24.3: 신뢰 전 저장소 안 Git
     실행·filter 비활성).
7. **Git 접근 포트.** ops에는 `MergeGit` trait(unmerged 목록, blob 읽기, `add`, 진행 상태, 저장소 로컬 config 읽기·쓰기)만
   두고 Git을 직접 실행하지 않습니다. 지금 제품 구현은 `stellar` CLI의 `merge_driver` 모듈이 시스템 Git을 고정 argv로 실행하는 adapter입니다
   (shell 없음, `--` 구분, `core.fsmonitor=false`, `GIT_OPTIONAL_LOCKS=0`). ops→git-porcelain edge가 정책에 생기면(ADR-0080 lane의 ops Git
   경로) adapter를 git-porcelain `GatedGitRunner` 기반 구현으로 바꿉니다.
8. **merge driver(XXIV.2.1·XXIV.2.2).** 명령은 `stellar merge-driver %O %A %B %P`입니다.
   - 대상은 텍스트 의미 파일 단위이며 지금은 컴포넌트 원본(`design/components/*.stellar.yaml`)입니다. 세 bytes를 같은
     codec으로 읽어(빈 base는 정의 없음) snapshot으로 올리고 엔진으로 병합합니다.
   - conflict가 없고 결과 검증과 codec 재해석 검증(쓴 bytes를 project decoder로 다시 읽어 같은 snapshot인지)을 통과하면
     `%A`에 결과 bytes를 쓰고 0을 반환합니다. 결과 bytes는 컴포넌트 원본의 유일한 쓰기 버전 v4(ADR-0077)이며, project
     codec의 공개 encoder(`ComponentSource::from_records`·`to_text`)가 씁니다(2026-11-23 통합 보완: lane 시점의 자체 v3
     encoder를 없앰). 재해석 검증은 그대로 둡니다.
   - conflict가 있으면 1(`unresolved-conflicts`), 병합 결과가 정의 삭제면 1(`merge-deleted`), 지원하지 않는 파일은 69,
     입력 없음은 66, 읽기·검증 실패는 65를 반환하고 어느 경우도 `%A`를 바꾸지 않습니다(CLI 종료 코드 표 XXIV.1.2). conflict 목록은
     프로젝트의 `.stellar/merge/`(자기 자신을 무시하는 `.gitignore`를 둠)에 JSON으로 남기고, Git 밖이므로 commit에 들어가지
     않습니다. driver는 Git을 실행하지 않습니다.
9. **등록(XXIV.2.3).** 신뢰한 프로젝트에서 사용자가 부른 `stellar merge-driver install <프로젝트>`만 등록합니다. 저장소
   로컬 `git config --local`에 `merge.stellar.name`·`merge.stellar.driver`(실행 파일 절대 경로)를 쓰고 프로젝트 루트
   `.gitattributes`에 `design/components/*.stellar.yaml merge=stellar` 한 줄을 더합니다(하위 폴더의 `.gitattributes`는 그 폴더
   기준이라 접두사가 필요 없고, 이미 있으면 그대로). 전역·시스템 config는 쓰지 않습니다. 다른 명령(driver 실행, `semantic-merge`, `resolve`)은 config와 `.gitattributes`를 바꾸지 않습니다.
10. **conformance corpus(XXVIII.2.3)와 fuzz(XVI.6.6).** corpus는 `crates/semantic-merge/conformance/`의 사례마다
    `MergeInput` canonical bytes(16진수 텍스트)와 기대 `MergeResult`·base→ours diff canonical bytes입니다. runner는
    `stellar_semantic_merge::conformance`의 공개 함수이고 공식 구현이 모든 사례를 bytes 단위로 통과해야 합니다. fuzz는 고정
    seed 결정론 harness로 결과가 (clean이고 새 위반 0) 또는 (명시 conflict) 또는 (구조화 오류)인지 봅니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| invalid 결과를 `MergeError`로 거부 | 사용자가 해결할 단서(어느 단위가 왜)가 사라집니다. conflict로 남겨 해결 모델로 풉니다. 미선택 |
| `KeepModified`를 별도 variant로 | 수정 쪽 선택과 같은 결과의 두 이름입니다. 미선택 |
| 해결 결과를 즉시 작업 트리에 쓰기 | 중간 상태가 디스크·index에 남고 검증 전 파일이 생깁니다. 미선택 |
| driver가 Git 텍스트 merge(`git merge-file`)로 대체 | 검증 없이 성공을 돌려주게 됩니다(결정 2와 충돌). 미선택 |
| ops→git-porcelain edge를 이 lane에서 추가 | 정책 파일(`tools/xtask`)이 이 lane 소유가 아니고 ADR-0080 lane과 겹칩니다. 포트로 분리합니다. 미선택 |
| `.stellar/merge/`를 프로젝트 `.gitignore`에 추가 | 사용자 파일을 바꿉니다. 폴더 안 `.gitignore` 하나로 Git 밖에 둡니다. 미선택 |

## Consequences

- 장점
  - 같은 Frame의 이웃한 두 줄(인접 모서리 등)을 두 브랜치가 바꿔도 driver를 등록한 저장소에서는 `git merge`가 의미 단위로
    끝납니다(ADR-0061 Consequences의 남은 단점 해소).
  - 위반이 있는 병합 결과가 clean으로 나오지 않습니다. 해결·쓰기도 검증을 통과해야 합니다.
- 단점
  - driver는 아직 컴포넌트 원본만 다룹니다. View·문서 텍스트 원본은 V.10 전환 뒤 같은 dispatch에 codec을 더합니다.
  - generation 문서 병합은 driver가 아니라 conflict 세션(`resolve`)에서만 합니다(`design/current`는 단일 줄 pointer).
  - CLI adapter가 git-porcelain을 거치지 않고 시스템 Git을 실행합니다. 결정 7의 통합 때 바꿉니다.
  - `resolve`의 generation 쓰기는 문서 집합이 바뀐 병합(문서 추가·삭제)을 지원하지 않고 Unsupported로 거부합니다.

## Compatibility

- 교환 인코딩: `ConflictKind` code 7\~10을 더했습니다. 기존 code 1\~6의 의미는 그대로이며 `MergeResult` 형식 version은 1을
  유지합니다(저장 포맷 아님).
- 공개 API: semantic-diff `validate::override_member`를 공개했고 `records::component_records`를 더했습니다.
- CLI: `merge-driver` 명령을 더했습니다. 기존 명령의 출력 schema는 그대로이며 conflict 종류 이름만 늘었습니다.
- 보호 경로(`crates/core-types`, `storage`, `ffi`, `schemas`, 성능 예산)는 바꾸지 않습니다.

## Performance

- 결과 검증은 병합마다 `validate_snapshot`을 candidate와 세 입력에 한 번씩 더 실행합니다(snapshot 크기에 선형, 정렬 집합
  log 인자). 되돌리기 반복은 되돌린 변경 수만큼입니다. 성능 예산 판정 대상 경로(편집·렌더)는 바꾸지 않습니다.

## Security

- driver와 세션은 신뢰하지 않는 Git 객체 bytes를 읽습니다. 파일당 크기 상한(원본 1 MiB)과 기존 제한 YAML loader를 거치고,
  실패는 nonzero로 끝나며 `%A`를 바꾸지 않습니다.
- `git add`(filter 실행)는 신뢰한 프로젝트에서만 합니다. 등록은 명시 명령으로만 하며 전역 config를 바꾸지 않습니다.

## Migration

- 기존 데이터 변환은 없습니다. 기존 저장소는 `stellar merge-driver install`을 실행한 뒤부터 driver를 씁니다.

## 개정(2026-11-23, integration-followups)

1. **결정 7 이행: Git 접근은 ops Git 경로 하나.** ADR-0080 lane으로 ops→git-porcelain edge가 생겨, CLI의 자체 시스템 Git
   adapter(`tools/stellar-cli/src/merge_driver/system_git.rs`)를 지우고 `stellar_ops::candidate::git_port::GatedMergeGit`
   (git-porcelain `GatedGitRunner` 기반 `MergeGit` 구현)으로 바꿨습니다. Git 탐색·환경 allowlist·timeout은 Stellar Git
   엔진과 같은 `system_git_runner`이고, 신뢰 판정은 `os_git_trust`(OS 사용자 신뢰 등록)입니다. 신뢰하지 않은 프로젝트에서는
   실행 전에 거부하고 프로세스를 만들지 않습니다(spawn 0 test). 고정 argv·shell 없음·경로 앞 `--`·`core.fsmonitor=false`·
   `GIT_OPTIONAL_LOCKS=0`·`GIT_TERMINAL_PROMPT=0` 규칙은 그대로입니다. 포트의 동작 목록(continue·commit·전역 config 없음)은
   바꾸지 않았습니다.
2. **driver 결과 bytes 확인.** 시스템 Git merge driver 통합 test가 병합 결과 파일이 컴포넌트 원본 v4
   (`schema_version: "4"`)인지 확인합니다.
3. **결정 6 이행: WorkingTree 쓰기는 다중 파일 journal 거래 하나.** `crates/ops/src/candidate/apply.rs`의 프로세스 내부
   되돌리기와 `DiskFiles::write`(fsync 없는 임시 파일 rename)를 지우고, 계획한 모든 대상(chunk → 컴포넌트 원본 →
   manifest → `design/current`)을 `stellar_storage::journal::commit` 거래 하나로 씁니다(ADR-0078). 반영 순서는 계획
   순서 그대로라 chunk → manifest → current가 유지됩니다. 대상마다 기대 hash(CAS)를 걸어 세션 뒤 외부에서 바뀐 파일은
   쓰지 않고 `Stale`로 끝냅니다. 쓰기 도중 실패하면 journal이 모두 앞 세대로 되돌리고, 프로세스가 죽으면 다음 적용
   (`recover_interrupted`, `apply_session` 시작, 또는 다음 journal 저장)이 marker 유무로 앞·새 세대 한쪽으로 맞춥니다.
   journal 오류 뒤에는 실제 파일을 다시 읽어 새 세대로 확정됐으면 성공으로 이어 갑니다(ADR-0078의 marker 뒤 오류
   규칙). `git add` 실패는 쓴 대상을 이전 bytes로 되돌리는 역 거래 하나를 commit합니다. CLI `resolve --apply`는 세션을
   열기 전에 남은 거래를 복구해, 세션이 복구된 작업 트리를 관찰하게 합니다. preview(`--apply` 없음)는 아무것도 쓰지
   않으므로 복구도 하지 않습니다.
