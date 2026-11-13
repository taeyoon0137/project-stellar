# 작업 보고서: XXIII.1.3 Source A·B·Base snapshot materialize

- WBS: XXIII.1.3
- 작업명: Source A/B/Base snapshot을 SnapshotProvider로 materialize하고 원본 worktree를 수정하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.2.6 (`[x]`). 이 lane의 XXIII.1.1·XXIII.1.2 결과를 사용합니다.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 테스트는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/composer/snapshot.rs`(신규) — `SourceSnapshot`, `ComposerSnapshots`,
  `ComposerSnapshotError`, `materialize_source`, `materialize_sources`.
- `crates/git-porcelain/src/snapshot/materialize.rs` — 내용을 항목마다 공급받는 `materialize_with`(작업
  트리 source용, 같은 안전 규칙), `create_unique_root`를 crate 내부 공유.
- `crates/git-porcelain/src/composer/mod.rs` — `snapshot` 모듈 등록·재노출.
- `docs/task-reports/XXIII.1.3.md`(신규), 계획서(XXIII.1.3 `[ ]`→`[x]`).

## 구현 요약

- **source별 provider**: commit·tree → `ls-tree` + `cat-file --batch`(XIV.2.4·XIV.2.6), index →
  `ls-files --stage`(XIV.2.5) + `cat-file --batch`, 작업 트리 → `ls-files` 목록 + 디스크 읽기(XIV.2.7,
  symlink 비추적). 모두 새 임시 디렉터리(`MaterializedSnapshot`, `.git` 없음, drop 시 삭제)에 씁니다.
- **내용 지문**(`entries`: path·mode·blob ID): commit·index는 Git이 기록한 ID, 작업 트리는 한 번 읽은
  바이트를 사본과 번호 이름 hash 입력 파일에 같이 쓰고 `git hash-object --no-filters --stdin-paths`
  (`-w` 없음)로 계산합니다. 이후 source 변경 검사(XXIII.3.4)의 기준입니다.
- **원본 비변경**: 쓰기 명령(`status`의 index stat 갱신, `hash-object -w`)을 실행하지 않습니다. 출력
  위치(`parent`)가 source·현재 worktree 안이면 아무것도 만들지 않고 `ParentInsideWorktree`입니다.
  개행이 든 출력 위치는 `InvalidParent`입니다. unmerged index는 `UnmergedIndex`(Semantic Conflict 경로
  사용)입니다.
- **A·B·Base**: `materialize_sources`는 비교 방식(XXIII.1.2)을 구하고 A·B를 materialize하며, `ThreeWay`
  일 때만 base commit을 Base로 materialize합니다(criss-cross·2-way는 `base: None`). linked worktree
  source는 그 worktree에서 읽습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 282 passed + doctest 3 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |
| `cargo check -p stellar-project --all-targets` | 통과 |

| 검사 | 결과 |
|---|---|
| **source hash 보존**: A = branch feature, B = 현재 작업 트리(staged 변경 위 unstaged 변경, untracked, 삭제, symlink, 실행 파일), Base = initial, 추가로 Index·linked worktree 작업 트리 materialize → 두 worktree의 HEAD·`for-each-ref`·index 파일 바이트·`count-objects -v`·작업 트리 파일(내용·mode·symlink 대상) 모두 불변 | 통과 |
| 다섯 사본 모두 `.git` 없음, 사본을 다시 hash한 값 = `entries` 지문, 작업 트리 지문 = 원본 파일의 `hash-object --no-filters` 값 | 통과 |
| 내용: 작업 트리 사본은 디스크 내용, index 사본은 staged 내용(삭제 파일 유지), A는 feature, Base는 initial, linked worktree 사본은 feature 내용, symlink는 링크로 보존, 실행 파일 mode 100755 | 통과 |
| drop 시 사본·hash용 임시 디렉터리 삭제, 원본 불변 | 통과 |
| 대조: 작업 트리 hash에 `-w`를 일시적으로 넣으면 object 수 비교로 테스트가 실패함을 확인 후 원복 | 확인 |
| 출력 위치가 worktree 안 → `ParentInsideWorktree`, 작업 트리 불변 / unmerged index → `UnmergedIndex`, 출력 위치에 아무것도 없음 | 통과 |
| 공통 이력 없는 source → `TwoWay`, Base 없음 | 통과 |
| `materialize_with` 검증 우선(`.git`·tree mode 거부 시 디렉터리·read 호출 0), hash 출력 해석(수 불일치 거부) | 통과 |

## 차이(작업 지시와 다른 점)

- 작업 트리 source는 Git object가 아니어서 `cat-file`로 읽을 수 없습니다. 같은 안전 규칙의
  `materialize_with`로 디스크 바이트를 복사하고, 지문은 object database에 쓰지 않는 `hash-object`로
  계산했습니다.

## 남은 위험 / 한계

- 작업 트리 지문은 읽는 시점 값입니다. 읽는 도중 사용자가 파일을 바꾸면 그 시점 바이트가 사본·지문에
  같이 들어갑니다(사본과 지문은 항상 일치). 저장 경쟁 처리는 XXIII.3.4 stale 검사에서 다룹니다.
- 큰 asset은 사본과 hash 입력 파일로 두 번 기록합니다. 대형 프로젝트 성능은 측정하지 않았습니다.
- Windows symlink materialize는 지원하지 않습니다(ADR-0006).
