# 작업 보고서: XIII.7.3 사용자 승인 후 `git lfs track` 적용

- WBS: XIII.7.3
- 작업명: 사용자 승인 API가 호출된 경우에만 `git lfs track`으로 `.gitattributes`를 수정(파일 문법
  재구현 없음)
- 상태: 완료 (`[x]`)
- 선행: XIII.7.2 (이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust + 로컬 시스템 git·git-lfs 3.8.1, 로컬 검증; 실제 Git fixture는 `cfg(unix)`).

## 변경 파일 목록

- `crates/git-porcelain/src/lfs/track.rs`(신규) — `LfsTrackTarget`, `LfsTrackRequest`
  (`preview`·`effects`), `LfsTrackApproval`, `LfsTrackOutcome`, `LfsTrackError`, `apply_lfs_track`.
- `crates/git-porcelain/src/lfs.rs` — `track` 모듈 등록.
- `crates/git-porcelain/src/lfs/advice.rs` — 확장자 형식 검사(`valid_extension`)를 crate 내부 공유.
- `docs/task-reports/XIII.7.3.md`(신규), 계획서(XIII.7.3 `[ ]`→`[x]`).

## 구현 요약

- **요청**: `LfsTrackTarget::Extension(ext)` → `git lfs track -- *.<ext>`,
  `LfsTrackTarget::File(path)` → `git lfs track --filename -- <path>`(glob 문자를 git-lfs가 literal로
  escape). 확장자는 소문자 ASCII 영숫자, 파일 경로는 snapshot 경로 규칙 + UTF-8 + 제어 문자 없음 +
  `#`·`!`·`"`·`-`로 시작하지 않음을 요구합니다. 적용 후 확인할 검증 경로(확장자 대상은 그 확장자의
  파일, 파일 대상은 그 파일)를 함께 받습니다.
- **승인**: `LfsTrackApproval::approve(&request)`만 증표를 만들며(필드 비공개, crate 밖 조립 불가 —
  `compile_fail` doctest), 증표는 argv·검증 경로 digest에 묶입니다. `apply_lfs_track`은 증표를 타입으로
  요구하고, 다른 요청의 증표면 프로세스를 만들지 않고 `ApprovalMismatch`입니다. 신뢰되지 않은
  프로젝트면 `Gate(ProjectNotTrusted)`로 spawn 0입니다.
- **실행 순서**: 승인 대조 → 중립 디렉터리 `git lfs version` probe(XIII.7.1; 미설치면
  `LfsNotInstalled`, 저장소 안 `git lfs` 실행 0 — 저장소 `alias.lfs` 실행 방지) → `check-attr filter`
  (이전 값) → `git lfs track` → `check-attr filter`로 `filter=lfs` 검증(아니면 `NotApplied`).
- **부수 효과 공개**: `effects()`는 `.gitattributes` 추가, git-lfs의 LFS hook 설치 가능성, 이력 불변을
  알립니다. git-lfs 3.8.1은 `track` 실행 시 저장소에 `pre-push` 등 hook을 설치합니다(LFS object 업로드에
  필요한 표준 동작이므로 막지 않음).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain` | lib 250 passed + doctest 3 passed(compile_fail 2), 0 failed |
| `cargo clippy -p stellar-git-porcelain --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-git-porcelain -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| **승인 없음 변경 0**(실제 Git+LFS): 다른 요청의 승인으로 호출 → `ApprovalMismatch`, spawn 증가 0, `.gitattributes` 미생성, `.git/hooks` 목록·`git status` 출력 불변, check-attr 결과 미추적 | 통과 |
| **승인 후 Git 결과 검증**: 승인 후 `git lfs track -- *.bin` 적용, check-attr로 `assets/a.bin`·`x/y/z.bin` = `filter=lfs`, `.png`·`.psd` 미추적. 재적용은 `was_tracked = true`로 성공 | 통과 |
| 파일 대상 literal: `assets/project [1].psd`만 추적, glob으로 해석될 `assets/project 1.psd`는 미추적 | 통과 |
| LFS 미설치(PATH 비움, 실제 Git): `LfsNotInstalled`, spawn 1(probe만), 저장소 `alias.lfs` marker 미생성, `.gitattributes` 미생성 | 통과 |
| executable 없는 실행기에서 승인 불일치 spawn 0, 일치 시 probe 1회 시도 / untrusted spawn 0 | 통과 |
| 잘못된 대상 14종 거부(대문자·glob 확장자, 검증 경로 불일치, 개행·`#`·`!`·`-` 시작, `.git`, 절대 경로, non-UTF8) | 통과 |
| `LfsTrackApproval { digest: 0 }` 외부 조립 compile_fail | 통과 |
| 실제 Git 3개 테스트가 건너뛰지 않고 실행됨(건너뜀 분기를 일시적으로 panic으로 바꿔 확인 후 원복) | 확인 |

## 차이(작업 지시와 다른 점)

- 없음. (`.gitattributes` 작성은 git-lfs가 하며, git-lfs가 함께 설치하는 LFS hook은 승인 화면 설명
  `effects()`로 공개합니다.)

## 남은 위험 / 한계

- 확장자 대상의 `*.<ext>` 패턴은 대소문자를 구분합니다(Git attribute 매칭과 같은 기준으로 검증 경로도
  대소문자 구분). Stellar asset 저장 확장자는 소문자이므로 문제 없지만, 대문자 확장자 파일은 파일
  대상으로 추적해야 합니다.
- 승인 UI(미리보기·부수 효과 표시)와 import 경고 → 승인 흐름 연결은 상위 계층 작업입니다.
- Windows(Git for Windows 번들 LFS) 환경은 실행하지 않았습니다(ADR-0006).
