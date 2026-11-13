# 작업 보고서: XXVI.4.1 tags·notes·bisect·sparse-checkout·submodule 읽기

- WBS: XXVI.4.1
- 작업명: tags, notes, bisect, sparse-checkout, submodule 상태의 read UI를 우선 구현(core API)
- 상태: 완료 (`[x]`)
- 선행: Phase XIV(phase 수준, 완화 적용)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/advanced/inspect.rs` (신규: `list_tags`, `list_notes`, `read_bisect_state`, `read_sparse_checkout`, `list_submodules`, `TagInfo`, `NoteEntry`, `BisectState`, `SparseCheckoutState`, `SubmoduleInfo`, 테스트)
- `crates/git-porcelain/src/advanced/mod.rs` (`inspect` 모듈)
- `docs/task-reports/XXVI.4.1.md`(신규), 계획서(XXVI.4.1 `[ ]`→`[x]`)

## 구현 요약

- **읽기 전용**입니다. 쓰기 기능은 XXVI.4.2 정책(별도 작업·승인, 기본 비활성 flag)을 따릅니다.
- tags: `for-each-ref refs/tags`(이름·annotated tag object·한 단계 peel한 대상·제목). notes: `notes list`(기본 notes ref).
  bisect: `refs/bisect/`(bad·good-*·skip-*·사용자 용어 보존) + `--git-path BISECT_START` 존재. sparse-checkout:
  `core.sparseCheckout`·`core.sparseCheckoutCone` + `sparse-checkout list`. submodule: index gitlink + `.gitmodules`
  (`config --file`) 이름·URL + local config 초기화 여부 + 경로의 `.git` 존재(checkout 여부).
- submodule 조회는 submodule 저장소 안에서 Git을 실행하지 않습니다. submodule URL은 민감값(`RemoteUrl`, Debug 비노출)입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 223 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **read fixture**(실제 Git): 빈 상태 기본값; lightweight·annotated tag(대상·tag object·한글 제목); note 1개 대상; bisect start/bad/good → active·bad·good 1, reset 후 비활성; cone sparse-checkout(`design`) → enabled·cone·패턴, 다른 경로 미checkout; 로컬 submodule → 경로·이름·URL·초기화·checkout·기록 commit 정확, Debug에 URL 없음; 조회 전후 status 동일(`read_only_inventory_fixture`) | 통과 |
| `config -z --get-regexp` 레코드 해석 | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 "Phase XIV"는 phase 수준 선행이라 완화 규칙을 적용했습니다. UI 없이 core API만 제공합니다.
- submodule 상태는 `git submodule status`(submodule 안에서 Git 실행) 대신 index·`.gitmodules`·local config로 읽습니다.

## 남은 위험 / 한계

- submodule 작업 트리의 수정·untracked 여부(`git submodule status`의 `+`/`-` 표시 일부)는 중첩 저장소 Git 실행이
  필요해 제공하지 않습니다.
- 기본이 아닌 notes ref, bisect 사용자 용어의 의미 해석은 하지 않습니다(이름·값만 보존).
