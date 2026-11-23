# ADR-0078: 여러 의미 파일 저장 journal과 한쪽 세대 복구

## Status

Accepted (2026-11-23, V.10.5 구현 기록)

## Context

- ADR-0059는 Git 원본을 의미 단위 텍스트 파일로 바꾸고, 여러 파일 저장의 원자성은 저장 journal로 보장한다고 정했습니다. journal 형식은 후속 결정으로 남겼습니다.
- 기술 아키텍처 §21은 "다중 텍스트 파일의 원자성은 V.10.5의 저장 journal로 검증한다"고 적습니다. 단일 파일 저장(`stellar_storage::source_file::write_checked`, V.10.1~V.10.2)은 한 파일의 temp/fsync/rename만 보장합니다.
- 한 번의 저장은 View 파일 몇 개 교체, 컴포넌트 파일 이름 바꾸기, 페이지 삭제를 함께 담을 수 있습니다. 파일마다 따로 rename하면 중간에 종료됐을 때 일부만 새 내용인 섞인 세대가 Git 작업 트리에 남습니다.
- 텍스트를 기본 원본으로 전환하는 V.10.6·V.10.7이 이 보장을 전제합니다. 텍스트 codec 전체(V.10.4)를 기다리면 안전 선행이 늦어지므로 codec과 무관한 bytes 수준 API가 필요합니다.
- 보호 경로 `crates/storage/`를 바꾸므로 이 ADR로 기록합니다.

## Decision

### 1. codec과 무관한 API

- 입력은 변경 집합입니다. 변경 하나는 다음 셋을 가집니다(`stellar_storage::journal::JournalChange`).
  - 프로젝트 루트 기준 상대 경로(`/` 구분)
  - 새 bytes(`Write`) 또는 삭제(`Delete`)
  - 기대 원본 hash. `None`은 "파일이 없어야 함"이고, 삭제는 항상 hash가 있습니다.
- 이름 바꾸기는 옛 경로 삭제와 새 경로 생성 두 항목입니다(`JournalChange::rename`). 같은 거래라 함께 반영되거나 함께 되돌려집니다.
- `journal::commit(root, changes, limit)`과 `journal::recover(root)` 두 함수가 공개 API입니다. `limit`은 대상 하나의 현재·새 bytes 상한입니다.
- 현재 내용과 같은 bytes를 쓰는 변경은 기대 hash만 확인하고 거래에서 뺍니다.

### 2. 위치와 형식

- journal은 Git 밖 `.stellar/journal/`에 둡니다. 처음 쓸 때 `.stellar/journal/.gitignore`(`*`)를 만듭니다. 다른 내용이면 덮어쓰지 않고 거부합니다(cache와 같은 규칙).
- 거래 하나는 `.stellar/journal/tx-<id>/` 디렉터리입니다.
  - `manifest`: 대상 목록, 대상별 기대 hash·길이, 새 hash·길이
  - `orig/<번호>`: 원본 사본(원본 권한 유지)
  - `new/<번호>`: 새 내용(교체 대상은 원본 권한)
  - `committed`: commit marker
- manifest는 줄 단위 텍스트입니다. 줄 순서가 항목 번호입니다.

  ```text
  stellar-journal 1
  tx <id>
  write <기대 hash|none> <기대 길이> <새 hash> <새 길이> <상대 경로>
  delete <기대 hash> <기대 길이> none 0 <상대 경로>
  ```

- 거래 ID는 시각·process·일련번호의 소문자 16진수입니다. 거래 디렉터리는 `create_dir`로만 만들고 이미 있는 이름은 쓰지 않습니다.

### 3. 저장 순서

1. 경로 규칙을 검사합니다. `.stellar/journal/` 디렉터리 fd에 `flock`(std `File::try_lock`)을 겁니다. 이미 잡혀 있으면 기다리지 않고 `Busy`입니다. 잠금을 잡은 채 남은 거래를 먼저 복구합니다.
2. 모든 대상의 현재 bytes hash를 기대 hash와 비교합니다(CAS). 하나라도 다르면 대상 파일, 대상 부모 디렉터리, 거래 디렉터리를 만들지 않고 `Stale`(다른 경로 목록)을 돌려줍니다.
3. 새 파일의 부모 디렉터리를 한 단계씩 실제 디렉터리로 만듭니다. 대상 부모가 journal과 다른 장치면 `CrossDevice`로 거부합니다. 같은 장치 안에서만 rename이 원자적이기 때문입니다.
4. 거래 디렉터리를 만들고 manifest를 temp/fsync/rename으로 씁니다.
5. 원본 사본과 새 내용을 `create_new`로 쓰고 fsync합니다. 그다음 `orig/`, `new/`, 거래 디렉터리를 sync합니다.
6. 대상마다 차례로 반영합니다. 반영 직전에 부모 디렉터리 identity와 기대 hash를 다시 확인하고, 다르면 이미 반영한 대상을 되돌린 뒤 `Stale`입니다.
   - 교체는 `new/<번호>` → 대상 rename입니다.
   - 새 파일은 hard link로 게시합니다. 마지막 순간에 생긴 항목도 교체하지 않기 위해서입니다.
   - 삭제는 대상 unlink입니다.
7. 대상 부모 디렉터리를 sync하고 commit marker를 temp/fsync/rename으로 씁니다. marker가 새 세대 확정 지점입니다.
8. 거래 디렉터리를 `trash-<id>`로 옮긴 뒤 지웁니다. 이 정리가 실패해도 새 세대는 확정이므로 성공을 돌려주고 `cleanup_pending`으로 알립니다.

marker 전에 실패하면(오류·디스크 가득 참·권한) 같은 호출 안에서 반영한 대상만 되돌리고 거래를 버립니다. 되돌림이 실패하면 거래를 남겨 다음 복구에 맡깁니다. marker 기록 뒤 디렉터리 sync가 실패하면 marker를 지운 뒤 되돌립니다.

### 4. 복구 규칙

- `commit` 시작과 `recover`가 남은 거래를 이름 순서로 처리합니다.
  - `trash-*`: 지웁니다.
  - manifest가 없는 `tx-*`: 대상을 건드리기 전이므로 버립니다(`Discarded`).
  - manifest를 읽을 수 없거나(형식·경로 규칙 위반, 거래 ID 불일치) symlink면 대상을 건드리지 않고 `quarantine-<id>`로 옮깁니다(`Unreadable`).
- marker가 없으면 앞 세대로 되돌립니다(`RolledBack`). marker가 있으면 남은 반영을 끝내 새 세대로 맞춥니다(`RolledForward`).
- 판정은 대상별 현재 hash로만 합니다. 목표 세대 내용이면 그대로 둡니다. 반대 세대 내용이면 journal 사본으로 옮깁니다(교체는 rename, 없던 파일은 hard link, 지울 파일은 unlink). 옮기기 전에 사본 hash를 확인하고, 사본이 없거나 손상됐으면 그 대상을 건드리지 않습니다.
- 현재 내용이 앞·새 세대 어느 쪽도 아니면 건드리지 않습니다. 외부 수정, symlink, 정규 파일 아님, 상한 초과가 이 경우입니다. 나머지 대상은 정한 방향으로 맞추고, 거래는 원본 사본·새 내용과 함께 `quarantine-<id>`에 남깁니다(`Quarantined`). 보존한 거래는 다음 저장을 막지 않습니다. 사용자 작업을 덮어쓰지 않는 것이 섞인 세대 0건보다 우선합니다. 이 경우만 대상이 세 번째 내용을 가집니다.
- 복구는 몇 번 중단되어도 다시 실행할 수 있습니다. 사본을 rename으로 소비해도 그 대상은 이미 목표 hash라 다시 옮기지 않습니다.

### 5. 경로와 파일 접근 안전

- 대상 경로는 상대 경로여야 합니다. 빈 구성 요소, `.`·`..`, 제어 문자, `\`, 어느 위치든 `.git`(ASCII 대소문자 무시), 첫 구성 요소 `.stellar`를 거부합니다. 같은 대상은 ASCII 대소문자를 접어 비교해 두 번 받지 않습니다.
- 복구는 manifest를 신뢰하지 않고 같은 경로 규칙을 다시 적용합니다. `.stellar/`는 저장소가 Git으로 담아 올 수 있는 비신뢰 입력이기 때문입니다.
- 부모 디렉터리는 기존 `fs_guard`로 한 단계씩 `lstat`합니다(`existing_real_dirs`, `ensure_real_dirs`). symlink 디렉터리와 symlink 대상은 따라가지 않고 거부합니다. 대상은 canonical 루트 기준 해석한 경로로 `O_NOFOLLOW_ANY` 읽기를 하고, 정규 파일·크기 상한을 열린 handle로 확인합니다.

## Alternatives

| 대안 | 판단 |
|---|---|
| marker를 반영 전에 쓰는 redo log | 확정 뒤에는 항상 앞으로 진행하므로 원본 사본이 필요 없습니다. 그러나 확정 뒤 반영 중 오류(권한, 디스크)에서 같은 호출이 앞 세대로 되돌릴 수 없습니다. 원본 보호 요구와 맞지 않아 미선택 |
| temp를 대상 디렉터리에 두기 | 중단되면 Git 작업 트리에 `.name.tmp-*`가 남아 `git status`에 보입니다. journal 안에 두고 같은 장치 rename으로 옮기는 쪽을 선택 |
| 원본을 hard link로 보존 | 복사보다 싸지만, 사본이 같은 inode라 외부 도구가 대상을 제자리 수정하면 사본도 바뀝니다. 대상 크기 상한 안에서 복사를 선택 |
| 판정할 수 없는 대상이 있으면 거래 전체를 멈춤 | 남은 거래가 모든 다음 저장을 막고, 나머지 대상도 섞인 채 남습니다. 대상별 판정 + 보존을 선택 |
| V.10.4 codec 완료 후 착수 | codec과 bytes 원자성은 독립이라 기다릴 이유가 없습니다. 미선택 |

## Consequences

- 장점
  - V.10.6 이후 저장 경로는 바뀐 의미 파일 집합을 한 번에 넘기면 앞·새 세대 한쪽만 보게 됩니다.
  - 외부 변경은 쓰기 전 CAS와 반영 직전 재확인으로 `Stale`이 됩니다. 복구는 외부 수정을 덮어쓰지 않습니다.
  - fault matrix가 성공 실행의 모든 단계(20단계)에 중단·오류 네 종류를 주입하고, 복구가 중단되는 경우도 복구 단계마다 검사합니다.
- 단점
  - 원본 사본 복사로 저장 I/O가 바뀐 파일 크기의 약 두 배입니다.
  - 마지막 확인과 rename 사이에 비협조적 외부 writer가 대상을 바꾸는 짧은 창은 남습니다(`source_file`과 같은 한계).
  - 저장 중 만든 빈 부모 디렉터리는 되돌릴 때 지우지 않습니다. Git은 빈 디렉터리를 추적하지 않습니다.
  - Unicode 정규화만 다른 같은 이름은 같은 대상으로 판정하지 못합니다. 대소문자만 다른 이름 바꾸기는 같은 대상으로 보아 거부합니다.
  - Windows는 디렉터리 fd 잠금이 없어 열기에서 실패합니다(ADR-0006에 따라 보류).

## Compatibility

- 새 공개 모듈 `stellar_storage::journal`을 더합니다. 기존 공개 API, chunk·세대 저장 형식, FFI는 바꾸지 않습니다.
- crate 내부 helper를 더합니다.
  - `fs_guard::existing_real_dirs`
  - `FileIdentity::dev`
  - `atomic_file::sync_parent_dir`의 crate 공개
- manifest 형식(`stellar-journal 1`)은 Git 밖 로컬 형식입니다. 공개 포맷 동결 대상이 아니며, 다른 머리 줄은 `Unreadable`로 보존합니다.

## Performance

- 저장 비용: 바뀐 파일마다 사본·새 내용 두 번 쓰기와 fsync, 디렉터리 sync, manifest·marker 두 번의 atomic write입니다.
- 남은 거래가 없으면 복구 비용은 journal 디렉터리 목록 한 번입니다.
- `config/performance-budgets.yaml`의 저장 예산은 텍스트 기본 원본 전환(V.10.7)에서 실측합니다. 이 행은 수치를 바꾸지 않습니다.

## Security

- journal 경로와 내용은 비신뢰 입력으로 다룹니다.
  - 경로 규칙을 다시 검사해 프로젝트 밖·`.git`·`.stellar` 쓰기를 막습니다. `.git/hooks`로 코드를 실행하게 만드는 manifest를 거부합니다.
  - 사본 hash를 확인하기 전에는 옮기지 않습니다.
  - journal 안의 symlink 항목은 따라가거나 지우지 않습니다.
- manifest 읽기는 16 MiB, 항목은 65,536개, 경로는 4,096 bytes가 상한입니다.

## Migration

- 기존 데이터 이전은 없습니다. 기본 저장 경로는 아직 journal을 쓰지 않으며, V.10.6이 연결합니다.
- 프로젝트에 `.stellar/journal/`이 없으면 첫 `commit`·`recover`가 만듭니다.
