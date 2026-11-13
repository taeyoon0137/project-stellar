# ADR-0017: library cache entry 범위와 content hash

## Status

Accepted (2026-11-14, 구현 기록)

## Context

XV.3.3은 exact commit의 Stellar project files를 content-addressed immutable cache entry로 만들도록
요구합니다. XV.4.2는 각 lock entry의 `content_hash`에 "cached content hash"를 기록하라고 하고,
XV.4.4는 그 값과 cache entry를 다시 대조합니다. 따라서 entry hash는 Git이 추적하는
`stellar.lock.yaml`에 들어가는 값입니다. 계산 방법이 바뀌면 같은 commit의 lock이 달라지므로 계산
규칙을 고정해야 합니다.

제약은 다음과 같습니다.

- dependencies crate는 core-types에만 의존하고 Git 객체를 직접 파싱하지 않습니다(XV.2.4, 아키텍처
  24.1). blob은 시스템 Git(`ls-tree`, `cat-file --batch`)으로만 읽습니다.
- hash 알고리즘은 ADR-0004의 SHA-256이고, 문자열은 `sha256:<64자리 소문자 hex>`입니다.
- 같은 hash를 디스크의 entry에서도 다시 계산할 수 있어야 합니다(XV.4.4). Git object id(SHA-1·
  SHA-256 저장소마다 다름)에는 의존할 수 없습니다.

## Decision

1. **entry 범위**는 commit tree의 Stellar project files입니다(아키텍처 19장). `stellar.yaml`,
   `stellar.lock.yaml`, 그리고 `design/`·`assets/` 아래 파일이 여기에 속합니다. 그 밖의 파일(README,
   소스, `.gitignore` 등)과 `.git`은 materialize하지 않습니다. `stellar.yaml`이 없으면 일반 Git
   저장소로 보고 거부합니다.
2. **허용 항목**은 일반 파일 blob(`100644`, `100755`)뿐입니다. 범위 안의 symlink(`120000`)와
   submodule(`160000`)은 거부합니다. 경로 규칙도 확인합니다. UTF-8이어야 하고, 빈 component·`.`·
   `..`·`.git`(대소문자 무시) component와 제어 문자·`\`를 금지합니다. 대소문자만 다른 경로와
   파일/디렉터리 충돌도 금지합니다(아키텍처 19.2).
3. **canonical listing**은 다음과 같습니다.

   ```text
   stellar-library-entry 1\n
   <mode> SP <size 10진> SP <파일 SHA-256 소문자 hex> SP <path> \n
   ```

   파일은 path의 UTF-8 byte 오름차순으로 나열합니다. `mode`는 `100644` 또는 `100755`입니다.
4. **entry content hash**는 `sha256:` + SHA-256(listing)입니다. lock `content_hash`는 이 값이고,
   디스크 entry에서 다시 계산한 값도 이 규칙을 따릅니다. 실행 bit는 파일 권한에서 읽습니다.
5. **저장 위치**는 `<cache root>/entries/sha256-<hex>/`입니다. staging 디렉터리에 모두 쓴 뒤 `rename`
   한 번으로 공개합니다. 공개된 entry는 다시 쓰지 않습니다. 파일은 읽기 전용(0444/0555)입니다.
   같은 내용이면 commit·repository가 달라도 entry 하나를 공유합니다.
6. (repository, commit) → content hash 대응은 privacy mode index(XV.3.5)에 기록하고, 원본 URL은
   local source config에만 둡니다.

## Alternatives

- **Git tree object id를 그대로 hash로 사용:** 저장소의 object format(SHA-1/SHA-256)에 따라 값이
  달라집니다. 디스크 재계산에는 Git object 형식 계산이 필요해 탈락했습니다.
- **commit 전체 tree를 materialize:** library 저장소의 소스·문서까지 공용 cache에 복사되고, 같은
  project 내용인데 README만 다른 commit이 중복 저장되므로 탈락했습니다.
- **symlink를 그대로 materialize:** 읽기 전용 공용 cache가 외부 경로를 가리킬 수 있어 탈락했습니다
  (아키텍처 19.3의 비신뢰 외부 링크 비노출 원칙).

## Consequences

- 같은 project files를 가진 commit들은 같은 lock `content_hash`와 같은 entry를 공유합니다.
- library 저장소가 `design/`·`assets/` 아래에 symlink·submodule을 두면 dependency로 쓸 수 없습니다.
- listing 형식 버전(`stellar-library-entry 1`)이 바뀌면 lock `content_hash`도 바뀌므로 새 ADR이
  필요합니다.

## Compatibility

- `stellar.lock.yaml`의 `content_hash` 의미가 이 규칙으로 고정됩니다(형식은 ADR-0004 문자열 그대로).
- cache 디렉터리 구조(`entries/`)는 사용자별 app data이며 Git이 추적하지 않습니다.
- `GitCommandRunner`에 stdin 입력 메서드(`run_git_with_input`)를 추가했습니다(crate 내부 trait,
  git-porcelain `GitRunner::run_with_input`과 같은 계약).

## Performance

- 새 commit마다 project files를 한 번 읽고 씁니다. 같은 commit을 다시 요청하면 index 조회만 하고 Git을
  실행하지 않습니다. blob은 256개 또는 64 MiB 단위 batch로 읽습니다. 파일 수(기본 100,000)와 총
  크기(기본 4 GiB)에 상한이 있습니다.

## Security

- 신뢰할 수 없는 library tree의 경로·종류를 공개 전에 검증하고, symlink를 만들지 않습니다.
- `cat-file` 출력 해석은 크기 overflow 없이 경계를 검사합니다.
- 오류에는 경로·URL 원문을 담지 않습니다.

## Migration

- 해당 없음. 이 결정 전에는 cache entry도 non-empty lock도 생성된 적이 없습니다.
