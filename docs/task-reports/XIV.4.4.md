# 작업 보고서: XIV.4.4 non-Stellar 파일 raw Git status

- WBS: XIV.4.4
- 작업명: non-Stellar files는 raw Git file status로만 표시하고 semantic parser로 해석하지 않음
- 상태: 완료 (`[x]`)
- 선행: XIV.2.1(`[x]`, 이 lane)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git 2.54, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/classify.rs` (신규: `classify_project_path`, `ProjectFileClass`, `StellarFileKind`, `RawFileStatus`, `StatusPartition`, `partition_status`, 테스트)
- `crates/git-porcelain/src/lib.rs` (`classify` 모듈)
- `docs/task-reports/XIV.4.4.md`(신규), 계획서(XIV.4.4 `[ ]`→`[x]`)

## 구현 요약

- 아키텍처 §19 프로젝트 구조에 따라 `stellar.yaml`(Manifest), `stellar.lock.yaml`(Lock), `design/…`
  (Design), `assets/…`(Asset)만 Stellar 파일로 분류합니다. 그 밖(`README.md`, `.gitignore`,
  `.gitattributes`, `.stellar/` 등)은 non-Stellar입니다. 경로는 정확한 바이트로 비교합니다
  (§19.2가 대소문자만 다른 경로를 금지하므로 `Design/`은 Stellar 파일이 아님).
- `partition_status`는 status 항목을 `stellar`(semantic status 입력), `raw`(non-Stellar, raw 표시
  전용 `RawFileStatus`), `cross_boundary`(두 경로 분류가 다른 rename/copy, raw 표시)로 나눕니다.
  non-Stellar 항목은 `StatusEntry`로 semantic 쪽에 전달되지 않습니다.
- `RawFileStatus`는 Git `XY`(`.M`, `R.`, `UU`, `??`, `!!` 등)와 경로·원래 경로를 해석 없이 보존합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | 160 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | exit 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | exit 0 |

| 검사 | 결과 |
|---|---|
| **README change raw status 유지**: 실제 Git에서 `README.md` 수정(`.M`) + `design/current.stellarref` staged → raw 목록에 `README.md` `.M`만, semantic 입력에는 design 파일만(`readme_change_keeps_raw_status`) | 통과 |
| 경로 분류 표 14종(접두어 경계·대소문자·하위 폴더 `stellar.yaml`·`.stellar/` 포함) | 통과 |
| 합성 status: non-Stellar 수정·rename·unmerged·untracked·ignored는 raw 코드 그대로, Stellar 항목만 semantic 입력, 경계 넘는 rename은 `cross_boundary` | 통과 |

## 차이(작업 지시와 다른 점)

- 경계를 넘는 rename/copy(`old.bin` → `design/…`)를 별도 목록으로 두었습니다. 한 항목을 semantic과 raw
  어느 한쪽으로만 보내면 다른 쪽 경로 정보가 사라지기 때문입니다.

## 남은 위험 / 한계

- Stellar 파일 집합은 §19 구조를 코드 상수로 반영했습니다. 구조가 바뀌면 이 분류도 함께 바꿔야 합니다.
- `.gitattributes`·`.gitignore`는 Stellar가 생성하지만 semantic 대상이 아니므로 raw로 둡니다.
