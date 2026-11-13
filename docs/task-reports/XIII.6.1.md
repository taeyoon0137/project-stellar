# 작업 보고서: XIII.6.1 Project Font import

- WBS: XIII.6.1
- 작업명: Project Font import를 AssetStore와 연결하고 font file hash, face count, licensing metadata 입력 필드를 기록
- 상태: 완료 (`[x]`)
- 선행: XIII.5.2(`[x]`), X.8.1(`[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증). 폰트 fixture는 테스트 코드가 header·name table만 합성합니다.

## 변경 파일 목록

- `crates/project/src/fonts/mod.rs`(신규) — 폰트 모듈 개요.
- `crates/project/src/fonts/sfnt.rs`(신규) — `FontFileFormat`, `FontFaceNames`, `FontFileInfo`,
  `FontParseError`, `read_font_file`(container probe·face 수·이름).
- `crates/project/src/fonts/license.rs`(신규) — `FontLicenseInfo`, `LicenseField`, `LicenseFieldError`.
- `crates/project/src/fonts/project_font.rs`(신규) — `ProjectFontRecord`(canonical encode/decode,
  face 등록), `import_project_font`, `ProjectFontImportError`, `ImportedProjectFont`.
- `crates/project/src/fonts/test_fixtures.rs`(신규, `cfg(test)`) — 합성 sfnt·TTC·WOFF·WOFF2 bytes, 임시 디렉터리.
- `crates/project/src/lib.rs` — `pub mod fonts;` 한 줄(+문서 주석).
- `crates/project/Cargo.toml`, `Cargo.lock` — `stellar-text` path 의존 추가(crate graph 허용은 78a574a).
- `docs/task-reports/XIII.6.1.md`(신규), 계획서(XIII.6.1 `[ ]`→`[x]`).

## 구현 요약

- **container probe**(`sfnt`): header와 name table만 읽어 형식(TrueType·OpenType CFF·Collection·
  WOFF·WOFF2), face 수, face별 family(nameID 16→1)·subfamily(17→2)·PostScript(6) 이름을 얻습니다.
  TTC는 face offset마다 sfnt signature를 확인하고, WOFF2 collection은 table directory(UIntBase128,
  transform 규칙)를 건너뛰어 collection directory의 `numFonts`와 face별 table index 범위를 검사합니다.
  WOFF·WOFF2 이름은 압축 table이라 읽지 않습니다.
- **신뢰하지 않는 입력**: 모든 읽기는 파일 길이 안인지 먼저 확인하고, face 수 4096·table 수 1024·
  name table 1 MiB·이름 1024 bytes 상한을 둡니다. 재귀 없이 반복문만 씁니다. container 구조 오류는
  typed error로 거부하고, name table 손상은 그 face 이름만 비웁니다. 이름 record 선택은 고정 순위
  (Windows English US → Windows 기타 언어 → Unicode → Mac Roman ASCII)로 결정론적입니다.
- **import**(`import_project_font`): 원본 probe(폰트가 아니면 쓰기 전 거부) → probe 형식의 media
  type이 `config/asset-import.yaml` 허용 목록에 있어야 함(binary 대체 저장 금지) → `AssetStore::import_file`
  (streaming hash·dedup·atomic write) → 저장된 content를 다시 probe해 face 수·이름을 확정합니다.
  font file hash는 에셋 content hash(SHA-256) digest와 같습니다.
- **record**(`ProjectFontRecord`): asset id, font file hash, 형식, face별 이름(길이 = face 수),
  사용자 입력 라이선스 metadata(이름·URL·저작권·메모, 모두 선택)를 기록합니다. 형식별 face 수
  규칙(단일 face 형식은 정확히 1)을 강제하고, canonical tagged record(R6)로 encode/decode합니다.
  decode는 절단·잔여 bytes·unknown mandatory tag·mandatory flag 불일치·필수 field 누락·형식과 맞지
  않는 face 수·빈 문자열·빈 라이선스 record를 거부하고 unknown optional field는 무시합니다.
- **등록**: `register_faces`가 face마다 `FontSource::Project`, `FontFaceKey(hash, index)`로
  `FontRegistry`에 등록합니다. 같은 파일 재import는 에셋 dedup과 registry dedup으로 같은 handle입니다.
- Stellar는 라이선스 입력값을 해석하거나 URL을 열지 않습니다(판정 상태는 XIII.6.6).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 268 passed, 0 failed; 통합 3·5·5·4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo xtask validate-crate-graph` | 종료 코드 0 |

| fixture | 결과 |
|---|---|
| **multi-face 등록**: 3 face TTC import → hash = 원본 SHA-256, 형식 Collection, face 3·이름, `font/collection`·`.ttc` content, registry 3 handle(Project 출처, hash+index 조회 일치), 재import dedup·같은 handle(`multi_face_collection_is_imported_and_registered`) | 통과 |
| 단일 face TTF import(`single_face_font_is_imported_with_font_media_type`) | 통과 |
| 폰트 아님·없는 파일·디렉터리·허용 목록 밖·정책 reject·크기 상한 → 쓰기 0(`non_fonts_and_disallowed_types_are_rejected_before_writing`) | 통과 |
| record canonical round-trip, 빈 라이선스 생략(`record_round_trips_canonically`) | 통과 |
| hostile record decode 거부(`decode_rejects_hostile_records`), 형식별 face 수(`record_constructor_checks_face_count_for_format`) | 통과 |
| probe: 형식 tag golden, TTC·WOFF·WOFF2(255UInt16 세 경로) face 수, 이름 순위·손상 이름, hostile header(`fonts::sfnt::tests`) | 통과 |
| 라이선스 field 입력·검증(`fonts::license::tests`) | 통과 |

## 차이(작업 지시와 다른 점)

- 허용 경로(`crates/project/src/fonts/`, `lib.rs` 한 줄) 밖인 `crates/project/Cargo.toml`·`Cargo.lock`에
  `stellar-text` 의존 한 줄을 추가했습니다. `FontFileHash`·`FontRegistry` 사용에 필요하며 crate graph는
  78a574a에서 이미 허용되었습니다.

## 남은 위험 / 한계

- record를 프로젝트 저장 구조(문서 chunk·manifest)에 연결하는 작업은 이 행 범위 밖입니다. 현재는
  값 타입과 canonical bytes까지만 제공합니다.
- import 실패 중 4단계(저장 후 재probe) 실패는 이미 쓴 content를 남깁니다. 참조가 없으므로 asset
  GC(XIII.5.6)가 회수합니다.
- WOFF·WOFF2 face 이름은 압축 해제 없이 읽지 않으므로 비어 있습니다.
- 실제 상용 폰트 파일 대신 합성 header fixture로 검증했습니다. 실제 system 폰트 probe는 XIII.6.2
  smoke에서 확인합니다.
