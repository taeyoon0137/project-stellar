# 작업 보고서: XIII.7.2 import 후 Git LFS 권고 정책

- WBS: XIII.7.2
- 작업명: 에셋 크기·확장자 기반 LFS 권고 정책을 config에서 읽고 import 후 경고만 생성(자동
  `.gitattributes` 수정 없음)
- 상태: 완료 (`[x]`)
- 선행: XIII.5.7, XIII.7.1 (모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/config/lfs-advice.yaml`(신규) — 출하 정책: `size_threshold_bytes`
  (10 MiB, 잠정), `extension: bin`.
- `crates/git-porcelain/src/lfs/advice.rs`(신규) — `LfsAdvicePolicy`, `LfsPolicyConfigError`,
  `ImportedAsset`, `LfsAdvice`(`lfs-recommended`), `LfsAdviceReason`, `LfsAdviceReport`,
  `advise_after_import`, `SHIPPED_CONFIG`.
- `crates/git-porcelain/src/lfs/attr.rs`(신규) — `git check-attr -z --stdin` 읽기
  (`check_attribute`, `parse_check_attr_z`, `AttrValue`, `AttrError`).
- `crates/git-porcelain/src/lfs.rs` — `attr`·`advice` 하위 모듈 등록.
- `crates/git-porcelain/src/ops.rs` — `run_ok_with_input`(stdin 입력 + 성공 요구) 내부 헬퍼.
- `crates/git-porcelain/src/lib.rs` — 모듈 문서 갱신.
- `docs/task-reports/XIII.7.2.md`(신규), 계획서(XIII.7.2 `[ ]`→`[x]`).

## 구현 요약

- **config**: 기존 config와 같은 `key: value` 한 줄 형식(중첩 없음)이며 `extension`만 반복할 수
  있습니다. 필수 key 누락, 단일 key 중복, 알 수 없는 key, `key: value`가 아닌 줄, 0·음수·부호·선행
  0·u64 초과 크기, 형식 위반 확장자(대문자·점 포함·빈 값·16자 초과), 중복 확장자는 모두 명시
  오류입니다. 파일은 64 KiB 상한을 넘으면 읽지 않습니다(`TooLarge`).
- **판정**(`LfsAdvicePolicy::evaluate`, 순수 함수): `size >= size_threshold_bytes`이면
  `SizeAtLeast`, 저장 경로 마지막 component 확장자(ASCII 대소문자 무시, 점으로 시작하는 이름은
  확장자 없음)가 목록에 있으면 `Extension`입니다. 이유는 크기 → 확장자 순입니다.
- **import 후 경고**(`advise_after_import`): 정책에 걸린 경로만 `git check-attr filter`로 확인해
  이미 `filter=lfs`인 경로는 `already_tracked`로 분리하고 나머지만 `LfsAdvice` 경고로 돌려줍니다.
  `.gitattributes` 문법은 직접 해석하지 않습니다. 정책에 걸린 경로가 없으면 Git을 실행하지
  않습니다. 결과는 경고뿐이며 `.gitattributes`·index·작업 트리를 바꾸지 않습니다.
- **check-attr 해석**: 응답 경로·속성 이름이 요청 순서와 다르거나 레코드 수가 맞지 않으면 해석
  오류입니다. 경로는 Git 실행 전에 snapshot 경로 규칙(빈 경로·절대 경로·`..`·`.git`·NUL 거부)으로
  검증합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 243 passed + doctest 2 passed, 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **threshold boundary fixture**: 임계값 1000에서 0·1·998·999 권고 없음, 1000·1001·2000·`u64::MAX` 권고. 출하 임계값 t−1 없음, t·t+1 권고. 임계값 1에서 0 없음·1 권고(`threshold_boundary_fixture`) | 통과 |
| 확장자 규칙: 대소문자 무시, `.bin`·`x.bin.png`·`dir.bin/file`·`x.` 제외, non-UTF8 이름 panic 없음, 크기+확장자 이유 순서, 코드 `lfs-recommended` | 통과 |
| 출하 config 해석(문자열·파일 동일), config 오류 15종 명시 오류, 없는 파일 `Io`, 64 KiB 초과 `TooLarge` | 통과 |
| 정책에 걸린 경로가 없으면 spawn 0(`no_candidates_spawns_no_git`) | 통과 |
| 실제 Git: `*.psd filter=lfs` 저장소에서 큰 PNG·`.bin`만 경고, 추적 중 `.psd`는 `already_tracked`, `.gitattributes` 바이트·`git status` 출력 불변 | 통과 |
| 실제 Git: `.gitattributes`가 없는 저장소에서 경고 생성 후에도 `.gitattributes` 미생성 | 통과 |
| check-attr 출력 해석(값 4종, 잘림·경로 불일치·속성 불일치·레코드 과다 거부), 잘못된 경로 spawn 0 | 통과 |

## 차이(작업 지시와 다른 점)

- 출하 config를 `config/`가 아니라 `crates/git-porcelain/config/lfs-advice.yaml`에 두었습니다. 이
  작업 lane의 수정 허용 경로가 `crates/git-porcelain/**`뿐이기 때문입니다. 형식은 기존
  `config/*.yaml`과 같으므로, 저장소 `config/`로 옮길 때는 파일 이동과 `SHIPPED_CONFIG`의
  `include_str!` 경로만 바꾸면 됩니다.
- 판정에 쓰는 확장자는 원본 파일 확장자가 아니라 저장 경로(`assets/<hash>.<ext>`)의 확장자입니다.
  `.gitattributes` 패턴이 저장소 안 경로에 적용되기 때문입니다.

## 남은 위험 / 한계

- 10 MiB 임계값과 `bin` 확장자 목록은 잠정값입니다. 저장소 벤치마크(아키텍처 29.2·42절 8번) 후
  확정해야 합니다.
- import 흐름(assets·project crate)에서 `advise_after_import`를 호출해 경고를 UI로 보내는 연결은
  상위 계층 작업입니다. git-porcelain은 assets crate에 의존할 수 없어(crate graph) 저장 경로와
  크기를 `ImportedAsset`으로 받습니다.
- 경로는 저장소 worktree root 기준입니다. Stellar 프로젝트가 저장소 하위 디렉터리에 있는 경우의
  경로 접두어 변환은 호출자가 맞춰야 합니다.
