# 작업 보고서: V.4.4 프로젝트 열기 loader

- WBS: V.4.4
- 작업명: current pointer→manifest→chunks 순으로 검증하는 loader
- 상태: 완료 (`[x]`)
- 선행: V.4.3, V.2.6

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/storage/src/generation/loader.rs`(신규) — `load_current_generation`,
  `write_manifest_file`, `read_manifest_file`, `manifest_path`, `LoadError`,
  `LoadedGeneration`.
- `crates/storage/src/generation/mod.rs` — `loader` 모듈 등록.
- `docs/task-reports/V.4.4.md`(신규), 계획서(V.4.4 `[ ]`→`[x]`).

## 구현 요약

- **manifest 파일 저장**: `design/generations/<algo>-<hex64>.starmanifest`
  (content-addressed, `ContentHash` 타입 입력만 허용 → traversal 불가). 기록은
  atomic write + 불변성 규칙(V.2.4와 동일: 동일 bytes 재사용, 불일치는
  `CorruptExistingManifest`).
- **loader 검증 순서**: current pointer 읽기(V.4.3) → manifest bytes hash가 주소와
  일치 확인 + 디코딩·검증(V.4.1) → project id 일치 확인 → 모든 chunk pointer 대상
  파일 존재 확인. chunk **내용** 검증은 읽기 경로(V.2.6)가 수행합니다 — 지연
  로딩(V.6.6)과 호환되도록 열기 시점에 전체 payload를 읽지 않습니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-storage` | 115 passed + doctest 2 passed, 0 failed |
| `cargo fmt -p stellar-storage -- --check` | 통과 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 경고 0 |

| 검사 | 결과 |
|---|---|
| 누락 manifest fixture 실패(`missing_manifest_fails`) | 통과 |
| 누락 chunk fixture 실패(`missing_chunk_fails`) | 통과 |
| 잘못된 project id fixture 실패(`wrong_project_id_fails`) | 통과 |
| 정상 프로젝트 로드, manifest 손상·current 누락·불변성 위반 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- chunk 검증은 존재 확인까지만 수행합니다(내용 검증은 V.2.6 읽기 경로 소관).
  열기 시 전체 chunk를 읽으면 V.6.6 지연 로딩 계약과 충돌합니다.

## 남은 위험 / 한계

- current 손상 시 정상 세대 fallback(V.4.5)과 읽기 전용 recovery(V.4.6)는 후속
  작업입니다.
