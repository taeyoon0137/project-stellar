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
  → **해소**: 의존 정책이 `assets → core-types`를 허용한 뒤(99448ba) `AssetId(StableId)`
  newtype으로 바꿨습니다(아래 "후속 수정(리뷰 반영 3차)"). 이 차이는 더 이상 없습니다.

## 남은 위험 / 한계

- AssetId 문자열 변환이 storage `ChunkId` codec에 위임되어 있어, storage가 ChunkId 문자열
  형식을 StableId와 다르게 바꾸면 함께 바뀝니다(현재 ChunkId는 StableId 위임).
- AssetId 생성(UUIDv7 발급)은 assets에 두지 않았습니다. import(XIII.5.2)에서 상위 crate가
  발급한 id를 받거나 bytes로 전달받는 방식으로 연결해야 합니다.
- ~~unknown optional tag를 보존하지 않고 무시하므로, 미래 버전이 쓴 optional field는 이 버전이
  재기록하면 사라집니다(R6 "무시" 정책 선택).~~ → 후속 수정으로 "보존" 정책으로 바꿨습니다
  (아래 절).
- media type 허용 목록·unknown type의 binary 저장 정책·최대 크기는 XIII.5.7에서 정합니다.

## 후속 수정(리뷰 반영)

- **원본 경로 기록 정책(Fix)**: `SourceMetadata`가 import한 사용자의 절대 로컬 경로를
  그대로 기록하던 문제를 고쳤습니다. record는 Git에 commit되므로(아키텍처 29.1 "clone·
  worktree·프로젝트 이동에서 재현 가능") 이제 원본 위치를 `SourceLocation`으로 기록합니다.
  - 프로젝트 내부: project root 기준 `/` 상대 경로만(`SourceMetadata::project`). 빈 경로·
    NUL·`\`·선행 `/`·빈 구성 요소·`.`·`..`는 생성과 디코딩 모두에서 거부합니다.
  - 프로젝트 외부: 파일 이름과 ~~비가역~~ 출처 지문(`external_origin_fingerprint` =
    domain 구분 prefix + 해석된 절대 경로 bytes의 SHA-256)만(`SourceMetadata::external`).
    → **정정**: key 없는 SHA-256은 비가역이 아닙니다. 파일 이름과 함께 commit되므로 누구나
    `/Users/<이름>/Downloads/<파일 이름>` 같은 후보를 계산해 맞춰 볼 수 있습니다(사전 공격).
    아래 "후속 수정(리뷰 반영 2차)"에서 기기별 비밀 key의 HMAC으로 바꿨습니다.
  - `from_import_path`는 root·원본을 canonicalize(symlink 해석)한 뒤 구성 요소 단위로
    포함 여부를 판정합니다(`/work/project-old`는 `/work/project`의 내부가 아님).
    UTF-8로 표현할 수 없는 경로는 기록하지 않고 `NonUtf8Path`로 거부합니다.
  - nested record 형식: tag 1 location kind(`u8`, 1 내부·2 외부), tag 2 path(내부 상대
    경로 또는 외부 파일 이름), tag 3 origin fingerprint(외부 전용 필수, 내부 kind에 있으면
    `UnexpectedField`로 거부). 정의되지 않은 kind는 `InvalidSourceKind`로 거부합니다.
  - 검증: 외부 원본 record bytes에 디렉터리 경로 문자열이 남지 않음
    (`resolved_paths_outside_project_keep_only_file_name_and_fingerprint`), 내부·외부 golden
    layout, kind·지문 조합 검증, canonicalize 기반 판정(실제 temp 디렉터리) 테스트를
    추가했습니다. `cargo test -p stellar-assets` 42 passed + doctest 2 passed, clippy 경고 0,
    fmt 통과.
- **`AssetId` 표현(재검토, 변경 없음)**: storage는 core-types `StableId`를 재노출하지 않고
  (`pub use` 없음), `stellar-assets`의 허용 의존은 `stellar-storage`뿐이며(`crate_graph.rs`),
  Cargo는 전이 의존 crate 이름 참조를 허용하지 않으므로 `AssetId(StableId)`로 바꿀 수
  없습니다. 위 "차이" 절의 16바이트 표현을 유지합니다. 직접 newtype으로 바꾸려면 storage의
  `StableId` 재노출 또는 assets → core-types edge 허용이 먼저 필요합니다.
- **unknown optional field 전방 호환 정책(Fix)**: 조용히 버리던(silent drop) 동작을 R6의
  **보존** 정책으로 명시·고정했습니다.
  - unknown mandatory tag는 기존대로 `UnknownMandatoryTag`로 거부합니다.
  - unknown optional tag는 `PreservedFields`(tag 오름차순, tag·payload 그대로)에 보존하고,
    재기록 시 known field와 함께 tag 순으로 다시 씁니다. 미래 버전이 쓴 record를 이 버전이
    decode → encode해도 같은 bytes입니다(결정론). AssetRecord와 nested SourceMetadata 모두
    같은 정책입니다.
  - 보존 field는 decode로만 생깁니다(공개 생성자는 빈 목록뿐). 새 content를 만드는 연산은
    `AssetRecord::new`로 보존 field 없는 record를 만들며 이전 record의 보존 field를 이어받지
    않습니다. known field의 정의되지 않은 값(예: location kind 3)은 보존하지 않고 거부합니다.
  - storage `take_fields`가 unknown optional field를 버리므로 같은 wire 규칙의 reader를
    assets에 두었습니다. 구조 오류(`InvalidFlags`·`DuplicateTag`·`MisorderedTag`·
    `UnknownMandatoryTag`·모든 절단 길이)와 known field 결과가 storage decoder와 같음을
    `record_reader_agrees_with_storage_take_fields`로 고정했습니다.
  - 검증: `cargo test -p stellar-assets` 45 passed + doctest 2 passed, clippy 경고 0, fmt 통과.

## 후속 수정(리뷰 반영 2차): 외부 원본 출처 지문을 기기 key HMAC으로 교체

- **문제**: 외부 원본 지문이 key 없는 `SHA-256(domain ‖ 절대 경로)`였습니다. Git에 commit된
  record에 파일 이름과 함께 남으므로, 저장소만 가진 사람도 후보 경로를 계산해 맞춰 보는
  사전 공격으로 사용자 이름·디렉터리를 알아낼 수 있었습니다. 위 1차 수정 절과 모듈 문서의
  "비가역·경로를 복원할 수 없음" 서술은 보장을 과장했습니다(위 본문에 정정 표시).
- **수정** (`crates/assets/src/origin.rs` 신규, `hash_stream.rs`·`record.rs`·`store.rs` 변경):
  - `hash_stream::hmac_sha256` — 기존 in-crate SHA-256 위의 HMAC-SHA-256(RFC 2104).
    RFC 4231 test case 1·2·3·4·6·7과 64 bytes key 경계 vector로 고정(값은 로컬
    `openssl`·Python `hmac`으로 교차 확인).
  - `OriginKey`(32 bytes OS 난수, `Debug`는 key 식별자만 표시)와 `OriginKeyFile` — 프로젝트
    밖 OS 사용자 데이터 영역 `<data_dir>/assets/origin-key`(신뢰 저장소 XVI.7.2와 같은 base,
    `data_dir`은 셸이 정함). 처음 필요할 때(외부 원본을 기록·확인할 때) `0600` temp에 쓰고
    sync한 뒤 hard link로 만들며, 이미 있으면 그 key를 씁니다(덮어쓰지 않음). 손상 파일·
    `0600`보다 넓은 권한·project root 안 경로(symlink 경유 포함)는 명시 오류입니다.
  - `OriginFingerprint` = key 식별자(`HMAC(key, id domain)` 앞 8 bytes) + `HMAC(key,
    "stellar.asset.source-origin.v2\0" ‖ 경로 bytes)`. record tag 3 payload 형식이
    `hmac-sha256:<key-id hex16>:<mac hex64>`(93자)로 바뀌었고, 이전 `sha256:<hex>` 형식은
    `InvalidOriginFingerprint(UnknownScheme)`로 거부합니다. 이전 형식은 이 lane의 미병합
    커밋에서만 만들어졌고 저장소에 그 형식의 fixture 파일이 없어 의도적으로 호환을 두지
    않았습니다(golden layout 테스트 길이를 갱신).
  - `SourceMetadata::from_resolved_paths`·`from_import_path`가 `&dyn OriginKeyProvider`를
    받고 외부 원본일 때만 key를 요청합니다(내부 원본은 key 파일을 만들지 않음).
    `matches_external_origin`·`check_external_origin`(→ `OriginCheck::{Matches,
    DifferentOrigin, ForeignKey, NotExternal}`)이 이 기기 key를 받습니다.
  - `AssetStore::new`는 key가 없습니다. 외부 원본을 기록·확인하려면
    `with_origin_key_file`(또는 테스트용 `with_origin_key`)로 지정해야 하며, 없으면
    `SourceMetadataError::OriginKey(OriginKeyError::Unconfigured)`로 아무 것도 쓰기 전에
    실패합니다.
- **보장 범위(정확한 서술)**: key가 없는 사람은 지문으로 경로 추측을 확인할 수 없습니다.
  key 파일을 읽을 수 있는 같은 기기 사용자는 확인할 수 있으므로 "복원 불가능한 값"이 아니라
  "key 없이 검증할 수 없는 값"입니다. key 식별자는 경로 정보가 없지만 같은 기기에서 만든
  record끼리 연결할 수 있게 합니다. 다른 기기에서는 같은 출처를 확인할 수 없습니다
  (XIII.5.5 `SourceUnavailable::OriginUnverifiable` → 사용자에게 원본을 다시 선택받음).
- **검증**: `committed_fingerprint_does_not_confirm_guessed_paths_without_key`(이전 key 없는
  hash·추측 key로 확인 불가), `fingerprint_is_keyed_domain_separated_and_path_sensitive`,
  `fingerprint_string_round_trips_strictly`, key 파일 생성·재사용·`0600`·손상 비덮어쓰기·
  넓은 권한 거부·프로젝트 안 경로 거부 테스트, store의 key 미설정·첫 외부 사용 시 생성·
  프로젝트 안 key 파일 거부 테스트. `cargo test -p stellar-assets` lib 115 passed +
  integration 1 passed + doctest 2 passed, clippy 경고 0, fmt 통과, `validate-crate-graph` 통과.
- **남은 위험**: 비Unix의 OS 난수는 아직 없습니다(`RandomUnavailable`, Windows 지원 보류).
  key는 메모리에서 지우지(zeroize) 않습니다. key 파일 위치(`data_dir`)를 셸이 연결해야 실제
  외부 원본 import가 동작합니다.

## 후속 수정(리뷰 반영 3차): `AssetId`를 `StableId` newtype으로 변경

- **문제**: 작업 지시("`AssetId` StableId newtype")와 달리 `AssetId`가 `[u8; 16]`을 감쌌고,
  문자열 변환을 storage `ChunkId` codec에 위임했습니다. 상태 `[x]`가 지시 미충족을 가렸습니다.
- **수정**: `crates/assets/Cargo.toml`에 `stellar-core-types` path 의존을 추가하고(의존 정책
  99448ba에서 허용, `validate-crate-graph` 통과) `pub struct AssetId(StableId)`로 바꿨습니다.
  `from_stable_id`·`stable_id`·`From<StableId>`·`From<AssetId> for StableId`를 추가했고,
  `Display`·`FromStr`은 StableId에 직접 위임합니다(`ParseAssetIdError = ParseStableIdError`).
  `ChunkId` codec 의존은 제거했습니다. 감싸지 않은 `StableId`를 `AssetId` 자리에 넘기면
  컴파일이 실패합니다(compile_fail doctest 추가, 명시 변환 예시 doctest 추가).
- **인코딩 호환**: canonical 인코딩은 그대로 16바이트 raw(big-endian)이고 정렬·문자열도
  같으므로 기존 fixture·golden 값이 그대로 유효합니다(`canonical_encoding_is_16_raw_bytes_and_round_trips`
  변경 없음 통과, 기존 sample bytes = `StableId::new_v7(0x01923c4d5e6f, 0xabc, 0x0123456789abcdef)`
  출력임을 `wraps_stable_id_without_changing_representation`으로 고정).
- **부수 변경**: 의존 추가로 workspace `Cargo.lock`의 `stellar-assets` 의존 목록에
  `"stellar-core-types"` 한 줄이 추가되었습니다(CI가 `--locked`를 쓰므로 함께 기록).
- **상태**: 지시 내용(StableId newtype)과 검증(round-trip)을 모두 충족하므로 계획서 `[x]`를
  유지합니다. `planning/task-status.yaml`은 이 작업에서 수정하지 않았습니다.
- **검증**: `cargo test -p stellar-assets` lib 116 passed + integration 1 passed + doctest
  4 passed(compile_fail 3, 실행 1), clippy 경고 0, fmt 통과, `validate-crate-graph` 통과,
  `cargo check -p stellar-project` 통과.

## 후속 수정(보안 리뷰 반영): 출처 key 파일 temp 생성 충돌 구분

- **문제**: `OriginKeyFile::load_or_create`가 예측 가능한 temp 이름(`.origin-key.tmp-<pid>-<n>`)을
  쓰면서, temp 생성(`create_new`)의 `AlreadyExists`를 hard link의 `AlreadyExists`("다른 process가
  먼저 key를 만듦")와 같은 분기로 처리했습니다. 이름을 미리 점유하면 다시 읽어도 key가 없어
  잘못된 `Corrupt`를 돌려줬고, 이 호출이 만들지 않은 점유 항목을 무조건 `remove_file`했습니다.
- **수정**: temp 이름에 매 호출 OS 난수 8 bytes suffix를 붙이고(`.origin-key.tmp-<pid>-<n>-<hex16>`),
  `atomic_write::create_new_temp`로 후보 이름을 차례로 `create_new` 합니다. 이미 있는 이름은 열거나
  지우지 않고 다음 이름으로 넘어가며, 모두 차 있으면 `Io { kind: AlreadyExists }`입니다.
  hard link 단계의 `AlreadyExists`만 "이미 발행됨"으로 보고 다시 읽습니다(최대 4회, 손상 파일은
  `Corrupt`). `remove_file`은 이 호출이 만든 temp에만 합니다.
- **검증**: `squatted_temp_names_neither_block_creation_nor_get_removed`(이전 형식 이름 32개를
  victim 파일 symlink로 점유 → 정상 생성, victim 불변, 점유 symlink 보존, temp 미잔류),
  `temp_collision_is_not_mistaken_for_published_key`(점유 이름뿐이면 `AlreadyExists` I/O 오류,
  점유 이름 건너뛰기, 최종 key가 있으면 덮어쓰지 않음), `temp_key_names_are_unpredictable_and_distinct`.
  `cargo test -p stellar-assets` lib 133 passed + integration 1 passed + doctest 8 passed, clippy
  경고 0, fmt 통과.
