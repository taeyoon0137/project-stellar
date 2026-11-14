# 작업 보고서: XIII.5.7 에셋 import 정책 config

- WBS: XIII.5.7
- 작업명: import 허용 media type과 최대 단일 파일 크기 정책을 config에 정의하고 unknown type의
  binary asset 저장 가능 여부를 명시
- 상태: 완료 (`[x]`)
- 선행: XIII.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `config/asset-import.yaml`(신규) — `max_file_size_bytes`, `unknown_media_type`, `media_type`
  허용 목록(12개)과 저장 확장자.
- `crates/assets/src/policy.rs`(신규) — `AssetImportPolicy`, `UnknownMediaTypePolicy`,
  `ImportDecision`, `PolicyViolation`, `PolicyConfigError`, `BINARY_MEDIA_TYPE`.
- `crates/assets/src/store.rs` — `ImportRequest::with_max_size`/`max_size`, hash pass 크기 제한,
  `ImportError::TooLarge`·`ImportError::Policy`.
- `crates/assets/src/update_source.rs` — `AssetStore::update_from_source_with_policy`.
- `crates/assets/src/lib.rs` — `policy` 모듈 등록.
- `docs/task-reports/XIII.5.7.md`(신규), 계획서(XIII.5.7 `[ ]`→`[x]`).

## 구현 요약

- **config 형식**: 기존 config(`storage-gc.yaml`·`document-limits.yaml`)와 같은 `key: value`
  한 줄 형식(중첩 없음)이며 `media_type: <type/subtype> <safe-ext>`만 반복할 수 있습니다.
  필수 key 누락·단일 key 중복·잘못된 값(0 이하 크기, 정의되지 않은 unknown 정책, media type·
  확장자 형식 위반, 토큰 수 위반)·정규화 후 media type 중복·알 수 없는 key는 모두 명시 오류입니다.
  보호 config(`config/performance-budgets.yaml`)는 건드리지 않았습니다.
- **크기**: `size <= max_file_size_bytes`만 허용합니다(경계값 포함, 1 byte 초과 거부). 정책
  요청은 원본 메타데이터 크기로 읽기 전에 거부하고, 읽는 도중 파일이 커져도 상한을 넘는 순간
  멈춥니다(`ImportError::TooLarge`). 어느 경우든 `assets/`에 아무 것도 쓰지 않습니다.
  출하 값은 512 MiB(잠정, 벤치마크 후 재조정 — config 주석에 명시)입니다.
- **허용 목록**: PNG·JPEG·GIF·WebP·AVIF·SVG 이미지, TTF·OTF·WOFF·WOFF2·collection 폰트,
  `application/octet-stream`. 허용 type은 그대로 기록하고 표의 확장자로 저장합니다(원본 파일
  확장자 무관, 예: `photo.JPEG` → `.jpg`).
- **unknown type**: 출하 config는 `store_as_binary` — 허용 목록 밖 type은 **binary asset으로
  저장 가능**합니다. record media type은 `application/octet-stream`, 확장자는 `bin`이며
  `ImportDecision::fallback_to_binary = true`로 표시합니다. `reject`로 바꾸면
  `PolicyViolation::MediaTypeNotAllowed`로 거부합니다.
- **연결**: `AssetImportPolicy::request(source, declared)`가 판정된 media type·확장자·최대 크기를
  담은 `ImportRequest`를 만듭니다. `update_from_source_with_policy`도 같은 정책을 적용하며,
  위반 시 아무 것도 쓰지 않아 기존 asset이 유지됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 97 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 출하 config 파싱, 크기 경계 fixture(max−1000·max−1·max 허용, max+1·max+1000 거부, 0 허용, `u64::MAX` 거부)(`shipped_config_defines_size_and_type_boundaries`) | 통과 |
| 출하 config의 type별 확장자, unknown type → `application/octet-stream`·`bin`·fallback 표시 | 통과 |
| 실제 import 크기 경계 fixture(max 1000: 0·999·1000 성공, 1001·2000 `TooLarge`, 초과 시 파일 미기록), 정책 없는 요청은 무제한(`import_size_boundary_fixture`) | 통과 |
| `reject` 정책의 unknown type 거부(`decide`·`request`) | 통과 |
| 정책 확장자 적용(`photo.JPEG` → `.jpg`), unknown type의 binary 저장 | 통과 |
| Update from Source 정책 적용: 상한 초과 거부·기존 유지, 상한 이내 갱신, 허용 해제된 type 거부 | 통과 |
| config 오류 15종 명시 오류, 없는 파일 `Io` | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- 512 MiB 상한과 허용 목록은 잠정값입니다. Git·LFS 권고 임계치(XIII.7.2)와 저장소 벤치마크
  결과로 재조정해야 합니다.
- media type은 호출자가 선언한 값을 판정합니다. 파일 내용 sniffing(magic bytes)으로 선언과
  실제 형식이 맞는지 확인하는 것은 이 작업 범위 밖입니다.
- \~\~정책 없이 만든 `ImportRequest::new`는 크기 제한이 없습니다. 사용자 import 경로는 항상
  `AssetImportPolicy::request`를 거치도록 상위 계층(project·셸)에서 연결해야 합니다.\~\~
  → 후속 수정으로 정책 없는 요청은 crate 내부 전용이 되었습니다(아래 절).
- 읽는 도중 커진 파일의 `TooLarge.size`는 전체 크기가 아니라 상한을 넘은 시점까지 읽은
  크기입니다.

## 후속 수정(리뷰 반영): 공개 import 경로에 정책 강제

- **문제**: 정책이 opt-in이었습니다. 공개 `ImportRequest::new`/`import_file`은 원본의 아무
  `[a-z0-9]{1,16}` 확장자(예: `html`, `exe`)를 저장 확장자로 쓰고 크기 제한이 없었으며,
  `config/asset-import.yaml`은 `AssetImportPolicy::request`를 쓸 때만 적용되었습니다.
- **수정**:
  - `ImportRequest::new`·`with_extension`·`with_max_size`를 `pub(crate)`로 바꿨습니다. crate
    밖에서는 `AssetImportPolicy::request`로만 요청을 만들 수 있어, 공개 import는 항상 정책의
    media type 판정(허용 목록·unknown type 처리), 표의 저장 확장자, 최대 단일 파일 크기를
    거칩니다. `without_source_metadata`와 조회 메서드는 공개로 남겼습니다.
  - `AssetStore::update_from_source`와 `update_from_selected_source`가 `&AssetImportPolicy`를
    필수 인자로 받습니다. 정책 없는 `update_from_source`와 `_with_policy` 변형은 제거했습니다
    (공개 API 변경, 현재 workspace에 외부 호출자 없음 — `cargo check -p stellar-project` 통과).
  - integration test(`tests/import_memory.rs`)도 출하 정책으로 요청을 만듭니다.
- **검증**: `public_import_path_always_applies_policy` — `page.html`(`text/html`)·
  `tool.exe`는 `application/octet-stream`·`.bin`으로 저장, `image/png`로 선언한 `logo.html`은
  `.png`, 요청은 항상 정책 상한을 담음, `reject` 정책은 요청 단계에서 거부하고 아무 것도 쓰지
  않음. 크기 경계 fixture에서 공개 요청이 상한 초과 시 `TooLarge`. 정책 없는 요청 생성과
  확장자·크기 덮어쓰기가 crate 밖에서 컴파일되지 않음(compile_fail doctest 2건, 일시적으로 일반
  doctest로 바꿔 `E0624` private 오류임을 확인). `cargo test -p stellar-assets` lib 118 passed +
  integration 1 passed + doctest 7 passed, clippy 경고 0, fmt 통과, `validate-crate-graph` 통과.
- **남은 위험**: crate 내부 코드는 여전히 정책 없는 요청을 만들 수 있습니다(단위 테스트·정책 구현
  전용). media type은 여전히 호출자 선언 값이며 내용 sniffing은 하지 않습니다.
