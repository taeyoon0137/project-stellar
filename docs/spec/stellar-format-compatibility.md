# Stellar 파일 포맷 버전 호환성 matrix와 deprecation 정책

- 문서 종류: 규범(normative) 명세와 현재 구현 기록.
- WBS: XXVIII.2.4
- 기준 commit: `78a574a` 위의 구현. 형식은 [`stellar-format.md`](stellar-format.md)(이하 "형식 명세"),
  보안 한도는 [`stellar-format-security.md`](stellar-format-security.md)(이하 "보안 문서")에 있습니다.
- 표기: 형식 명세 §0과 같습니다.
- 이 문서의 정책은 이미 Accepted된 ADR(ADR-0004, ADR-0007, ADR-0008, ADR-0010, ADR-0011, ADR-0012)의
  결정을 모은 것입니다. ADR에 없는 결정은 만들지 않았고 §8 "정하지 않은 사항"에 남겼습니다.

## 1. 범위

- 프로젝트 폴더 안 파일의 버전 축과, 서로 다른 버전의 reader·data 조합에서 일어나는 일.
- 형식을 바꿀 때 지켜야 하는 절차(deprecation 정책).
- 제품 버전(앱 release 번호)과 형식 버전의 대응, 공개 형식 버전 동결은 다루지 않습니다(XXVIII.3.1,
  XXVIII.3.3).

## 2. 버전 축 목록

| 축 | 기록 위치 | 현재 값 | 코드 상수 | 비교 규칙(현재 구현) |
|---|---|---|---|---|
| chunk envelope format version | `.starchunk` header | 1 | `storage::chunk::envelope::CHUNK_FORMAT_VERSION` | 정확히 일치. 아니면 `UnsupportedVersion` |
| generation manifest format version | `.starmanifest` header | 1 | `storage::generation::manifest::MANIFEST_FORMAT_VERSION` | 정확히 일치. 아니면 `UnsupportedVersion` |
| storage 문서 스키마 버전 | manifest `schema_version` | 최신 2 | `xtask::migration_fixtures::LATEST_SCHEMA` | 열기에서 비교하지 않음(§4.2) |
| chunk payload 스키마 버전 | chunk pointer `schema_version` | 문서 chunk 1 | `project::document_io::DOCUMENT_CHUNK_SCHEMA_VERSION` | 읽기에서 비교하지 않음(§4.2) |
| content hash 알고리즘 | hash 문자열 prefix | `sha256` | `storage::hash::HashAlgorithm` | 모르는 이름은 `UnknownAlgorithm` |
| chunk kind code | envelope·pointer `kind` | 초기 6종 | `storage::chunk::kinds::INITIAL_KINDS` | 읽기에서 pointer kind와 envelope kind가 같아야 함. registry 해석은 열기에서 쓰지 않음(§4.3) |
| 문서 ChunkId 유도 domain | 유도 입력 | `stellar/document-chunk-id/v1\0` | `project::document_io::serialize` | reader는 유도를 가정하지 않음 |
| `stellar.yaml` 스키마 버전 | `schema_version` | 1 | `project::manifest::CURRENT_SCHEMA_VERSION` | 형식(`parse_u32`)만 검사, 값 비교 없음 |
| `stellar.lock.yaml` 스키마 버전 | `schema_version` | 1 | `project::lock::CURRENT_LOCK_SCHEMA_VERSION` | 형식만 검사, 값 비교 없음 |
| (미연결 모델) lock 스키마 버전 | `schema_version` | 1 | `dependencies::lock::LOCK_SCHEMA_VERSION` | 정확히 1, 아니면 `UnsupportedVersion`. 열기 경로에 연결되지 않음 |
| 복구 스냅샷 버전(앱 로컬) | 첫 `u32` | 1 | `project::recovery_payload::RECOVERY_PAYLOAD_VERSION` | 정확히 일치 |
| 신뢰 목록 스키마 버전(앱 로컬) | YAML `schema_version` | 1 | `project::trust_store::TRUST_STORE_SCHEMA_VERSION` | 정확히 일치. 아니면 `Schema` 오류 |

형식 계약인 numeric registry(버전 번호는 아니지만 바꾸면 호환성이 깨짐):

| registry | 값 | 근거 |
|---|---|---|
| chunk kind code | 형식 명세 §7 표 | ADR-0010, `storage::chunk::kinds` golden test |
| tagged field tag(AssetRecord, SourceMetadata) | 형식 명세 §15 표 | `assets::record` |
| 문서 ChunkId family | 1, 2, 3 | `project::document_io::serialize` |
| 의미 모델 payload 안의 registry(NodeKind `u16`, `ColorSpace` tag, OrderKey 등) | XXVIII.1.3 범위 | `document::node_kind`, ADR-0003, ADR-0008 |

`ErrorCode`, FFI `StellarStatus` 등 ABI 값은 파일 형식이 아니므로 이 문서의 범위 밖입니다(ADR-0007,
ADR-0009).

## 3. 호환성 판정 규칙

근거: `storage::migration::compat::assess_compatibility`, 기술 아키텍처 §36, ADR-0011.

문서(세대)는 `format_version`, `schema_version`, `minimum_reader_version`, `minimum_writer_version`을
선언합니다. reader는 지원 format version `F`와 자기 스키마 버전 `A`를 가집니다. 판정은 다음 순서를
따라야 합니다.

| 순서 | 조건 | 결과 |
|---|---|---|
| 1 | `minimum_reader_version > minimum_writer_version` | 거부(`InvalidDeclaration::ReaderAboveWriter`) |
| 1 | `minimum_writer_version > schema_version` | 거부(`InvalidDeclaration::WriterAboveSchema`) |
| 2 | `format_version ≠ F` | 거부(`FormatMismatch`) |
| 3 | `A < minimum_reader_version` | 거부(`ReaderTooOld`) |
| 4 | `minimum_reader_version ≤ A < minimum_writer_version` | 읽기 전용(`ReadOnly`) |
| 5 | `A ≥ minimum_writer_version` | 편집 가능(`Editable`) |

예(테스트 `compatibility_matrix`, 문서 format 1·schema 5·읽기 최소 3·쓰기 최소 4):

| reader 지원 format | reader 스키마 | 결과 |
|---|---|---|
| 1 | 5 | 편집 가능 |
| 1 | 4 | 편집 가능 |
| 1 | 3 | 읽기 전용 |
| 1 | 2 | 거부(`ReaderTooOld { reader: 2, required: 3 }`) |
| 2 | 5 | 거부(`FormatMismatch { document: 1, supported: 2 }`) |
| 0 | 5 | 거부(`FormatMismatch { document: 1, supported: 0 }`) |

다른 테스트: 모든 값이 1이면 편집 가능(`same_version_everywhere_is_editable`), 신버전 reader(스키마 7)가
구버전 문서(schema 2, 쓰기 최소 2)를 열면 편집 가능(`newer_reader_than_schema_is_editable`).

**현재 구현**: manifest에는 `schema_version`만 있고 `minimum_reader_version`·`minimum_writer_version`이
없습니다. 열기 경로는 이 판정을 호출하지 않습니다. 이 표는 판정 함수의 계약이며, 디스크에 두 최소
버전을 기록하는 방법은 정하지 않았습니다(§8).

## 4. 축별 reader 동작 (현재 구현)

### 4.1 컨테이너 format version

| data | 기준 commit reader의 동작 |
|---|---|
| chunk format version 1 | 정상 |
| chunk format version ≠ 1 | 그 chunk 읽기가 `Envelope(UnsupportedVersion)`로 실패. `open_project_storage`는 chunk 내용을 읽지 않으므로 열기 자체는 통과하고, 읽는 시점(지연 로딩의 DocumentManifest 포함)에 실패 |
| current manifest format version ≠ 1 | current 세대 검증이 `Manifest(UnsupportedVersion)`로 실패. `open_project_storage`는 fallback으로 넘어가 format 1인 정상 세대 중 generation이 가장 큰 것을 `ReadOnlyRecovery`로 엶. 그런 세대가 없으면 `NoValidGeneration` |

즉 format version을 올린 writer가 만든 세대를 구버전 reader가 만나면, 보존된 이전 세대가 있으면 그
세대를 읽기 전용으로 엽니다. `LazyGeneration::open`은 fallback 없이 current만 엽니다.

### 4.2 스키마 버전

| data | 동작 |
|---|---|
| manifest `schema_version` = 아무 `u32` | 열기 성공. 값을 비교하지 않습니다(기준 commit에서 99로 재현) |
| 저장 | base 세대의 `schema_version`을 그대로 씁니다(`project::external_change`) |
| migration 목표 < 현재 | `NoPath`(다운그레이드 경로 없음) |
| migration 목표 > 현재, step 경로 있음 | 새 세대로 변환. 원본 불변 |
| migration 경로 끊김 | `NoPath { stuck_at, target }`, 아무것도 쓰지 않음 |
| chunk pointer `schema_version` | 읽기에서 비교하지 않습니다. migration은 모든 pointer를 도달한 manifest 스키마 버전으로 바꿉니다 |

따라서 현재 구현은 기술 아키텍처 §36의 "새 스키마를 구버전에서 편집 금지"를 강제하지 않습니다(§9).

### 4.3 chunk kind

| data | registry 규칙(`ChunkKindRegistry::resolve`) | 기준 commit의 열기 경로 |
|---|---|---|
| 등록된 kind | `Known` | 존재만 확인. DocumentManifest kind만 열 때 읽음 |
| 모르는 optional kind(bit 31 = 0) | `SkipUnknownOptional` | 존재만 확인. 내용은 요청이 없으면 읽지 않음 |
| 모르는 required kind(bit 31 = 1) | `UnknownRequiredKind` 오류 | 거부하지 않음. 존재만 확인하고 열기 성공(기준 commit에서 `0x80000099`로 재현) |

- 저장 규칙: 새 세대에는 저장 호출자가 넘긴 logical chunk만 남습니다. base에만 있는 pointer는 새
  세대에서 빠집니다(`storage::chunk::planner`). 모르는 kind의 chunk를 다음 세대로 넘기려면 호출자가
  `Clean` 상태로 다시 넘겨야 합니다.
- registry 규칙(ADR-0010)에 따르면 optional kind 추가는 구버전 reader와 호환되고, required kind 추가는
  구버전 reader가 거부해야 합니다. 현재 열기 경로는 이 거부를 하지 않습니다(§9).

### 4.4 tagged field

| data | storage decoder(`take_fields`) | AssetRecord decoder |
|---|---|---|
| 아는 tag | 반환 | 반환(mandatory flag가 정의와 다르면 거부) |
| 모르는 optional tag | 건너뜀(다시 쓰면 사라짐) | 보존(다시 쓸 때 그대로) |
| 모르는 mandatory tag | `UnknownMandatoryTag` | `UnknownMandatoryTag` |
| 정의되지 않은 flags bit | `InvalidFlags` | `InvalidFlags` |

새 optional field 추가는 구버전 reader와 호환됩니다. 새 mandatory field 추가는 구버전 reader가
거부합니다. 현재 tagged record를 쓰는 디스크 형식은 AssetRecord뿐이며, AssetRecord는 아직 chunk로
저장되지 않습니다(형식 명세 §15).

### 4.5 content hash 알고리즘

| data | 동작 |
|---|---|
| `sha256:<hex64>` | 정상 |
| 다른 prefix(`blake3:` 등), 대문자 `SHA256:` | `UnknownAlgorithm`. envelope·pointer는 `InvalidHash`, current pointer는 `InvalidFormat`, scanner·GC는 그 파일을 후보에서 제외 |

### 4.6 YAML 매니페스트와 잠금 파일

| data | 동작 |
|---|---|
| 정의되지 않은 키(어느 매핑이든) | `UnknownKey`. 프로젝트를 열지 못함(`ProbeError::ManifestSchema`) |
| `schema_version`이 엄격 `u32` 아님(`01`, `+1`, `1.0`, `yes` 등) | `InvalidSchemaVersion` |
| `schema_version` = 2 이상 | 받음(값 비교 없음) |
| `stellar-dependencies` 형식의 키(`library_id`, `local_override`, 매핑 `version`) | `UnknownKey` |

따라서 YAML에 키를 하나라도 추가하면 기준 commit의 reader는 그 프로젝트를 열지 못합니다. YAML의 스키마
버전 번호는 현재 호환성 판단에 쓰이지 않습니다.

### 4.7 앱 로컬 형식

복구 스냅샷은 버전이 정확히 같아야 하며 다르면 `UnsupportedVersion`입니다. 신뢰 목록도 `schema_version`이
정확히 1이어야 합니다. 앱 로컬 데이터는 프로젝트와 함께 이동하지 않습니다.

## 5. 변경 종류별 호환성

| 변경 | 구버전 reader(기준 commit) | 필요한 절차 |
|---|---|---|
| envelope·manifest·current pointer 구조 변경 | chunk 읽기 실패 또는 이전 세대로 읽기 전용 fallback(§4.1) | format version 상승 + 새 ADR(ADR-0010) |
| 압축 도입 | 위와 같음 | format version 상승(ADR-0010) |
| 새 hash 알고리즘 | `UnknownAlgorithm` | 새 prefix + 새 ADR + migration(ADR-0004) |
| 새 optional chunk kind | 영향 없음(존재만 확인) | kind 등록 + golden test 갱신 |
| 새 required chunk kind | 규칙상 거부해야 하나 현재는 열림(§4.3) | kind 등록 + golden test 갱신. 구버전 거부가 필요하면 §9 G2 해결이 선행 |
| 기존 kind payload의 스키마 변경 | 감지하지 않음(§4.2) | migration step 등록 + golden fixture 기대값 갱신 + ADR(ADR-0011) |
| 새 optional tagged field | 건너뛰거나(storage) 보존(AssetRecord) | 없음 |
| 새 mandatory tagged field | `UnknownMandatoryTag` | 스키마 변경으로 취급 |
| YAML 키 추가 | `UnknownKey`로 열기 실패 | 스키마 변경으로 취급. `schema_version` 비교가 없으므로(§4.6) 버전 번호만으로는 구분 불가 |
| numeric registry 값 변경·재사용 | 오독 | 금지(§7 D1) |
| chunk 분할 임계치 변경 | 영향 없음(포맷 필드 아님) | ADR-0012 supersede |

## 6. 과거 fixture 검증

기준 commit에서 실행해 확인한 항목입니다(실행 기록: `docs/task-reports/XXVIII.2.4.md`).

| 검증 | 대상 | 고정하는 호환성 |
|---|---|---|
| `cargo xtask test-migrations` | `v1-minimal`, `v1-two-views` | storage 스키마 1인 과거 fixture가 등록된 경로(noop v1→v2)로 `LATEST_SCHEMA` = 2에 도달하고 payload snapshot이 기대값과 같음 |
| `migration_git_fixture.rs` | v1 세대 | migration 뒤 Git 변화가 `design/current` 수정과 새 manifest 추가뿐 |
| `storage::chunk::envelope` `golden_header_bytes_are_fixed` | format 1 header | format version 1 envelope bytes가 바뀌지 않음 |
| `storage::generation::manifest` `golden_header_layout_is_fixed`, `golden_round_trip` | format 1 manifest | encoder 출력이 형식 명세 §10.1 순서로 직접 조립한 bytes와 같고, decode 뒤 재인코딩이 byte-identical |
| `storage::chunk::path` `golden_path_for_known_hash` | object 경로 | chunk 경로 형식이 바뀌지 않음 |
| `storage::chunk::kinds` golden test | kind code | 초기 kind code 값 고정 |
| `docs/fixtures/storage/canonical-primitives.yaml` | primitive bytes | canonical primitive 인코딩 고정 |
| `storage::migration::compat` 테스트 | §3 표 | 판정 matrix |
| `storage::migration::registry` 테스트 | step registry | gap·역방향·중복·초과 거부 |
| seed corpus `manifest_minimal.hex`, `chunk_golden_test.hex` 등 | format 1 파일 | 보안 문서 §9.1 |

과거 fixture 규칙: 새 스키마 버전을 추가할 때 과거 fixture는 그대로 두고 기대 snapshot만 최신 버전으로
갱신해야 합니다(`xtask::migration_fixtures` module 문서, ADR-0011 Migration).

## 7. deprecation 정책

현재 ADR로 확정된 규칙입니다.

| ID | 규칙 | 근거 |
|---|---|---|
| D1 | numeric 값(chunk kind code, `TypedValue`·`ColorSpace` tag, `ErrorCode`, `PropertyKey`, field tag 등)은 재사용하거나 바꾸면 안 됩니다. 새 값은 끝에 추가합니다. 바꾸려면 해당 ADR을 supersede하고 migration을 정의합니다. | ADR-0007, ADR-0008, ADR-0010 |
| D2 | envelope·manifest·경로·current pointer·ViewTree payload 형식을 바꾸려면 format version을 올리고 새 ADR을 둡니다. 지원하지 않는 format version은 거부합니다. | ADR-0010 |
| D3 | hash 알고리즘 교체는 새 prefix 등록(새 ADR)으로만 합니다. 기존 `sha256:` 값은 다시 해석하지 않고, 재해싱은 migration으로만 합니다. | ADR-0004 |
| D4 | 스키마 변경은 migration step 등록과 golden fixture 기대값 갱신으로 도입하고, 보호 경로 정책에 따라 같은 diff에 ADR을 둡니다. | ADR-0011 |
| D5 | migration은 전진만 합니다(`to > from`). 자동 다운그레이드는 없습니다. 실패하면 원본 세대를 바꾸지 않습니다. 결과는 Git commit 없이 working tree 변경으로 남깁니다. | ADR-0011, 기술 아키텍처 §36 |
| D6 | 기존 세대와 chunk 파일은 불변입니다. 형식·정책 변경은 이후 저장이 만드는 새 세대에만 반영되며 기존 세대를 다시 쓰지 않습니다. | ADR-0010, ADR-0012 Migration |
| D7 | optional chunk kind 추가는 호환 변경, required chunk kind 추가는 구버전 reader가 거부하는 비호환 변경입니다. | ADR-0010 Compatibility |
| D8 | `Typography` placeholder를 정식 구조로 바꿀 때 tag `0x0C`를 유지합니다. | ADR-0008 Migration |

## 8. 정하지 않은 사항

아래는 어떤 ADR도 정하지 않았습니다. 정하려면 새 ADR이 필요합니다.

1. `minimum_reader_version`·`minimum_writer_version`을 디스크 어디에 기록할지(ADR-0011).
2. 열기 경로가 §3 판정, manifest `schema_version`, chunk kind registry를 언제 적용할지.
3. 과거 스키마 버전 지원 기간과, 오래된 migration step을 제거할 수 있는 조건.
4. 형식 요소를 deprecated로 표시하는 방법(경고, 진단, 문서 표기).
5. 앱 release 번호와 형식·스키마 버전의 대응, 공개 형식 버전 동결(XXVIII.3.1, XXVIII.3.3).
6. `stellar.yaml`·`stellar.lock.yaml`의 `schema_version` 비교 규칙과, `stellar-dependencies`의 선언·lock
   모델(형식 명세 §14.6)과의 통합.
7. 기술 아키텍처 §19 경로 이름과 구현 경로 중 어느 쪽으로 맞출지(ADR-0010).

## 9. 기술 아키텍처 §36과 현재 구현의 차이

| ID | §36 요구 | 현재 구현 |
|---|---|---|
| G1 | 프로젝트에 네 버전 기록 | manifest에 `schema_version`만 기록 |
| G2 | 새 스키마를 구버전에서 편집 금지, 안전한 경우 읽기 전용 | 열기가 스키마 버전과 required kind를 검사하지 않아 구버전 앱이 새 스키마 세대를 편집 가능 상태로 엶 |
| G3 | 최신 앱은 이전 프로젝트를 결정론적으로 migration | migration 실행기와 fixture runner는 있으나, `run_migration`을 부르는 제품 코드가 없음(호출자는 `xtask::migration_fixtures`와 테스트뿐) |
| G4 | migration 원자적, 실패 시 원본 불변, 자동 다운그레이드·자동 commit 금지 | 충족(§4.2, §6) |
