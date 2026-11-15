# 작업 보고서: XIV.5.5 선택 후보 저장 직렬화와 재열기

## WBS

- WBS: XIV.5.5
- 작업명: CandidateIndexSnapshot을 deterministic project files/chunks로 serialize하되 WorkingTree를 수정하지 않는 temp directory writer 구현
- 상태: 완료 (`[x]`) — 기존 generation 형식의 지원 가능한 문서를 실제 index에서 읽고 선택 후보를 저장 파일로 직렬화한 뒤 기존 loader로 재열어 비교합니다. 무손실 증명이 없는 입력은 명시적으로 unsupported입니다.
- 선행: XIV.5.4, VI.10.2, V.5.1 (`[x]`)

## 변경 파일

- `crates/ops/src/candidate.rs`, `crates/ops/src/candidate/tests.rs`, `crates/ops/src/lib.rs`: 저장 파일 → 레코드 → semantic 왕복 검사, candidate → 기존 serializer·SavePlan → chunk envelope·manifest·current bytes.
- `crates/runtime/src/staging.rs`, `crates/runtime/src/staging/tests.rs`, `crates/runtime/src/lib.rs`: 실제 index materialize, 저장된 working generation과 선택 조합, 기존 `write_candidate_tree`, `SavedProjectLoader::for_materialized_snapshot` 재열기 검증.
- `crates/runtime/Cargo.toml`, `Cargo.lock`: runtime → ops·git-porcelain 직접 의존과 테스트 feature. 외부 패키지 추가 없음.
- `tools/xtask/src/crate_graph.rs`, `docs/architecture/crate-dependency-graph.md`, `docs/adr/ADR-0013-app-runtime-crate.md`: 조합 위치와 두 edge의 허용 근거.
- 이 보고서, `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`, `planning/task-status.yaml`: 완료 상태와 생성 원장 증거.

기존 `crates/git-porcelain/src/candidate_tree.rs`의 writer와 semantic-diff는 수정하지 않았습니다. 이전 보고서에서 미구현으로 남겼던 문서 레코드 → storage 파일 연결을 현재 ops 계층에서 완성했습니다.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-runtime staging::tests --no-default-features` (구현 전) | 101 — candidate·runtime API 부재 확인. 최초 fixture의 PropertyPath 생성자 오타도 확인 후 수정 |
| `cargo test -p stellar-ops candidate::tests` | 0 — opaque semantic 속성·stale base 거부 |
| `cargo test -p stellar-ops -p stellar-runtime -p stellar-git-porcelain -p xtask` (중간) | 101 — ops 38개 통과. runtime fixture의 공유 spawn counter 기대값과 잘못 고른 chunk ID 두 건 실패, fixture 수정 후 아래 재검증 |
| `cargo test -p stellar-runtime -p xtask --quiet` | 0 — runtime 87 unit + 22 integration, xtask 187 unit + 16 integration 통과 |
| `cargo test -p stellar-git-porcelain --quiet` | 0 — 314 unit + 3 doc 통과 |
| `cargo test -p stellar-semantic-diff --quiet` | 0 — 78 unit + 4 integration 통과 |
| `cargo clippy -p stellar-ops -p stellar-runtime -p xtask --all-targets -- -D warnings` | 0 — 경고 0 |
| `cargo fmt -p stellar-ops -p stellar-runtime -p xtask -- --check` | 0 |
| `cargo xtask validate-crate-graph` | 0 |
| `cargo xtask validate-adr crates/runtime/Cargo.toml crates/runtime/src/staging.rs docs/adr/ADR-0013-app-runtime-crate.md` | 0 |
| `git diff --check` | 0 |
| `cargo xtask validate-task-report docs/task-reports/XIV.5.5.md` | 0 — 최초 표 header 형식 오류(65)를 수정한 뒤 통과 |
| `cargo xtask generate-ledger` | 0 — 1152 tasks 생성, XIV.5.5 record만 변경 |
| `cargo xtask validate-task-status` | 0 |
| `cargo xtask sync-plan` | 0 |
| `cargo xtask verify-task XIV.5.5` | 0 |
| `cargo xtask validate-commit-message 'Task(XIV.5.5): 선택 후보 저장 직렬화와 재열기 검증 완성'` | 0 |

구현 근거 커밋은 `1af7e0f5c81ad6ef9786ace58b651dedad72d720`입니다. 원장은 위 생성기로 갱신했고 직접 편집하지 않았습니다.

## 환경과 증거

- 필수 실행 환경: `DEV-MAC`.
- 실측 환경: macOS 27.2 (26B123), Darwin 27.2.0 arm64, rustc/cargo 1.96.0, Apple Git 2.54.0.
- 실행 증거: 위 명령과 아래 실제 repository fixture 테스트. CI·LAB·REF 실행 없음.
- 모든 staging 검증은 Cargo가 만든 임시 Git fixture에서 실행했습니다. 실제 작업 저장소의 index나 실사용 프로젝트를 fixture로 쓰지 않았습니다. push·전역/계정 설정 변경·새 checkout·상시 worker 추가 없음.

## 테스트

- `selected_property_round_trips_without_changing_working_bytes_or_real_index`:
  - 실제 저장 파이프라인으로 문서 두 개·generation 두 개를 생성하고 Git에 기록합니다. 동일 node의 이름 A와 잠금 B를 함께 수정하고 A만 선택합니다.
  - 후보를 기존 loader로 다시 열어 전체 문서 레코드가 A만 바꾼 기대값과 같고, B와 다른 문서는 index 상태임을 확인합니다.
  - 작업 트리 전체 파일 hash, `.git/index` 실제 bytes, `ls-files --stage` snapshot이 전후 같습니다.
  - index에 이미 stage된 non-Stellar 파일, lockfile 원시 bytes, `stellar.yaml`의 주석, 이전 generation·object 파일 전체와 current의 실행 mode를 보존합니다.
  - 같은 입력으로 두 번 만든 후보의 경로별 bytes hash가 같습니다. `.git`을 만들지 않고 drop 시 임시 tree를 제거합니다.
  - 새 candidate object 경로와 충돌하는 opaque orphan 파일을 index에 넣으면 덮어쓰지 않고 Unsupported로 거부합니다.
- `unsupported_chunks_and_metadata_preserve_every_byte_and_leave_no_candidate`: optional opaque chunk, pointer schema, root 문서 집합, generation schema, 비정규 DocumentManifest ChunkId의 5종을 Unsupported로 거부합니다. 원본·index 전체 bytes 불변, 남는 임시 tree 0.
- `no_op_uses_index_generation_and_rejects_worktree_temp_parent_and_untrusted_git`: HEAD와 다른 실제 index generation을 기준으로 선택 없음은 새 generation 없이 유지합니다. 작업 트리 내부 임시 경로 거부, 신뢰 전 Git spawn 증가 0.
- `opaque_semantic_field_and_stale_base_are_explicitly_rejected_without_writes`: 역투영에서 사라질 opaque 속성은 Unsupported, 다른 index 기준은 StaleBase. 파일 변경 0.
- 최종 실패 0, clippy 경고 0. 초기 컴파일·fixture 실패는 위 표에 구분했습니다.

## 성능

- 성능 benchmark·REF 예산은 측정하지 않았습니다. 전체 문서의 무손실 증명을 위해 현재 generation의 모든 문서·chunk를 확인합니다. 이 경로를 대형 프로젝트의 성능 완료 근거로 쓰지 않습니다.
- 기존 writer의 경로 index·batch blob 읽기 상한을 재사용합니다. 새 immutable 파일 대조도 경로 BTreeMap을 사용합니다.

## 차이

- 현재 기본 저장 형식은 ADR-0059의 전환 완료 전 generation 형식입니다. 텍스트 원본 export/check·웹·Canvas 구현은 변경하지 않았습니다.
- 새로운 저장 알고리즘을 만들지 않고 `serialize_records`, `build_save_plan`, `ChunkEnvelope`, 기존 경로 함수와 loader를 조합했습니다. storage 파일 포맷은 바꾸지 않았습니다.
- 기존에 없던 통합 위치는 runtime이며, ops와 git-porcelain edge만 ADR·정책·문서에 함께 추가했습니다. ops·git-porcelain·semantic-diff의 기존 경계를 유지합니다.
- 외부 library와 lockfile(XV)은 index bytes 그대로 유지합니다. library resource를 재작성하거나 네트워크에서 resolve하지 않습니다.
- `Cargo.lock`은 Cargo 의존 해석으로 갱신됩니다. 완료 원장은 `cargo xtask generate-ledger`로 생성합니다. 구현 커밋 SHA가 필요하므로 완료 상태·원장 기록은 구현 커밋 뒤 별도 문서 커밋에 둡니다.
- XIV.5.6은 이번에 착수하지 않았습니다. file delta의 삭제·mode·symlink·blob 읽기 상한은 별도 검증 단위로 남깁니다.

## 남은 위험

- 지원 범위는 현재 serializer로 저장 파일 전체를 정확히 재현할 수 있는 기존 문서의 변경입니다. 문서 추가·삭제, ComponentInstance·문서 밖 semantic 데이터, optional chunk·opaque 필드 손실, 지원하지 않는 schema·chunk 배치는 Unsupported입니다. 입력을 축소해 성공시키지 않습니다.
- 후보는 기존 index 파일을 기본 보존합니다. 선택한 semantic 변경만 새 generation에 반영하며, 이전 세대·불필요해진 object의 삭제/GC는 하지 않습니다.
- API는 후보 준비까지만 합니다. file-level delta(XIV.5.6), blob 기록·실제 index 갱신(XIV.6), GUI·FFI 연결은 미구현입니다.
- 준비 전후 index snapshot을 비교해 관찰한 변경은 거부하지만 이후 index 설치의 원자성·동시 writer 충돌 처리는 XIV.6의 책임입니다. 같은 상태로 되돌아오는 ABA나 동일 사용자의 임시 디렉터리 변조 전체를 잠금으로 막는 구현은 아닙니다.
- 전체 workspace·실제 앱 UI·Windows·CI·REF 성능 검증은 실행하지 않았습니다.

## 2026-11-15 재검증

- Codex 작업 검토(계획 B9)에서 이 행의 상태를 다시 정했습니다. 위 기록은 Codex 작업 시간에 작성·통합되어 상태의 근거로 쓰지 않고, 아래 실행 결과를 근거로 씁니다.
- 논리 검토: 계획 U041 수정. storage 공개 API의 ADR 기록은 B3(`ecf2225d`)에서 보완했습니다.
- 환경: `DEV-MAC`(Apple M3 Max, macOS 27.2, rustc 1.96.0, Apple Git 2.54.0). 코드 기준은 `3463761b`이며 이 검토의 문서 커밋은 코드를 바꾸지 않습니다.

| 명령 | 결과 |
|---|---|
| `cargo test --workspace --no-fail-fast` | 0 — 4,039 통과, 0 실패, 10 ignored(수동 측정·장기 fuzz) |

- 검증 기준(write 전후 working tree file hash 동일): `staging::tests::selected_property_round_trips_without_changing_working_bytes_or_real_index` 통과.
- 판정: `[x]` 유지.
