# 작업 보고서: XIII.5.1 AssetId·AssetRecord

- WBS: XIII.5.1
- 작업명: `AssetId` StableId newtype과 `AssetRecord { content_hash, media_type, size, source_metadata }` 정의
- 상태: 완료 (`[x]`)
- 선행: III.1.1, V.1.5

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/assets/src/id.rs`(신규) — `AssetId`, `ParseAssetIdError`.
- `crates/assets/src/media_type.rs`(신규) — `MediaType`, `MediaTypeError`.
- `crates/assets/src/record.rs`(신규) — `AssetRecord`, `SourceMetadata`, `AssetRecordError`,
  `SourceMetadataError`, `RecordKind`.
- `crates/assets/src/lib.rs` — `id`·`media_type`·`record` 모듈 등록.
- `docs/task-reports/XIII.5.1.md`(신규), 계획서(XIII.5.1 `[ ]`→`[x]`).

## 구현 요약

- **`AssetId`**: StableId와 같은 16바이트(big-endian, UUIDv7 레이아웃) 값을 감싼 newtype
  입니다. 정렬은 byte 사전식(= StableId 정렬), canonical 인코딩은 16바이트 raw(길이
  프리픽스 없음, manifest의 StableId 필드와 같은 규칙)입니다. 문자열은 storage가 공개하는
  StableId 문자열 codec(`ChunkId`의 `Display`/`FromStr`)에 변환만 맡겨 StableId와 같은
  형식(표준 소문자 hyphen UUID, 입력 대소문자 무관)을 보장하고, 파싱 오류 타입도
  `ParseStableIdError` 그대로입니다(`ParseAssetIdError` alias). 다른 도메인 식별자와
  섞어 쓰면 컴파일이 실패합니다(compile_fail doctest 2건).
- **`MediaType`**: `type/subtype` essence만 허용합니다(RFC 6838 `restricted-name`, 각 부분
  최대 127자, 파라미터·공백 거부). 대소문자를 구분하지 않으므로 ASCII 소문자로 정규화한
  값이 canonical입니다. 어떤 type을 허용할지는 다루지 않습니다(XIII.5.7).
- **`SourceMetadata`**: import 당시 외부 원본 경로(UTF-8) 하나를 담는 optional 출처
  정보입니다. 빈 경로·NUL 포함 경로는 거부합니다.
- **`AssetRecord` binary 형식**: canonical tagged record(R6). tag 1 content hash
  (ADR-0004 문자열, mandatory), 2 media type(mandatory), 3 size `u64` LE(mandatory),
  4 source metadata(nested record, optional, `None`이면 생략). payload는 값의 canonical
  인코딩이며 끝까지 정확히 소비해야 합니다. record는 AssetId를 포함하지 않아 같은 content
  hash를 여러 AssetId가 참조할 수 있습니다(XIII.5.3 전제).
- **엄격한 디코딩**: 절단·끝 잔여 bytes·field payload 잔여/부족·tag 중복/역순·정의되지 않은
  flags·unknown mandatory tag(nested 포함)·필수 field 누락·known tag의 mandatory flag
  불일치·잘못된 hash 문자열·잘못되거나 비canonical(대문자) media type·잘못된 source 경로를
  모두 오류로 거부합니다. unknown optional tag는 R6 정책대로 무시하며 재기록 시 결정론적으로
  제거됩니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | 33 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | 통과(exit 0) |

| 검사 | 결과 |
|---|---|
| AssetId binary·canonical(16바이트 raw)·string round-trip, nil, 대문자 입력 정규화(`id::tests`) | 통과 |
| AssetId 문자열 오류가 StableId 파서와 같은 값, 절단 거부, byte 순 정렬 | 통과 |
| AssetRecord round-trip(source 유무, 유니코드 경로, size 0·u32 경계·u64::MAX, 연속 5개) 및 재기록 동일 bytes | 통과 |
| 문서화한 layout과 golden bytes 일치(`encoding_matches_documented_layout`) | 통과 |
| 모든 절단 길이·잔여 bytes·unknown mandatory tag·flag 불일치·필수 field 누락·중복/역순 tag 거부 | 통과 |
| media type 형식·정규화·길이 경계, 비canonical media type·잘못된 hash·잘못된 source 경로 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- `AssetId`는 core-types `StableId`를 직접 필드로 갖지 않고 **StableId와 같은 16바이트
  표현**을 감쌉니다. crate 의존 정책(II.2.8, `stellar-assets` → `stellar-storage`만 허용)상
  assets가 core-types 타입 이름을 참조할 수 없고, storage도 `StableId`를 재노출하지 않기
  때문입니다. bytes·문자열 형식·정렬·파싱 오류는 StableId와 같으며, StableId와의 변환은
  core-types를 의존하는 상위 crate(project)가 16바이트로 수행합니다. 직접 newtype이
  필요하다고 판단되면 의존 정책 변경(assets → core-types edge 허용)이 선행되어야 합니다.

## 남은 위험 / 한계

- AssetId 문자열 변환이 storage `ChunkId` codec에 위임되어 있어, storage가 ChunkId 문자열
  형식을 StableId와 다르게 바꾸면 함께 바뀝니다(현재 ChunkId는 StableId 위임).
- AssetId 생성(UUIDv7 발급)은 assets에 두지 않았습니다. import(XIII.5.2)에서 상위 crate가
  발급한 id를 받거나 bytes로 전달받는 방식으로 연결해야 합니다.
- unknown optional tag를 보존하지 않고 무시하므로, 미래 버전이 쓴 optional field는 이 버전이
  재기록하면 사라집니다(R6 "무시" 정책 선택).
- media type 허용 목록·unknown type의 binary 저장 정책·최대 크기는 XIII.5.7에서 정합니다.
