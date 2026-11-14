# 작업 보고서: XIV.4.5 손상된 WorkingTree에서 semantic status 실패와 raw status 유지

- WBS: XIV.4.5
- 작업명: corrupt WorkingTree Stellar state는 semantic status를 실패시키되 raw Git status는 계속 제공하고 자동 수정하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.4.1(`[x]`, 이 lane), V.4.4(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/project_status.rs`(신규) — `SemanticStatusProvider`, `ProjectStatus`, `read_project_status`.
- `crates/git-porcelain/src/lib.rs` — `pub mod project_status;`.
- `crates/semantic-diff/src/status.rs` — `StatusFailure`, `SemanticStatus::try_classify`.
- `docs/task-reports/XIV.4.5.md`(신규), 계획서(XIV.4.5 `[ ]`→`[x]`).

## 구현 요약

- **git-porcelain**: `read_project_status(git, worktree, provider)`는 raw status(`status --porcelain=v2
  -z --branch` → `RepositoryStatus`, non-Stellar raw, 경계 rename, Stellar 파일 상태)를 먼저 읽고 그 다음
  호출자 `SemanticStatusProvider`를 실행합니다. provider 실패는 `ProjectStatus::semantic = Err(..)`로 그대로
  보존하고 raw 필드는 유효합니다. raw status가 실패하거나 untrusted면 provider를 실행하지 않습니다(spawn 0).
  이 함수는 파일을 쓰지 않으며 provider 계약도 저장소·작업 트리 수정을 금지합니다(자동 수정 없음).
- **semantic-diff**: `SemanticStatus::try_classify(head, index, working)`은 세 snapshot 읽기 결과 중
  하나라도 실패하면 부분 결과 없이 실패한 모든 쪽과 원인(`StatusFailure`, Head→Index→WorkingTree 순)을
  돌려줍니다. 빈 snapshot 대체·복구를 하지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-semantic-diff -p stellar-git-porcelain` | git-porcelain 286 passed(+ doc 3), semantic-diff unit 52 + fuzz 4 passed, 0 failed |
| `cargo clippy -p stellar-semantic-diff -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-semantic-diff -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **corrupt chunk fixture(실제 Git)**: 정상 commit 상태에서는 semantic 성공. 작업 트리 `design/objects/aa/one.starchunk`를 손상시키고 README 수정 → `semantic = Err(CorruptChunk(path))`, raw에 `README.md .M`, Stellar 파일 상태에 chunk unstaged Modified, unstaged 수 2, basis `SavedFilesOnDisk`; 호출 전후 작업 트리 bytes·index snapshot·손상 chunk bytes 동일(`corrupt_chunk_fails_semantic_status_but_raw_status_remains`) | 통과 |
| untrusted → `ProjectNotTrusted`, spawn 0, provider 호출 0(`untrusted_project_status_spawns_nothing_and_skips_provider`) | 통과 |
| semantic: 잘린 WorkingTree chunk(`SubtreeRecords::decode` 실패) → `StatusFailure{[WorkingTree]}`, 여러 쪽 실패 모두 보고, 모두 성공이면 `classify`와 동일(`corrupt_snapshot_fails_semantic_status_without_partial_result`) | 통과 |

## 차이(작업 지시와 다른 점)

- 실제 Git fixture의 provider는 테스트용(검사합 형식 fixture chunk)입니다. git-porcelain은 crate 그래프상
  프로젝트 loader·semantic-diff에 의존할 수 없어, 실제 provider(`SavedProjectLoader` + `SnapshotBuilder` +
  `try_classify`)는 trait로 받습니다. 실패 분리·무수정 계약은 provider 구현과 무관하게 이 함수가 보장합니다.

## 남은 위험 / 한계

- 실제 provider 연결 위치는 semantic-diff와 project를 함께 쓰는 crate가 필요합니다(현재 crate 그래프에
  edge 없음, XIV.4.1 보고서 참고).
- provider가 계약을 어기고 파일을 쓰는 경우는 막지 못합니다(trait 문서 계약).

## 후속 수정 (리뷰 반영)

- **종단 검사의 provider는 테스트용(리뷰 minor, 사실 기록)**: 실제 Git corrupt chunk fixture
  (`corrupt_chunk_fails_semantic_status_but_raw_status_remains`)의 `SemanticStatusProvider`는 테스트 구현
  (`ChunkCheckingProvider`)입니다. fixture chunk 형식(`CHUNK` 머리·본문·본문 바이트 합 1바이트)으로 손상을
  판단하며, V.4.4 loader와 semantic-diff `try_classify`를 쓰지 않습니다. fixture chunk도 실제 `.starchunk` 형식이
  아닙니다. XIV.14.6 corpus의 손상 프로젝트 scenario provider(`schema_version: 1` 접두어·`CHUNK:` 머리 검사)도
  같은 성격입니다. 두 반쪽은 각각
  실제 코드로 검사됩니다. semantic-diff `try_classify`는 실제로 잘린 `SubtreeRecords` chunk에서 부분 결과 없이
  실패하고(`corrupt_snapshot_fails_semantic_status_without_partial_result`), git-porcelain
  `read_project_status`는 provider가 실패해도 실제 Git raw status를 유지하며 아무것도 쓰지 않습니다.
- 상태 `[x]` 유지 근거: 검증 열은 "corrupt chunk fixture raw status 가능"이고, 이것은 실제 Git 저장소에서 작업 트리
  chunk를 손상시킨 상태로 확인했습니다. raw status 유지·무수정 계약은 provider 구현과 무관하게
  `read_project_status`가 보장합니다. 실제 provider(`SavedProjectLoader` + `SnapshotBuilder` + `try_classify`)를
  둘 crate가 crate 그래프에 없습니다(git-porcelain은 dev-dependency로도 project·semantic-diff에 의존할 수 없음).
  XIV.5.5는 산출물(serializer) 자체가 없어 `[~]`이고, 이 행은 통합 연결만 없다는 점이 다릅니다.
- 후속 필요: 통합 crate 위치를 ADR로 정한 뒤 실제 저장 파이프라인으로 만든 프로젝트의 chunk를 손상시켜 V.4.4
  loader 실패 → semantic status 실패 → raw status 유지를 한 번에 확인하는 종단 테스트를 추가해야 합니다.
