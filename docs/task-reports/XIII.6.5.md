# 작업 보고서: XIII.6.5 폰트 파일 hash 불일치 재현성 경고

- WBS: XIII.6.5
- 작업명: font file hash가 예상과 다르면 같은 family name이어도 다른 face로 처리하고 재현성 경고를 생성
- 상태: 완료 (`[x]`) — 2026-11-23 plan-revision이 ADR-0006 근거로 Windows 선행을 뺀 뒤 계획서에 반영했습니다(재판정의 Windows 선행 재작성안). 재판정 기록: `[~]` 유지 — 구현 판정 통과, 선행 미충족(2026-11-23 재판정, 아래 "재판정" 절). 이전 기록: 부분 완료 (`[~]`) — 플랫폼 독립 검사와 macOS·Project·External 출처 연결로 구현·검증 완료,
  선행 XIII.6.3(Windows system font adapter)은 ADR-0006으로 `[!]`
- 선행: XIII.6.1·XIII.6.2·XIII.6.4(이 lane에서 완료). 2026-11-23 개정 전 선행 범위에 있던 XIII.6.3(`[!]`, ADR-0006)은 뺐습니다.

## 필수 실행 환경

- `DEV-MAC` (순수 Rust 검사, 합성 폰트 fixture를 실제 파일로 import·discovery).

## 변경 파일 목록

- `crates/project/src/fonts/reproducibility.rs`(신규) — `ExpectedFontFace`, `FaceNameIndex`,
  `FontFaceCandidate`, `FontHashMismatchWarning`, `ReproducibilityReport`, `check_font_reproducibility`.
- `crates/project/src/fonts/mod.rs` — `reproducibility` 모듈 추가.
- `docs/task-reports/XIII.6.5.md`(신규), 계획서(XIII.6.5 `[ ]`→`[~]`).

## 구현 요약

- **해석은 hash+index로만**: 기록된 face(`ExpectedFontFace` = `FontFaceRef` + family 이름)는
  registry에서 파일 hash + face index로만 찾습니다. family 이름이 같아도 hash가 다르면 registry key가
  달라 다른 face이고, 그 handle을 대신 돌려주지 않습니다.
- **재현성 경고**: 기록 face가 없을 때 `FaceNameIndex`(출처별 발견 face의 family 이름 → key)에서
  같은 이름·다른 hash face를 찾아 `font-hash-mismatch` 경고를 만듭니다. 경고에는 기록 face, family
  이름, 후보 key와 출처가 들어갑니다. 같은 파일의 다른 face index는 hash 불일치가 아니므로 후보에서
  뺍니다. 같은 기록은 경고 한 번으로 묶고 결과는 정렬됩니다.
- **출처 연결**: `add_system_catalog`(XIII.6.2), `add_project_font`(XIII.6.1)와 일반 `insert`
  (External 등)로 색인을 채우고, `ExpectedFontFace::from_project_font`로 프로젝트 폰트 기록을 검사
  입력으로 바꿉니다. 이름 비교는 앞뒤 공백 제거 + Unicode 소문자입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 286 passed, 1 ignored, 0 failed; 통합 3·5·5·4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| **same name/different hash**: 프로젝트가 import한 "Fixture Sans" Regular(original)와 system 위치의 같은 이름 revised(bytes 다름) → 이름 동일·hash 다름, 기록 face 미해석(`None`), `font-hash-mismatch` 경고 1개(후보 = system face). 기록 파일도 등록하면 그 handle로만 해석, 경고 0, 두 face handle 다름(`same_name_with_different_hash_is_a_different_face_with_warning`) | 통과 |
| 같은 이름 후보 없음·같은 파일 다른 index·family 없는 face → 경고 0, 대소문자·공백 무시 비교(`no_warning_without_same_name_candidates_or_for_same_file_other_index`) | 통과 |
| 중복 기록 경고 1회, 출처 정렬, 빈 family 거부(`warnings_are_deduplicated_and_ordered`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 범위 XIII.6.1~XIII.6.4 중 XIII.6.3(Windows)은 ADR-0006으로 `[!]`입니다. 검사 로직은 플랫폼
  독립이고 macOS·Project·External 출처로 검증했지만, Windows system font 출처 연결은 확인할 수 없어
  XIII.4.4 선례대로 `[~]`로 둡니다.

## 남은 위험 / 한계

- 경고를 문서 진단 집계(XXV.2.1)와 UI에 연결하는 작업은 이 행 범위 밖입니다.
- 이름 비교는 Unicode 정규화(NFC/NFD)를 하지 않습니다. 정규화 형태만 다른 family 이름은 다른 이름으로
  봅니다.
- WOFF·WOFF2 face는 이름을 읽지 않으므로(XIII.6.1) 색인 후보가 되지 않습니다.
- Windows 재개 시 Windows system font discovery 결과를 같은 색인에 넣어 같은 fixture를 다시 검증해야
  합니다.

## 재판정 (2026-11-23, ledger-verification)

- 판정: `[~]` 유지 — 구현 판정 통과, 선행 미충족.
- 근거: 같은 family 이름·다른 hash를 다른 face로 처리하고 재현성 경고를 만드는 검사는 플랫폼 독립이며 macOS·Project·External 출처로 확인했습니다. 선행 XIII.6.3(Windows)이 `[!]`라 `[~]`를 유지합니다.
- 판정 기준: 행 지시·검증 열의 불변식을 구현 코드에서 확인했고, 테스트 통과는 보조 근거로만 썼습니다. 계획서 §1.2 규칙 4(선행이 모두 `[x]`여야 완료)를 적용했습니다. 기준 커밋은 `main` `ce60bb92`입니다.

### 확인한 코드 위치

- `crates/project/src/fonts/reproducibility.rs`: registry key는 파일 hash + face index, `check_font_reproducibility`가 `font-hash-mismatch` 경고를 만들고 정렬·중복 제거합니다.
- `check_font_reproducibility`를 부르는 제품 경로는 아직 없습니다(경고를 진단 집계·UI에 연결하는 일은 XXV.2.1 범위).

### 실행한 검증

| 명령 | 결과 |
|---|---|
| `cargo test --workspace`(DEV-MAC, 샌드박스 해제) | 테스트 binary 142개 모두 ok, 4136 passed, 0 failed |
| `cargo test -p stellar-project` | 392 passed, 0 failed |

### 남은 조건과 finding

- Windows 선행 재작성안: "XIII.6.1\~XIII.6.4"를 "XIII.6.1, XIII.6.2, XIII.6.4"로 둡니다. Windows system font 출처 연결 검증은 XIII.6.3 자신의 검증에 더합니다.
