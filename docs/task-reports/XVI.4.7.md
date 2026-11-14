# 작업 보고서: XVI.4.7 단일 노드 삽입 인덱스

## WBS

- WBS: XVI.4.7
- 상태: 부분 구현 (`[~]`). 단일 Frame 삽입의 영속 ID·형제 OrderKey 검사를 증분 인덱스로 연결합니다. RestoreNodeRecords와 다른 command의 전체 인덱스 재구성은 남습니다.

## 변경 파일

- `crates/document/src/model.rs`: 정의·Path element·modifier ID 소유 수와 부모별 `(OrderKey, StableId)`를 유지합니다. 삽입·삭제·payload·stack 교체에서 갱신하고 저장 레코드에서 재구성합니다.
- `crates/document/src/path.rs`: point·handle ID 순회를 공유합니다.
- `crates/commands/src/node_ops.rs`: InsertNode의 중복 ID·형제 키 검사를 모델 조회로 연결합니다. 외부 참조 종류가 필요할 때만 기존 ID 분류를 만듭니다. 회귀 검사와 재사용 가능한 release 측정을 포함합니다.
- `crates/commands/src/tree.rs`: Path ID 순회를 공유합니다.
- `docs/task-reports/VII.2.1.md`: 기존 lane의 측정·지원 범위를 보존합니다.
- `docs/PROJECT_STELLAR_IMPLEMENTATION_PLAN.md`, 생성 원장 `planning/task-status.yaml`: 전체 완료 조건을 보존하고 부분 구현을 명시합니다.

## 명령과 종료 코드

아래 전체 Rust·release·최초 Swift 검증은 로컬 `dc5fda72`(47cba3e6 + ff14c4db 및 회귀 보강) 기준입니다.

| 명령 | 결과 |
|---|---|
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true RUST_TEST_THREADS=1 cargo test -p stellar-commands -p stellar-document --locked` | 0 — 최종 보강 포함 475 passed, 0 failed, 기존 2개와 추가 성능 측정 1개 ignored |
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true cargo test --release -p stellar-commands measure_insert_single_frame --locked -- --ignored --nocapture` (3회) | 각 0 — 1,000/10,000/50,000 leaf의 실제 command execute 측정 |
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true RUST_TEST_THREADS=1 cargo xtask check-core` | 0 — fmt/clippy 경고 0, workspace 3,924 passed/11 ignored, 별도 doctest 54 passed |
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true RUST_TEST_THREADS=1 ./scripts/macos/smoke.sh` | 0 — smoke OK; window-session 68, session-commands 82, window-close 98(`native=false`), key-mapping 101 |
| `target/debug/xtask sync-plan --check` / `validate-task-status` / `validate-adr` / `validate-crate-graph` / 두 작업 보고서 검사 / `git diff --check` | 각 0 |

## 환경과 증거

DEV-MAC, Apple M3 Max, Mac15,11, 메모리 36 GiB, macOS 27.2 (26B123), rustc 1.96.0. 전용 브랜치에서 fetch한 `origin/main=47cba3e628f0c1dd1264e64da2fe0834b3b1045a`에 원본 lane `ff14c4db6ebe837b73e456efcd1cec437a2cbf17`을 이력 보존 병합했습니다. 원본 lane은 clean 확인 후 읽기만 했습니다. 원시 로그는 저장소 밖 `edit-perf-integration-20261114-1955/`에 보관합니다.

종료 전 fetch에서 원격이 `f7d343cb243702c6cd41d88f5635313300ab4ea3`으로 진전한 것을 확인하여 양쪽 이력을 보존한 추가 병합을 수행했습니다. 충돌은 없었습니다. 원격에도 같은 `ff14c4db`가 포함되며, 원격 대비 차이는 `node_ops.rs`의 회귀 보강과 XVI.4.7 계획·원장·보고서 4개 파일입니다. 삽입 제품 코드와 그 document 의존 코드는 동일합니다. 원격에서 추가된 창 신뢰·실제 Git 이름 Stage 경로는 관련 Rust 4개 crate와 새 Swift smoke로 재검증했고, 최종 병합에서 전체 workspace Rust를 다시 실행한 것으로 보고하지 않습니다. 재검증 로그는 같은 외부 폴더의 `final-f7d343cb/`에 보존합니다. 병합 직후 예전 `target/debug/xtask`로 실행한 FFI header 검사는 exit 65였으나, `cargo xtask generate-ffi-header --check`가 변경된 도구 소스를 재빌드한 뒤 exit 0으로 통과했습니다. header/template 소스를 수정하거나 검사를 완화하지 않았습니다.

최신 병합의 검증:

| 명령 | 결과 |
|---|---|
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true RUST_TEST_THREADS=1 cargo test -p stellar-commands -p stellar-document -p stellar-runtime -p stellar-ffi --locked` | 0 — 674 passed/3 ignored, 실제 Stage runtime·FFI 및 삽입 회귀 포함 |
| `CARGO_BUILD_JOBS=4 CARGO_NET_OFFLINE=true RUST_TEST_THREADS=1 ./scripts/macos/smoke.sh` (f7d343cb 병합 후) | 0 — 신뢰 승인·철회·재열기, 실제 이름만 Stage/시스템 Git index 재조회, 잠금의 worktree 보존, 취소·stale·미신뢰 보호, 기존 셸 검사 통과 |
| `cargo fmt --all -- --check` / `cargo xtask generate-ffi-header --check` / `sync-plan --check` / `validate-task-status` / `validate-adr` / `validate-crate-graph` / 보고서 검사 / `git diff --check` | 각 0 |

## 테스트

- 무작위 command 순서의 적용 직후와 inverse 직후마다 전체 ID 분류·ChildIndex 재구성과 대조합니다. 제거된 ID·key도 기억하여 잔존 인덱스를 검사합니다.
- 직접 모델 API로 동일 순서 키·Path ID·modifier ID가 겹친 상태를 만든 뒤 node 삭제, payload 교체, stack 비우기를 수행하여 마지막 소유자까지 조회가 유지되는지 확인합니다.
- Frame·Path subtree와 modifier 두 개를 삽입하는 각 primitive 앞에서 실패를 주입합니다. rollback 후 레코드와 새 ID·순서 키 부재를 확인하고 같은 요청 재시도, 저장 레코드 재열기로 인덱스 재구성을 확인합니다.

임시 HOME·프로젝트의 AppKit host 검사와 시스템 Git `checkout-index`로 실제 index를 별도 재열어 검증했습니다. 일반 Git 패널 stub 검사와 구분하며, 사용자 물리 입력을 직접 수행한 증거는 아닙니다.

## 성능

재현 명령은 `cargo test --release -p stellar-commands measure_insert_single_frame --locked -- --ignored --nocapture`입니다. 루트 1개·그룹 100개에 leaf 1,000/10,000/50,000개를 만든 뒤 서로 다른 ID·키로 Frame 101개를 추가합니다. 요청 생성 이후 `DocumentState::execute`의 검증·역명령·모델 적용까지만 측정하며 디스크 저장·dirty 판정·runtime·화면 표시를 포함하지 않습니다. 기존 lane의 전후 수치는 VII.2.1 보고서에 보존합니다. 이번 통합 코드에서 3회 실행한 각 percentile·setup의 중앙값은 다음과 같습니다. 기존 기준 바이너리를 이 회차에 재실행한 전후 비교는 아닙니다.

| 기존 leaf 수 | p50 | p95 | p99 | 모델·인덱스 setup |
|---|---:|---:|---:|---:|
| 1,000 | 0.917 µs | 2.334 µs | 5.042 µs | 1.279 ms |
| 10,000 | 1.625 µs | 2.083 µs | 3.375 µs | 5.636 ms |
| 50,000 | 2.250 µs | 5.708 µs | 6.916 µs | 46.182 ms |

5만 leaf의 3회 p95 범위는 2.167–6.500 µs, setup은 24.295–46.367 ms입니다. 동시 작업이 있는 개발 기기의 관측값이며 하드웨어를 격리한 lab 결과로 취급하지 않습니다. 원시 결과는 외부 `release-1.log`~`release-3.log`와 `release-summary.json`에 보존합니다.

## 차이

기존 검증 가능한 lane의 단일 삽입 개선을 먼저 통합했습니다. XVI.4.7의 RestoreNodeRecords 50,000 node ≤1ms 및 모든 command 전환 완료 조건은 바꾸지 않습니다. 명령 오류 규칙, 기본 저장, 의존성, 원격 서비스를 변경하지 않습니다.

## 남은 위험

- RestoreNodeRecords, 이름 편집 시 dirty 전체 재직렬화, 참조·modifier를 포함하는 삽입의 전체 조회는 이번 범위에서 최적화하지 않았습니다.
- 새 인덱스의 메모리 사용량·전체 프로젝트 열기 비용은 측정하지 않았습니다. 초기 모델 구성 비용은 별도 setup 수치입니다.
- DEV-MAC 단일 command 측정이며 REF-MAC/Windows와 T0→T5 성능 성공을 뜻하지 않습니다. Swift smoke는 셸 기능 회귀 검사입니다. 5만 노드 삽입을 실제 앱 창에서 조작한 측정·물리 입력·Metal Present는 수행하지 않았습니다. inspector miss build 785 slices 중 345개가 2ms를 넘은 관측 등은 별도 UI 성능 한계로 남습니다.
- 기존 병렬 IPC 시간 상한 실패 이력은 남으며 이번 통합은 직렬 전체 Rust 검증으로 확인합니다.
