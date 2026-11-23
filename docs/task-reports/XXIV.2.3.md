# 작업 보고서: XXIV.2.3 사용자 승인 명령으로만 driver 등록

- WBS: XXIV.2.3
- 작업명: `.gitattributes`/Git config 등록은 사용자 승인 명령으로만 수행하고 전역 config를 자동 변경하지 않음
- 상태: 부분 완료 (`[~]`) — 2026-11-23 plan-revision 계획서 판정: 계획 1.1의 "올바른 커밋"이 없음: 구현 커밋이 `Feat:`·`Fix:`·`Test:` 제목이라 `Task(<ID>)` 제목 커밋이 없고, 원장(`cargo xtask generate-ledger`)이 근거 커밋과 검증 시각을 채우지 못합니다. 구현·검증은 완료입니다; 선행 XXIV.2.1(`[~]`)가 `[x]`가 아님(계획 1.2 규칙 4). 보고 당시 기록: 완료 (`[x]`)
- 선행: XXIV.2.1
- 결정: [ADR-0081](../adr/ADR-0081-semantic-merge-resolution-and-git-driver.md) 결정 7·9

## 필수 실행 환경

- `DEV-MAC` (Rust, 시스템 Git 2.x, 로컬 검증).

## 변경 파일 목록

- `crates/ops/src/candidate/git_port.rs`(신규) — `MergeGit` 포트(unmerged, blob, add, 진행 상태, 저장소 로컬 config 읽기·쓰기).
- `crates/ops/src/candidate/install.rs`(신규) — `install_driver`(멱등, 로컬 config 두 key와 `.gitattributes` 한 줄).
- `crates/ops/src/candidate.rs` — 모듈 등록.
- `tools/stellar-cli/src/merge_driver.rs` — `install` 하위 명령(신뢰 확인), `merge_driver/system_git.rs`(신규) 시스템 Git adapter.
- `tools/stellar-cli/src/cli.rs`(설명 한 줄), `tests/fixtures/help.txt`, `tests/merge_driver.rs`.

## 구현 요약

- `stellar merge-driver install <프로젝트>`만 등록합니다. 신뢰한 프로젝트(OS 사용자 데이터 신뢰 저장소)가 아니면 77
  (`untrusted-project`)로 거부하고 아무것도 쓰지 않습니다.
- 저장소 로컬 `merge.stellar.name`·`merge.stellar.driver`(현재 실행 파일 절대 경로를 shell 한 단어로 감싼 명령)를 쓰고, 프로젝트
  `.gitattributes`에 `design/components/*.stellar.yaml merge=stellar`를 기존 줄 뒤에 더합니다(CAS 쓰기). 같은 값이 있으면 쓰지
  않습니다.
- 포트에는 전역·시스템 config 쓰기가 없습니다. adapter는 `git config --local`만 실행합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-ops --lib candidate::install` | 2 passed |
| `cargo test -p stellar-cli` | 전체 통과(merge_driver 4, system_git unit 1 포함) |
| `cargo clippy -p stellar-cli -p stellar-ops --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 동의 없는 명령(driver 실행, semantic-merge) 뒤 저장소 로컬 `merge.*` 없음, `.gitattributes` bytes 동일 | 통과 |
| 신뢰하지 않은 프로젝트의 install은 77이고 config·`.gitattributes` 동일 | 통과 |
| 신뢰 후 install: 로컬 config 두 key, `.gitattributes` 끝 줄 추가, 전역 `~/.gitconfig` bytes 동일 | 통과 |
| 두 번째 install은 바꾼 항목 0, 등록한 driver로 같은 자리 자식 추가가 `git merge`만으로 병합 | 통과 |
| ops 메모리 Git: 기존 `.gitattributes` 줄 보존, 멱등, 프로젝트가 아니면 Git 호출 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 등록 명령을 신뢰한 프로젝트로 제한했습니다(기술 아키텍처 §24.3: 신뢰 전 외부 merge driver 비활성).
- Git 실행은 git-porcelain이 아니라 CLI adapter입니다(ADR-0081 결정 7). ops→git-porcelain 경로가 생기면 바꿉니다.

## 남은 위험 / 한계

- driver 명령에 실행 파일 절대 경로를 씁니다. 실행 파일을 옮기면 install을 다시 실행해야 합니다.
