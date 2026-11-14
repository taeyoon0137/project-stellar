# 작업 보고서: XIV.5.6 후보와 index의 파일 delta

## WBS

- WBS: XIV.5.6
- 작업명: candidate와 기존 index tree의 file-level diff로 변경 path·mode·blob bytes만 산출
- 상태: `[x]` — 파일 delta와 기존 runtime 연결 검증을 완료했습니다.
- 선행: XIV.5.5, XIV.2.5 (`[x]`)

## 변경 파일

- `crates/git-porcelain/src/candidate_tree.rs`: 후보 생성 시 읽은 전체 `IndexSnapshot`을 보관합니다.
- `crates/git-porcelain/src/candidate_delta.rs`, `crates/git-porcelain/src/candidate_delta/tests.rs`, `crates/git-porcelain/src/lib.rs`: 변경된 파일만 반환하는 API와 실제 Git fixture 검증.
- `crates/runtime/src/staging/tests.rs`: 기존 저장 프로젝트 A/B 선택 fixture에 정확히 3개 delta와 동일 후보 0 검증 추가.
- 이 보고서와 후속 계획·생성 원장. 저장 포맷·외부 의존성·runner·ops 구현은 변경하지 않습니다.

## 명령과 종료 코드

| 명령 | 결과 |
|---|---|
| `cargo check -p stellar-git-porcelain` | 0 |
| `cargo test -p stellar-git-porcelain candidate_delta::tests -- --nocapture` (최초) | 101 — 비UTF8 파일 이름을 거부하는 macOS fixture, 같은 OID dedup으로 달라진 batch 기대값의 2건 실패 |
| 같은 명령 (fixture 수정 후) | 0 — 9개 통과 |
| `cargo test -p stellar-git-porcelain --quiet` | 0 — 323 unit + 3 doc 통과 |
| `cargo test -p stellar-runtime staging::tests --no-default-features` | 0 — 3개 통과 |
| `cargo clippy -p stellar-git-porcelain -p stellar-runtime --all-targets -- -D warnings` | 0 — 경고 0 |
| `cargo fmt -p stellar-runtime -- --check` (최초) | 1 — 새 assertion 줄바꿈, fmt 적용 후 아래 재검증 |
| `cargo fmt -p stellar-git-porcelain -p stellar-runtime -- --check` | 0 |
| `cargo xtask check-core` | 0 — workspace fmt·clippy·test·doc-test 모두 통과 (`target/xtask-logs/`) |
| `cargo xtask validate-task-report docs/task-reports/XIV.5.6.md` | 0 |
| `cargo xtask validate-crate-graph` | 0 |
| `cargo xtask validate-adr crates/git-porcelain/src/candidate_delta.rs crates/git-porcelain/src/candidate_tree.rs crates/runtime/src/staging/tests.rs` | 0 |
| `cargo xtask generate-ledger` | 0 — Task 커밋 `68be98af` 이후 실행, 해당 작업만 갱신 |
| `cargo xtask validate-task-status` / `cargo xtask sync-plan` / `cargo xtask verify-task XIV.5.6` | 모두 0 |
| `git diff --check` | 0 |

## 환경과 증거

- `DEV-MAC`: macOS 27.2 (26B123), Apple silicon, Rust/Cargo 1.96.0, Apple Git 2.54.0.
- 실제 Git 검증은 기존 `GitFixture`가 만든 임시 프로젝트에서만 실행했습니다. 테스트가 Git을 발견하지 못하면 unit fixture 일부는 건너뛰지만, 이번 환경은 Git 버전을 확인했고 runtime fixture는 Git 필수 조건으로 실행했습니다.
- 전체 check-core 집계: test 단계 3842 passed·8 ignored·0 failed(이 단계 doctest 포함), 별도 doc-test 54 passed·0 failed. 독립 코드 리뷰에서 새 확정 결함 없음.
- CI·LAB·REF·실제 앱 QA 실행 없음. 이 결과는 플랫폼 동등성·릴리스 gate 통과 주장이 아닙니다.

## 테스트

- bytes가 같아도 executable mode가 다르면 Upsert, symlink는 링크 대상 bytes로 비교, 삭제는 Remove, 신규 파일은 원시 binary bytes를 반환합니다. 경로 정렬은 GitPath bytes 기준입니다.
- 출처가 Bytes인 동일 파일도 delta 0이며, 빈 index·빈 후보·탭/개행/한글 경로를 검사했습니다. 같은 입력의 반복 결과가 같습니다.
- 별도 staged README의 bytes는 유지하고 WorkingTree 수정분은 제외합니다. gitlink는 blob으로 읽거나 삭제하지 않으며, gitlink 자신·하위 경로와 충돌하는 후보는 명시적으로 거부합니다.
- 누락 파일, 일반 파일을 symlink로 교체, 상위 symlink, mode 변조, non-Stellar bytes 변조, 읽기 상한 초과는 부분 delta 없이 실패합니다. stale index는 후보 파일 읽기 전에 진단하고 untrusted Git spawn 증가량은 0입니다.
- 후보와 index의 단일/합계 bytes 상한 및 batch 분할을 실제 명령 증가량으로 확인했습니다. 계측은 fixture 실행기별 before/after 차이를 사용합니다.
- 기존 runtime 정상 저장 프로젝트에서 속성 A만 담긴 후보의 delta는 chunk·generation·current 3개입니다. B·다른 문서·기존 staged 파일은 후보에서 보존되고 원본 WorkingTree·index bytes는 불변입니다. index generation 기준 no-op은 delta 0입니다.

## 성능

- 경로 대조는 BTreeMap/BTreeSet, index 읽기는 기존 OrderedBlobReader의 batch 상한을 재사용합니다.
- 기본 한도: 단일 파일 256 MiB, batch 64 MiB, 비교 index bytes와 candidate bytes 각각 512 MiB. 결과에는 바뀐 bytes만 남습니다. 대형 프로젝트 benchmark·REF 예산은 측정하지 않았습니다.

## 차이

- gitlink는 변경 대상이 아니라 보존 대상으로 처리합니다. 변경·대체하려는 후보는 진단합니다.
- 구현 후 테스트를 추가했으므로 계획 §1.2의 테스트 선행 red 증거는 없습니다. 최초 실패는 fixture 오류이며 요구사항 실패 증거로 대체하지 않습니다.
- 생성 원장은 해당 Task 커밋이 존재한 뒤에만 `cargo xtask generate-ledger`로 갱신합니다.

## 남은 위험

- delta 반환 뒤 실제 index 적용의 원자적 비교·경합·실패 복원은 XIV.6의 책임입니다. 이 API는 읽기 전용이며 실제 stage를 완료했다는 의미가 아닙니다.
- index 전후 snapshot 비교는 path/mode/OID/stage 기준입니다. ABA나 index flags/확장 metadata 변경까지 잠그거나 판별하지 않습니다.
- 동일 사용자의 candidate 디렉터리 동시 변조를 잠금으로 배제하지 않습니다. 파일 읽기는 기존 안전 helper와 상한을 사용합니다.
- 저장 의미 지원 범위는 XIV.5.5와 같습니다. 무손실이 입증되지 않은 문서·chunk·opaque 필드는 계속 Unsupported입니다.
