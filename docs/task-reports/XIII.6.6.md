# 작업 보고서: XIII.6.6 폰트 라이선스·임베딩 상태

- WBS: XIII.6.6
- 작업명: 라이선스·임베딩 상태를 Unknown, Allowed, Restricted, UserConfirmed로 저장하고 Stellar가 법적 판정을 자동 확정하지 않게 함
- 상태: 완료 (`[x]`)
- 선행: XIII.6.1(이 lane에서 완료)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/project/src/fonts/license.rs` — `FontLicenseState`(안정 tag), `LicenseChangeOrigin`,
  `LicenseTransitionError`, `FontLicenseState::transition`, `FontLicenseInfo::state`·`change_state`.
- `crates/project/src/fonts/project_font.rs` — 라이선스 nested record field 5(상태), decode 검사
  (`UnknownLicenseState`, 명시적 `Unknown` 거부), `ProjectFontRecord::license_state`·
  `change_license_state`, `ImportedProjectFont::embedding`.
- `crates/project/src/fonts/sfnt.rs` — OS/2 `fsType` 읽기(`EmbeddingPermissions`, `EmbeddingLevel`,
  `FontFaceInfo::embedding`).
- `crates/project/src/fonts/test_fixtures.rs` — `sfnt_face_with_tables`, `os2_table`.
- `crates/project/src/fonts/mod.rs` — `license` 모듈 설명 갱신.
- `docs/task-reports/XIII.6.6.md`(신규), 계획서(XIII.6.6 `[ ]`→`[x]`).

## 구현 요약

- **상태 저장**: `FontLicenseState`는 `Unknown`(1)·`Allowed`(2)·`Restricted`(3)·`UserConfirmed`(4)
  안정 tag를 가지며 기본값은 `Unknown`입니다. 프로젝트 폰트 record의 라이선스 nested record field 5에
  `u8`로 저장하고, `Unknown`이면 field를 생략합니다(canonical). 알 수 없는 tag와 명시적 `Unknown`은
  decode에서 거부합니다. 상태 field를 모르는 이전 reader는 이 optional field를 무시해 `Unknown`으로
  읽으므로, 호환 차이가 있어도 판정이 새로 생기지는 않습니다.
- **자동 확정 금지**: 상태 변경은 `transition(to, origin)`으로만 합니다. 사용자(`User`) 변경은 모든
  상태로 가능하고, 자동(`Automatic`) 경로는 `Unknown`으로 되돌리는 것만 허용하며 나머지는
  `AutomaticDetermination` 오류로 거부합니다(상태 유지). import는 호출자가 넘긴 라이선스 입력을 그대로
  기록하고 상태를 스스로 정하지 않습니다.
- **파일 선언은 참고 정보**: OS/2 `fsType`을 `EmbeddingPermissions`(원본 bits, 가장 덜 제한적인 수준,
  no-subsetting·bitmap-only)로 읽어 `ImportedProjectFont::embedding`으로 돌려주지만 상태를 바꾸지
  않습니다. OS/2가 없거나 짧거나 파일 밖이면 `None`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 291 passed, 1 ignored, 0 failed; 통합 3·5·5·4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| **상태 transition fixture**: (현재 4 × 요청 4 × 주체 2) 32개 조합 전체 — 사용자 전부 허용, 자동은 `Unknown`만 허용·나머지 `AutomaticDetermination`; 대표 경로(사용자 Restricted → 자동 Allowed 거부·상태 유지 → 자동 Unknown → 사용자 UserConfirmed)(`state_transition_fixture`) | 통과 |
| 상태 tag golden, 기본값 Unknown(`state_tags_are_stable_golden`) | 통과 |
| fsType RestrictedLicense·Installable 선언 폰트 import → 상태 Unknown, 선언은 `embedding`으로만 보고; 자동 판정 시도 거부; 사용자 상태 3종 encode/decode round-trip; 자동 Unknown 초기화 후 bytes가 Unknown 원본과 동일; 상태만 있는 라이선스 저장(`license_state_is_stored_and_never_determined_automatically`) | 통과 |
| 알 수 없는 상태 tag(0, 5, 255) 거부, 명시적 Unknown 비canonical 거부(`decode_rejects_unknown_or_explicit_unknown_license_state`) | 통과 |
| fsType 수준·flag 해석, OS/2 없음·짧음·파일 밖 → `None`(`os2_fs_type_is_read_as_embedding_declaration`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. OS/2 `fsType` 읽기는 "판정을 자동 확정하지 않음"을 검증하기 위한 참고 정보로 추가했습니다.

## 남은 위험 / 한계

- `LicenseChangeOrigin`은 호출자가 정합니다. agent 도구(ADR-0005)나 동기화 경로가 `User`로 호출하지
  않도록 연결 시점의 호출 경계에서 강제해야 합니다.
- 상태에 따른 Git 포함 경고(아키텍처 11.3 "라이선스상 저장 가능한 폰트만 Git에 포함")는 이 행 범위
  밖입니다. 현재는 상태 저장과 자동 확정 금지까지만 제공합니다.
- 상태 변경 이력(누가 언제 바꿨는지)은 기록하지 않습니다. command·history 연결 시 다룹니다.
