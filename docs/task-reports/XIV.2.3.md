# 작업 보고서: XIV.2.3 `SnapshotSource`

- WBS: XIV.2.3
- 작업명: `SnapshotSource`를 Head(commit), Index, WorkingTree, Commit(hash), Tree(hash)로 정의
- 상태: 완료 (`[x]`)
- 선행: XIV.2.2(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/git-porcelain/src/snapshot/mod.rs` (신규: `SnapshotSource`, `SnapshotSourceKind`, `SourceSpecError`, spec 왕복, 테스트)
- `crates/git-porcelain/src/lib.rs` (`snapshot` 모듈, `pub use snapshot::SnapshotSource`; 골격용 빈 `SnapshotSource` trait 제거)
- `docs/task-reports/XIV.2.3.md`(신규), 계획서(XIV.2.3 `[ ]`→`[x]`)

## 구현 요약

- `SnapshotSource`: `Head`(읽는 시점의 HEAD commit, unborn이면 빈 snapshot), `Index`(stage 0),
  `WorkingTree`(디스크에 저장된 파일), `Commit(ObjectId)`, `Tree(ObjectId)`. object ID는 검증된
  `ObjectId`(소문자 16진수 40·64자)만 허용합니다.
- `kind()`(GitTree/Index/WorkingTree)와 `is_immutable()`(지정 commit·tree만 불변)을 제공합니다.
- 텍스트 spec `head`·`index`·`worktree`·`commit:<oid>`·`tree:<oid>`로 왕복 변환합니다
  (`to_spec`/`parse_spec`, `Display`/`FromStr`). 공백·대문자 이름·대문자 hex·길이 오류는 거부합니다.
- II.2.6 골격에서 남겨 둔 빈 `SnapshotSource` trait는 같은 이름의 실제 타입으로 대체했습니다(다른
  crate 사용처 없음 확인).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 135 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **type round-trip**: 5개 variant(SHA-1·SHA-256 commit/tree 포함 7개 값) spec→parse·`FromStr`·`Display` 왕복(`every_source_round_trips_through_spec`) | 통과 |
| 종류·불변성 표(`kinds_and_immutability`) | 통과 |
| 잘못된 spec 12종 거부(이름·공백·대문자 hex·길이)(`invalid_specs_are_rejected`) | 통과 |

## 차이(작업 지시와 다른 점)

- `Head`는 값을 담지 않고 "읽는 시점의 HEAD commit"으로 정의했습니다. 특정 commit을 고정하려면
  `Commit(hash)`를 씁니다(HEAD 해석은 XIV.2.4 provider가 수행).

## 남은 위험 / 한계

- spec은 Stellar 내부 왕복용 텍스트입니다. 저장 포맷에 쓰려면 별도 schema 결정이 필요합니다.
