# 작업 보고서: XV.3.2 repository별 bare mirror와 fetch dedup

- WBS: XV.3.2
- 작업명: repository URL별 bare mirror를 만들고 fetch를 GitRunner로 수행
- 상태: 완료 (`[x]`)
- 선행: XIV.1.3(`[x]`), XV.3.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 Git, 로컬 경로 원격 — 외부 네트워크 미사용).

## 변경 파일 목록

- `crates/dependencies/src/mirror.rs`(신규) — `RepositoryKey`, `RepositoryKeyer` trait, `MirrorError`,
  `MirrorCache`(`ensure_fetched`, `mirror_dir`, `FetchedRefs` 구현).
- `crates/dependencies/src/test_support.rs` — 테스트용 `FnvKeyer`.
- `crates/dependencies/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XV.3.2.md`(신규), 계획서(XV.3.2 `[ ]`→`[x]`).

## 구현 요약

- mirror 경로는 `<cache root>/mirrors/<repository-key>.git`입니다. key는 `RepositoryKeyer`가 만든
  소문자 hex(16~64자)라 디렉터리 이름에 URL이 드러나지 않고, 같은 비교 키(`.git`·끝 `/` 차이)는 같은
  mirror를 씁니다.
- `ensure_fetched`: mirror가 없으면 `git init --bare --quiet --template= <dir>`(로컬, template 비움 →
  hook 미복사), 이어서 `git --git-dir <dir> fetch --quiet --prune -- <url> +refs/heads/*:refs/heads/*
  +refs/tags/*:refs/tags/*`. 성공한 repository key를 세션 집합에 기록해 **세션 안에서 같은 저장소는
  다시 fetch하지 않습니다.** 실패는 기록하지 않아 재시도 가능하고, 오류에는 종료 코드만 담습니다.
- `MirrorCache`는 XV.2.4의 `FetchedRefs`를 구현하므로 `GitSelectorResolver`가 commit 검증 전에
  mirror를 확보합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-dependencies` | 39 passed, 0 failed; doctest 1 passed |
| `cargo clippy -p stellar-dependencies --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-dependencies -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 같은 저장소의 두 의존성(v1·v2, 한쪽은 끝 `/` 표기)을 실제 Git으로 resolve → node 2개, **fetch 1회**, ls-remote 1회, init 1회, mirror 1개(`two_dependencies_on_one_repository_fetch_once`) | 통과 |
| mirror가 cache root `mirrors/` 아래, 이름에 URL 텍스트 없음, 표기가 달라도 같은 mirror | 통과 |
| 새 세션은 기존 mirror 재사용(init 0) + 세션당 fetch 1회 | 통과 |
| 없는 저장소 fetch 실패 → `FetchFailed`(비영 종료), 메시지에 경로 없음, fetched 미기록·재시도 시 다시 fetch, `FetchedRefs` 경로는 `GitFailed{fetch}`(`failed_fetch_is_not_marked_fetched`) | 통과 |
| `RepositoryKey` 형식(길이·소문자 hex·경로 문자 거부)(`repository_key_format`) | 통과 |

## 차이(작업 지시와 다른 점)

- "GitRunner로 수행"은 이 crate의 `GitCommandRunner` trait을 통합니다(의존 방향상 git-porcelain
  직접 사용 불가). 운영 adapter(`GitRunner`)와 운영 keyer(로컬 비밀 key 기반 keyed hash, storage
  SHA-256)는 project 연결 작업입니다. 테스트 keyer는 비암호학적 FNV-1a입니다.

## 남은 위험 / 한계

- 여러 프로세스가 같은 mirror를 동시에 fetch할 때의 잠금은 XV.3.6 범위입니다.
- 오래된 mirror 정리(GC)는 XV.8 범위입니다.
