# Stellar 프로젝트 파일 포맷 명세 — 구현 기준선

- 문서 종류: 규범(normative) 명세. 구현된 형식을 코드에서 옮겨 적었습니다.
- WBS: XXVIII.1.2
- 기준 commit: `78a574a` 위의 구현. 이 문서의 모든 규칙은 아래 근거 표기에 적은 crate·module과
  일치해야 합니다. 코드와 이 문서가 다르면 이 문서가 틀린 것이며, 문서를 고칩니다.
- 공개 상태: 내부 명세입니다. 기술 아키텍처 §35의 공개 전 조건(포맷 버전 정책 안정화, 독립 구현,
  conformance CI 등)은 아직 충족되지 않았습니다.
- 관련 문서: 보안 한도와 malformed 입력 동작은 [`stellar-format-security.md`](stellar-format-security.md)
  (XXVIII.1.4), 버전 호환성 matrix와 deprecation 정책은
  [`stellar-format-compatibility.md`](stellar-format-compatibility.md)(XXVIII.2.4)에 있습니다.

## 0. 표기

규범 용어:

| 표기 | 뜻 |
|---|---|
| **해야 합니다** | 반드시 지킵니다. 어기면 형식 위반입니다. |
| **하면 안 됩니다** | 금지입니다. |
| **할 수 있습니다** | 허용되지만 필수가 아닙니다. |
| **현재 구현** | 규칙이 아니라 기준 commit의 관찰된 동작입니다. 바뀔 수 있습니다. |

근거 표기 `storage::chunk::envelope`는 crate `stellar-storage`의 module `chunk::envelope`
(`crates/storage/src/chunk/envelope.rs`)를 뜻합니다. crate 접두어는 `core-types`
(`stellar-core-types`), `storage`, `project`(`stellar-project`), `document`(`stellar-document`),
`assets`(`stellar-assets`), `dependencies`(`stellar-dependencies`), `xtask`(`tools/xtask`)입니다.

바이트 열은 소문자 16진수로 적고, `‖`는 이어 붙이기입니다. "LE"는 little-endian, "BE"는
big-endian입니다.

## 1. 범위

이 명세가 다루는 것:

1. 프로젝트 폴더 안의 저장 파일 배치(§2).
2. canonical binary 인코딩(§3)과 content hash(§4).
3. 식별자: `StableId`, `ChunkId`, 문서 ChunkId 유도, `AssetId`(§5).
4. chunk 파일(`.starchunk`)의 envelope, 경로, 쓰기·읽기 규칙(§6), chunk kind(§7), chunk pointer(§8).
5. storage·project가 정의한 chunk payload: ViewTree, DocumentManifest, 문서 chunk model(§9).
6. generation manifest(`.starmanifest`)와 current pointer(§10, §11), 열기·저장 절차(§12, §13).
7. 제한 YAML과 `stellar.yaml`·`stellar.lock.yaml` 스키마(§14).
8. unknown field·unknown 요소 규칙(§15).
9. 버전과 migration 규칙(§16).

다루지 않는 것:

- 문서 의미 모델 payload의 내부 형식(`document::definitions`의 `DocumentDefinition`,
  `document::record_codec`의 node 레코드·`SubtreeRecords`, `document::modifier`의 `ModifierStack`).
  semantic model 명세(XXVIII.1.3)의 범위입니다. 이 문서는 바깥 틀(§9)만 정합니다.
- Semantic Diff·Merge 규칙(XXVIII.1.3).
- 에셋 content 파일(`assets/`)의 내용. 에셋 파일은 원본 bytes 그대로입니다(§2).
- 프로젝트 밖에 저장되는 형식: 복구 스냅샷(`project::recovery_payload`), 신뢰 목록
  (`project::trust_store`). 버전 목록만 호환성 문서에 있습니다.
- 성능 정책(chunk 분할 임계치 등). 포맷 필드가 아닙니다(ADR-0012).

## 2. 프로젝트 폴더 배치

경로는 프로젝트 root 기준이며 구분자는 `/`로 적습니다. 디스크에서는 플랫폼 구분자를 씁니다.

| 경로 | 내용 | 형식 절 | 근거 |
|---|---|---|---|
| `stellar.yaml` | 프로젝트 매니페스트 | §14.5 | `project::manifest`, `project::probe` |
| `stellar.lock.yaml` | 의존성 잠금 파일 | §14.6 | `project::lock` |
| `design/current` | current pointer(manifest hash 한 줄) | §11 | `storage::generation::current` |
| `design/generations/<alg>-<hex64>.starmanifest` | generation manifest | §10 | `storage::generation::loader` |
| `design/objects/<xx>/<alg>-<hex64>.starchunk` | chunk 파일 | §6 | `storage::chunk::path` |
| `assets/<xx>/<alg>-<hex64>.<ext>` | 에셋 원본 bytes | 범위 밖 | `assets::path` |
| `.gitignore`, `.gitattributes` | 새 프로젝트 기본 내용 | — | `project::scaffold` |

- `<alg>`는 hash 알고리즘 이름(현재 `sha256`), `<hex64>`는 digest의 소문자 16진수 64자,
  `<xx>`는 `<hex64>`의 앞 2자입니다. 파일 이름에서는 hash 문자열의 `:`를 `-`로 바꿉니다
  (ADR-0010: `:`는 Windows 파일 이름 금지 문자).
- 에셋 확장자 `<ext>`는 `[a-z0-9]` 1~16자이고, 규칙을 벗어나면 `bin`입니다(`assets::path`).
- atomic 쓰기의 임시 파일은 대상과 같은 디렉터리의 `.<대상 파일 이름>.tmp-<pid>-<counter>`입니다
  (`storage::atomic_file`). 이 이름은 content-addressed 이름이 아니므로 reader와 GC는 무시합니다.
- 새 프로젝트 scaffold는 `stellar.yaml`, `stellar.lock.yaml`, `.gitignore`, `.gitattributes`와 빈
  `design/`·`assets/`를 만듭니다. 첫 generation은 만들지 않습니다(`project::scaffold`).

기술 아키텍처 §19와 다른 점(현재 구현이 기준):

| 아키텍처 §19 | 구현 |
|---|---|
| `design/current.stellarref` | `design/current` |
| `design/generations/<manifest-hash>.stargen` | `design/generations/<alg>-<hex64>.starmanifest` |
| `design/objects/sha256/<prefix>/<content-hash>.starchunk` | `design/objects/<prefix>/<alg>-<hex64>.starchunk` |
| `assets/sha256/<prefix>/...` | `assets/<prefix>/<alg>-<hex64>.<ext>` |

ADR-0010은 이 차이를 기록만 하고 어느 쪽으로 맞출지 정하지 않았습니다.

## 3. canonical binary 인코딩

근거: `storage::canonical::encode`, `storage::canonical::decode`, 규칙 문서
`docs/architecture/storage-canonical-primitives.md`(R1~R6), golden fixture
`docs/fixtures/storage/canonical-primitives.yaml`, ADR-0010.

canonical 인코딩은 primitive 층입니다. 필드의 순서와 종류는 각 컨테이너 형식(§6~§11)이 정합니다.
같은 논리 값은 어느 플랫폼에서든 같은 bytes가 되어야 합니다.

### 3.1 정수와 bool (R1, R2)

| 타입 | 폭(byte) | 표현 |
|---|---|---|
| `u8`, `i8` | 1 | 그대로(부호는 2의 보수) |
| `u16`, `i16` | 2 | LE |
| `u32`, `i32` | 4 | LE |
| `u64`, `i64` | 8 | LE |
| `bool` | 1 | `00` = false, `01` = true |

- 가변 길이 정수(varint)는 쓰지 않습니다.
- decoder는 `bool` 자리의 `00`·`01`이 아닌 byte를 `InvalidBool`로 거부해야 합니다.

예(golden fixture): `u16` 258 → `0201`, `i32` −256 → `00ffffff`, `bool` true → `01`.

### 3.2 float (R5)

- `f32`는 IEEE-754 binary32 비트를 LE 4 byte로, `f64`는 binary64 비트를 LE 8 byte로 씁니다.
- encoder는 NaN과 ±Infinity를 `NonFiniteFloat`으로 거부하고, `-0.0`을 `+0.0`으로 바꿔 써야 합니다.
  `f32` 규칙은 `core-types::value::FiniteF32`(ADR-0008)를 그대로 쓰고 `f64`도 같은 규칙입니다.
- decoder는 NaN·±Infinity·`-0.0`의 비트 패턴을 고치지 않고 `InvalidFloat`으로 거부해야 합니다.
  subnormal을 포함한 나머지 유한 값은 받습니다.

예(golden fixture): `f32` 1.0 → `0000803f`, `f64` −0.0 → `0000000000000000`(정규화).

### 3.3 길이 프리픽스 bytes와 문자열 (R3)

```text
bytes  = len:u32 LE ‖ payload[len]
string = bytes, payload는 UTF-8
```

- 길이가 `u32` 범위를 넘으면 encoder는 `LengthOverflow`로 거부합니다.
- decoder는 길이가 남은 입력보다 크면 `LengthOverflow`, 문자열 payload가 UTF-8이 아니면
  `InvalidUtf8`으로 거부해야 합니다.
- 유니코드 정규화는 이 층이 하지 않습니다(상위 계층 책임, ADR-0008 `NormalizedText`).

예(golden fixture): 빈 문자열 → `00000000`, `"é"` → `02000000c3a9`.

### 3.4 고정 폭 raw 필드

magic(8 byte)과 16 byte 식별자는 길이 프리픽스 없이 그대로 씁니다(`put_fixed_raw`/`take_raw`).
`StableId`·`ChunkId`의 16 byte는 정수가 아니므로 R1의 예외로 UUID byte 순서(BE) 그대로 씁니다
(ADR-0010).

### 3.5 map (R4)

```text
map   = count:u32 LE ‖ entry[count]
entry = key:bytes ‖ value:bytes
```

- entry는 key bytes의 사전식 순서(byte 단위 비교, 짧은 접두가 먼저)로 **엄격히 증가**해야 합니다.
- encoder는 입력 순서와 무관하게 정렬해 쓰고, 같은 key가 두 번 있으면 `DuplicateKey`로 거부합니다.
- decoder는 같은 key를 `DuplicateKey`, 역순 key를 `MisorderedKey`로 거부해야 합니다.
- value는 호출자가 미리 canonical 인코딩한 bytes이며, 해석은 호출자가 합니다.

### 3.6 tagged field 레코드 (R6 wire 형식)

```text
record = count:u32 LE ‖ field[count]
field  = tag:u32 LE ‖ flags:u8 ‖ payload:bytes
flags  = bit0 mandatory, bit1~7 = 0
```

- field는 tag 오름차순으로 **엄격히 증가**해야 합니다. encoder는 정렬해 쓰고 중복 tag를
  `DuplicateTag`로 거부합니다.
- decoder는 정의되지 않은 flags bit를 `InvalidFlags`, 같은 tag를 `DuplicateTag`, 역순 tag를
  `MisorderedTag`로 거부해야 합니다. 순서 검사는 모르는 tag에도 적용합니다.
- 모르는 tag의 처리(R6)는 §15에서 정합니다.

### 3.7 decoder 공통 요구

- 입력이 부족하면 `UnexpectedEof`로 거부해야 합니다.
- 컨테이너 형식이 끝을 정하는 경우(§9.1, §9.2, §10) 값을 다 읽은 뒤 남은 bytes를
  `TrailingBytes`로 거부해야 합니다.
- malformed 입력에서 panic하면 안 됩니다. 자세한 한도는 보안 문서 §3에 있습니다.

## 4. content hash

근거: `storage::hash`, ADR-0004, ADR-0015.

```text
content-hash = "sha256:" ‖ hex64
hex64        = 소문자 16진수 [0-9a-f] 64자 (SHA-256 digest 32 byte)
```

- 알고리즘은 SHA-256(FIPS 180-4)입니다. 문자열은 정확히 71자입니다.
- parse 규칙(순서대로):
  1. 첫 `:`가 없으면 `MissingSeparator`.
  2. `:` 앞이 정확히 `sha256`이 아니면 `UnknownAlgorithm`(대소문자 구분: `SHA256`도 거부).
  3. `:` 뒤의 **byte 길이**가 64가 아니면 `InvalidLength`.
  4. `:` 뒤의 모든 문자가 `[0-9a-f]`가 아니면 첫 위반 문자로 `InvalidHexDigit`(대문자 거부).
- hash 대상:
  - chunk: payload bytes만(§6). envelope header는 hash에 들어가지 않습니다.
  - generation manifest: 인코딩된 manifest bytes 전체(§10).
- 알 수 없는 알고리즘 이름은 거부하므로, 새 알고리즘은 새 ADR과 새 prefix로만 들어옵니다(ADR-0004).

## 5. 식별자

### 5.1 StableId

근거: `core-types::ids`, ADR-0007, 기술 아키텍처 §12.1.

- binary: 16 byte. 비교·정렬은 byte 사전식입니다. UUIDv7이면 이 순서가 timestamp 순서입니다.
- 문자열: `8-4-4-4-12` 형태의 hyphen UUID 36자입니다. 출력은 소문자여야 합니다. parse는 대소문자를
  모두 받습니다. 길이가 36이 아니면 `ParseStableIdError::Length`, hyphen 위치나 16진수가 틀리면
  `Format`입니다.
- 생성: `StableId::new_v7(unix_ts_ms, rand_a, rand_b)`는 RFC 9562 UUIDv7 레이아웃을 만듭니다.

  | bit 위치(상위부터) | 내용 |
  |---|---|
  | 0~47 | `unix_ts_ms` 하위 48 bit (BE) |
  | 48~51 | version `0111` |
  | 52~63 | `rand_a` 하위 12 bit |
  | 64~65 | variant `10` |
  | 66~127 | `rand_b` 하위 62 bit |

- core-types는 시계와 엔트로피를 읽지 않습니다. 시각과 seed는 상위 계층이 주입합니다
  (`StableIdGenerator`).
- **현재 구현**: binary decode(`from_bytes`)와 문자열 parse는 version·variant bit를 검사하지 않습니다.
  모든 128 bit 값(nil 포함, UUIDv4 형태 포함)을 받습니다. reader는 version으로 거부하면 안 됩니다.

### 5.2 ChunkId

근거: `storage::chunk::id`, ADR-0010.

- `ChunkId`는 `StableId`를 감싼 논리 chunk 식별자입니다. binary(16 byte)·문자열 형식은 `StableId`와
  같습니다. 내용이 바뀌어도 유지됩니다.
- 타입 수준에서 원시 `StableId`와 섞이지 않습니다(compile_fail doctest).

### 5.3 문서 ChunkId 유도

근거: `project::document_io::serialize`(`derive_chunk_id`, `manifest_chunk_id`, `view_tree_chunk_id`,
`subtree_chunk_id`).

문서 chunk의 ChunkId는 다음 값으로 만들어야 합니다.

```text
input  = "stellar/document-chunk-id/v1" ‖ 00 ‖ family:u8 ‖ document:StableId(16) ‖ object:StableId(16)
digest = SHA-256(input)
id     = digest[0..16]
id[6]  = (id[6] & 0x0f) | 0x80    # version 8
id[8]  = (id[8] & 0x3f) | 0x80    # variant 10
```

| family | 값 | object |
|---|---|---|
| DocumentManifest | `1` | 문서 StableId(document와 같음) |
| ViewTree | `2` | View StableId |
| subtree | `3` | subtree root node StableId |

- domain 문자열과 family 값은 저장 형식의 일부입니다. 바꾸면 모든 문서 ChunkId가 바뀝니다.
- version nibble 8이라 UUIDv7 StableId를 그대로 쓰는 다른 ChunkId와 같은 값이 될 수 없습니다.
- reader는 이 유도 규칙을 가정하면 안 됩니다. 저장된 참조(manifest → ViewTree → subtree)만
  따라갑니다(`project::document_io::load`).

### 5.4 AssetId와 project id

- `AssetId`는 `StableId`와 같은 16 byte 표현과 같은 문자열 형식을 씁니다(`assets::id`).
- `stellar.yaml`의 `project.id` 문자열은 `project::identity::parse_project_id`가 `StableId`로 바꿉니다.
  36자, 8·13·18·23번째가 `-`, 나머지가 16진수(대소문자 무관)여야 합니다. 형식이 틀리면 `None`입니다.
- generation manifest의 project id는 열기 시 호출자가 넘긴 기대 project id와 같아야 합니다(§12).

## 6. chunk 파일 (`.starchunk`)

근거: `storage::chunk::envelope`, `storage::chunk::path`, `storage::chunk::store`, ADR-0010.

### 6.1 envelope header (format version 1)

| offset | 필드 | 형식 |
|---|---|---|
| 0 | magic | raw 8 byte `STARCHNK`(`5354415243484e4b`) |
| 8 | format version | `u32` LE, `1` |
| 12 | kind | `u32` LE kind code(§7) |
| 16 | uncompressed length | `u64` LE, payload byte 수 |
| 24 | payload hash | `string`(§3.3), content hash 문자열(§4) |

chunk 파일은 `header ‖ payload`입니다. SHA-256이면 header는 99 byte(24 + 4 + 71)입니다.
압축 필드는 없습니다. 압축을 도입하면 format version을 올려야 합니다(ADR-0010).

golden header(kind `ChunkKindCode(1)`, payload `test`; `storage::chunk::envelope` 테스트
`golden_header_bytes_are_fixed`):

```text
5354415243484e4b 01000000 01000000 0400000000000000 47000000
7368613235363a39663836643038313838346337643635396132666561613063
353561643031356133626634663162326230623832326364313564366331356230663030613038
```

header decode 오류: magic 불일치 `BadMagic`, format version이 1이 아니면 `UnsupportedVersion`, hash
문자열 형식 위반 `InvalidHash`, canonical 오류는 `Decode(...)`입니다.

### 6.2 object 경로

```text
design/objects/<hex64 앞 2자>/<alg>-<hex64>.starchunk
```

경로는 `ContentHash` 값에서만 만들어야 합니다. 임의 문자열에서 경로를 만들면 안 됩니다
(`ChunkObjectPath::for_hash`).

### 6.3 쓰기 규칙

chunk 파일은 불변입니다(`ChunkStore::write_chunk`).

1. `payload`에서 envelope를 만들고 파일 bytes를 인코딩합니다.
2. 대상 파일이 이미 있으면 전체 bytes를 비교합니다. 같으면 재사용(`Reused`)합니다. 다르면
   `CorruptExisting`으로 실패하고 덮어쓰면 안 됩니다.
3. 대상 파일이 없으면 부모 디렉터리를 만들고 atomic 쓰기(§13.3)로 기록합니다(`Written`).

### 6.4 읽기 검증

reader는 `(요청 hash, 기대 kind)`로 chunk를 읽고 다음을 순서대로 모두 검증해야 합니다
(`ChunkStore::read_chunk`). 기대 kind는 chunk pointer(§8)가 압니다.

| 순서 | 검사 | 실패 |
|---|---|---|
| 1 | 경로의 파일이 있음 | `MissingChunk` |
| 2 | envelope header(§6.1) decode | `Envelope(...)` |
| 3 | header kind = 기대 kind | `KindMismatch` |
| 4 | header 뒤 남은 bytes 수 = uncompressed length | `LengthMismatch` |
| 5 | header payload hash = 요청 hash | `AddressMismatch` |
| 6 | SHA-256(payload) = header payload hash | `PayloadHashMismatch` |

읽기는 파일을 고치거나 지우면 안 됩니다.

## 7. chunk kind

근거: `storage::chunk::kind_registry`, `storage::chunk::kinds`, ADR-0010.

- kind code는 `u32`입니다. 최상위 bit(bit 31, `0x80000000`)가 1이면 **required**, 0이면
  **optional**입니다. 모르는 kind라도 code만 보고 처리 방식을 정할 수 있습니다.
- 등록된 초기 kind(V.6.1):

| 이름 | code | required |
|---|---|---|
| `project-metadata` | `0x80000001` | 예 |
| `document-manifest` | `0x80000002` | 예 |
| `view-tree` | `0x80000003` | 예 |
| `component-library` | `0x80000004` | 예 |
| `token-set` | `0x80000005` | 예 |
| `asset-metadata` | `0x00000006` | 아니오 |

- registry 해석(`ChunkKindRegistry::resolve`): 등록된 code는 `Known`, 모르는 optional code는
  `SkipUnknownOptional`, 모르는 required code는 `UnknownRequiredKind` 오류입니다. code·이름 중복
  등록은 거부합니다.
- 현재 구현에서 payload 형식이 정해진 kind는 `document-manifest`(§9.2)와 `view-tree`(§9.1,
  §9.3)뿐입니다. 나머지 kind의 payload는 정하지 않았습니다(ADR-0010).
- **현재 구현**: 열기 경로(§12)는 registry를 호출하지 않습니다. manifest에 모르는 required kind
  pointer가 있어도 열기는 성공합니다. 자세한 내용은 호환성 문서 §4.3에 있습니다.

## 8. chunk pointer

근거: `storage::chunk::pointer`, `storage::chunk::remap`, ADR-0010.

### 8.1 형식

```text
pointer = chunk_id:raw16 ‖ content_hash:string ‖ kind:u32 LE ‖ schema_version:u32 LE
```

- SHA-256이면 pointer 하나는 99 byte(16 + 4 + 71 + 4 + 4)입니다.
- `schema_version`은 payload 스키마 버전입니다. payload 안에는 스키마 버전을 두지 않습니다
  (ViewTree, ADR-0010).
- pointer의 순서는 `(chunk_id, content_hash, kind, schema_version)` 사전식입니다. content hash의
  순서는 `(알고리즘, digest bytes)`입니다.

### 8.2 pointer 집합 규칙

- 한 manifest에서 ChunkId는 정확히 한 번 나와야 합니다. 같은 ChunkId가 다른 hash를 가리키면
  `ConflictingChunkId`, 같은 항목이 반복되면 `DuplicateChunkId`입니다(`validate_pointer_set`).
- 서로 다른 ChunkId가 같은 content hash를 가리키는 것(dedup)은 허용합니다.

### 8.3 remap manifest

- `ChunkRemapManifest`는 split·merge로 옮겨진 `from → to` ChunkId 재매핑입니다. 자기 재매핑과 중복
  source는 등록 시 거부합니다. chain은 허용하지만 cycle은 거부하고, chain의 최종 대상은 살아 있는
  ChunkId여야 합니다.
- **현재 구현**: remap manifest는 메모리 타입이며 디스크 인코딩이 없습니다. 이 명세는 파일 형식을
  정하지 않습니다.

## 9. chunk payload 형식

### 9.1 ViewTree payload

근거: `storage::chunk::view_tree`.

```text
view_tree = view_root:raw16 ‖ count:u32 LE ‖ (subtree_root:raw16 ‖ subtree_chunk:raw16)[count]
```

- `view_root`는 View root node StableId, 각 항목은 subtree root node StableId와 subtree 내용 chunk의
  ChunkId입니다. 항목 순서는 의미가 있습니다(자식 순서).
- subtree root는 한 payload 안에서 중복되면 안 됩니다(`DuplicateSubtreeRoot`). encoder와 decoder가
  모두 검사합니다.
- decoder는 끝까지 정확히 소비해야 합니다(`TrailingBytes`).

### 9.2 DocumentManifest payload

근거: `project::document_io::manifest`.

```text
document_manifest = definition:bytes ‖ count:u32 LE ‖ (view:raw16 ‖ view_tree_chunk:raw16)[count]
```

- `definition`은 `document::definitions::DocumentDefinition::encode` 결과를 길이 프리픽스로 감싼
  것입니다. 내부 형식은 이 명세의 범위 밖입니다(§1).
- View 항목은 정의의 View 순서(page 순서 → page 안 View 순서)와 개수·순서가 정확히 같아야 합니다.
  다르면 `ViewMismatch { index }`입니다. encoder와 decoder가 모두 검사합니다.
- decoder는 끝까지 정확히 소비해야 합니다.

### 9.3 문서 chunk model

근거: `project::document_io`, `project::document_io::serialize`, `project::document_io::load`.

| chunk | kind | ChunkId(§5.3) | payload |
|---|---|---|---|
| DocumentManifest | `0x80000002` | family 1 | §9.2 |
| ViewTree | `0x80000003` | family 2 | §9.1 |
| subtree | `0x80000003` | family 3 | `document::record_codec::SubtreeRecords` |

- 세 chunk 모두 pointer `schema_version`은 `DOCUMENT_CHUNK_SCHEMA_VERSION` = `1`입니다.
- subtree chunk는 별도 kind 없이 ViewTree kind를 씁니다. 어떤 payload로 해석할지는 참조 경로가
  정합니다.
- **현재 구현**: serializer는 View마다 ViewTree 하나와 subtree 하나(subtree root = View root)를
  만듭니다.
- reader(`load_document`)는 다음을 검사해야 합니다.
  1. ViewTree의 `view_root`가 정의에 있는 그 View의 root frame과 같음(`ViewRootMismatch`).
  2. subtree payload의 root가 ViewTree 항목의 subtree root와 같음(`SubtreeRootMismatch`).
  3. 합친 레코드가 문서 불변식을 지킴(`DocumentModel::from_records`, 문서 validator).

## 10. generation manifest (`.starmanifest`)

근거: `storage::generation::manifest`, `storage::generation::loader`, ADR-0010.

### 10.1 형식 (format version 1)

| 필드 | 형식 |
|---|---|
| magic | raw 8 byte `STARMFST`(`535441524d465354`) |
| format version | `u32` LE, `1` |
| generation | `u64` LE |
| parent flag | `bool` |
| parent generation | flag가 true일 때만 `u64` LE |
| project id | raw 16 byte StableId |
| schema version | `u32` LE |
| pointer count | `u32` LE |
| pointers | pointer(§8.1) × count |
| root document count | `u32` LE |
| root document ids | raw 16 byte StableId × count |

### 10.2 구조 규칙

- `generation`은 1 이상이어야 합니다(`ZeroGeneration`).
- parent가 있으면 `parent < generation`이어야 합니다(`InvalidParent`).
- pointer는 pointer 순서(§8.1)로 **엄격히 증가**해야 합니다. encoder는 정렬해 쓰고, decoder는 위반을
  `MisorderedPointers`로 거부합니다. 완전히 같은 pointer가 반복되면 순서 검사에서
  `MisorderedPointers`가 되고, 같은 ChunkId가 다른 hash로 오름차순이면 pointer 집합 검사(§8.2)에서
  `PointerSet(ConflictingChunkId)`가 됩니다.
- root document id는 입력 순서를 보존하고 중복되면 안 됩니다(`DuplicateRootDocument`).
- magic 불일치 `BadMagic`, format version이 1이 아니면 `UnsupportedVersion`입니다.
- decoder는 끝까지 정확히 소비해야 하며, decode 뒤에도 구조 규칙을 다시 검증합니다. 같은 논리
  manifest는 pointer 입력 순서와 무관하게 같은 bytes가 됩니다.

### 10.3 파일과 hash

- manifest hash는 인코딩된 bytes 전체의 content hash(§4)입니다.
- 파일 경로는 `design/generations/<alg>-<hex64>.starmanifest`입니다.
- 쓰기는 chunk와 같은 불변 규칙(§6.3)을 따릅니다. 같은 이름의 기존 파일이 다르면
  `CorruptExistingManifest`입니다.
- 읽기는 파일이 없으면 `MissingManifest`, 파일 bytes의 hash가 이름의 hash와 다르면
  `ManifestHashMismatch`, 그 다음 decode·검증합니다.

## 11. current pointer (`design/current`)

근거: `storage::generation::current`, ADR-0010.

```text
design/current = content-hash ‖ 0a
```

- 파일은 current generation manifest의 hash 문자열 하나와 LF 하나만 담아야 합니다. SHA-256이면
  72 byte입니다.
- reader 규칙: 파일이 없으면 `Missing`, 마지막 byte가 LF가 아니면 `InvalidFormat`, LF가 더 있으면
  `InvalidFormat`, hash parse 실패도 `InvalidFormat`입니다. 따라서 CRLF(`…\r\n`)는 hex 길이 65로
  거부됩니다. UTF-8이 아닌 내용은 `Io`(읽기 실패)입니다.
- writer는 atomic 쓰기(§13.3)로만 교체해야 합니다. 어느 시점에 읽어도 이전 hash나 새 hash 중 하나만
  보입니다.

## 12. 열기 절차 (reader 요구)

근거: `storage::generation::loader`, `storage::generation::scanner`, `storage::generation::open`,
`storage::generation::lazy`.

1. **current 세대 검증**(`load_current_generation`): current pointer(§11) → manifest 읽기·hash 일치·
   decode(§10) → manifest project id = 기대 project id(`WrongProjectId`) → 모든 pointer의 chunk 파일이
   일반 파일로 존재(`MissingChunk`). 이 단계는 chunk 내용을 읽지 않습니다.
2. **fallback**(`open_project_storage`): 1단계가 어떤 이유로든 실패하면 `design/generations/`의 직속
   항목 중 이름이 `<alg>-<hex64>.starmanifest` 형식인 파일을 hash 순서로 정렬해 하나씩 1단계와 같은
   방식으로 검증합니다. 통과한 세대 중 generation이 가장 큰 것을 고릅니다. 같은 generation이 여럿이면
   hash 순서에서 먼저 나온 것입니다. 결과는 `ReadOnlyRecovery`이며 current pointer를 바꾸지 않습니다.
   통과한 세대가 없으면 `NoValidGeneration`, 디렉터리를 읽지 못하면 `Scan`입니다. 디렉터리가 없으면
   후보 없음으로 봅니다.
3. **지연 로딩**(`LazyGeneration::open`): current 세대 검증 뒤 kind가 `document-manifest`인 chunk만
   §6.4 전체 검증으로 읽습니다. 다른 chunk는 요청할 때 같은 검증으로 읽습니다.

열기는 어떤 경로에서도 디스크에 쓰면 안 됩니다. 손상된 파일을 고치면 안 됩니다.

## 13. 저장 절차 (writer 요구)

근거: `storage::save::{plan, optimistic, executor, commit, cancel}`, `storage::chunk::planner`,
`storage::atomic_file`, ADR-0011.

### 13.1 저장 계획

- target generation은 base generation + 1이고, 첫 저장은 1입니다. parent는 base generation입니다.
- 변경되지 않은 chunk는 base pointer를 그대로 씁니다. 변경 표시된 chunk라도 payload hash가 base와
  같으면 다시 쓰지 않습니다. 결과 pointer는 정렬되고 §8.2를 만족해야 합니다.
- **현재 구현**: 제품 저장 경로(`project::external_change`)는 manifest `schema_version`을 base 세대
  값 그대로 씁니다. 제품 코드에 "현재 storage 스키마 버전" 상수는 없습니다(§16.1).

### 13.2 단계 순서

1. optimistic base check: current pointer가 계획의 base hash와 정확히 같아야 합니다. 첫 저장이면
   current pointer 파일이 없어야 합니다. 다르면 `StaleBase`이고 아무것도 쓰지 않습니다.
2. 변경 chunk 기록(§6.3).
3. 기록한 chunk를 다시 읽어 §6.4 전체 검증.
4. 새 manifest 기록(§10.3).
5. current pointer 교체(§11). 이 단계가 current 세대를 바꾸는 유일한 단계입니다.

취소 token은 1~4단계의 각 실행 단위(base check, chunk 하나의 기록, chunk 하나의 검증, manifest 기록)
직전과 5단계 직전에 확인하며, 5단계에 들어간 뒤에는 확인하지 않습니다(`storage::save::cancel`).
5단계 전에 실패하면 디스크의 변화는 새 파일 추가뿐이므로 이전 current 세대가 그대로 열립니다.

### 13.3 atomic 쓰기

대상과 같은 디렉터리에 임시 파일 생성 → 기록 → flush → `sync_all` → 임시 파일을 대상 이름으로
rename → 부모 디렉터리 sync(POSIX만) 순서입니다. rename이 대상 경로를 바꾸는 유일한 단계입니다.

## 14. 제한 YAML과 매니페스트

### 14.1 입력

근거: `project::yaml::loader`.

- 입력은 UTF-8 문자열입니다. 크기 상한과 문자 집합(YAML 1.2 `c-printable`) 검사를 파싱보다 먼저
  합니다. 수치 한도는 보안 문서 §4에 있습니다.
- **현재 구현**: 맨 앞의 BOM(U+FEFF)은 `c-printable`이라 허용되지만 제거되지 않고 첫 스칼라의
  일부가 됩니다(예: 키 `"\u{feff}schema_version"`). 그래서 BOM으로 시작하는 `stellar.yaml`은
  스키마 단계에서 `UnknownKey`로 거부됩니다. writer는 BOM을 쓰지 않습니다.
- CRLF 줄바꿈은 읽을 수 있습니다. writer는 LF만 씁니다.

### 14.2 허용 구조와 거부 구조

document 하나의 매핑·시퀀스·스칼라만 허용합니다. root가 스칼라여도 loader는 받지만, 매니페스트와
잠금 파일 스키마는 root가 매핑이어야 합니다.

| 입력 | 결과(오류 코드) |
|---|---|
| anchor(`&a`) | `anchor` |
| alias(`*a`) | `alias` |
| 모든 tag(`!!str`, `!<...>`, `!custom` 포함) | `custom-tag` |
| 매핑 키 `<<` | `merge-key` |
| 매핑·시퀀스 키 | `non-scalar-key` |
| 같은 매핑 안의 같은 키(인용 여부 무관) | `duplicate-key` |
| document 두 개 이상(`---`, `...` 뒤 새 document) | `multiple-documents` |
| document 없음(빈 입력, 주석만) | `empty-document` |
| 하위 parser 문법 오류 | `parse` |

- 중복 키는 마지막 값으로 덮어쓰지 않고 거부합니다. 다른 매핑의 같은 키는 허용합니다.
- 값이나 시퀀스 항목으로 나온 `<<`는 문자열로 허용합니다.
- 선행 `---`, `%YAML`·`%TAG` directive는 document가 하나면 허용합니다.
- 오류는 1-기반 줄·열과 위 고유 코드를 가집니다(`LoaderError`). 크기·문자·깊이·선읽기 한도 오류
  코드(`input-too-large`, `non-printable`, `nesting-too-deep`, `lookahead-too-large`)는 보안 문서 §4에
  있습니다.

### 14.3 스칼라 해석

근거: `project::yaml::loader`, `project::yaml::scalar`.

- loader는 모든 스칼라를 문자열로 보존합니다. 암시적 타입 추론을 하지 않습니다. `~`, `null`, 빈 값,
  `"123"`은 각각 문자열 `~`, `null`, 빈 문자열, `123`입니다.
- 타입이 필요한 필드는 엄격 해석기만 써야 합니다.
  - 불리언(`parse_bool`): 정확히 `true`·`false`만. `yes`·`on`·`True` 등은 불리언이 아닙니다.
  - 음이 아닌 정수(`parse_u32`): `[0-9]+`, 선행 0 금지(`0` 자체는 허용), `u32` 범위. 부호·진법
    접두·`_`·소수점·공백은 거부합니다.
- 날짜·timestamp는 문자열입니다.

### 14.4 canonical writer

근거: `project::yaml::writer`.

- 키 순서는 값 트리의 저장 순서를 그대로 씁니다(재정렬 없음). 들여쓰기는 단계마다 공백 2칸, 줄바꿈은
  LF, 파일은 LF 하나로 끝납니다. 주석은 쓰지 않습니다.
- 매핑 값·시퀀스 항목은 `key: value`, `- item` 형태이며, 빈 매핑은 `{}`, 빈 시퀀스는 `[]`입니다.
- 스칼라 인용: 영문자로 시작하고 영문자·숫자·`_`·`-`로만 된 문자열 중 `true`·`false`·`null`·`yes`·
  `no`·`on`·`off`·`y`·`n`(대소문자 무시)이 아닌 것만 따옴표 없이 씁니다. 나머지는 큰따옴표로 감싸고
  `\\`, `\"`, `\n`, `\t`, `\r`을 이스케이프합니다. C0·DEL·C1 제어 문자, U+2028, U+2029, U+FEFF,
  U+FFFE, U+FFFF는 `\uXXXX`(소문자 hex 4자리)로 씁니다.
- 렌더링한 키가 1024자를 넘으면 explicit key(`? key` 다음 줄 `: value`)로 씁니다.
- 파일로 쓰는 경로는 `write_canonical_checked`를 써야 합니다. 이 함수는 비스칼라 키·`<<` 키·중복 키·
  깊이 초과·크기 초과를 거부하고, 출력을 loader로 다시 읽어 같은 값인지 확인합니다
  (`NotReloadable`, `RoundTripMismatch`).

### 14.5 `stellar.yaml`

근거: `project::manifest`, `project::probe`. 열기 경로(`probe_project`)는 root 검증 → 파일 읽기 →
제한 loader → `Manifest::from_yaml` 순서입니다.

| 경로 | 필수 | 형식 |
|---|---|---|
| `schema_version` | 예 | 스칼라, `parse_u32` |
| `project` | 예 | 매핑, 키 `id`·`name`만 |
| `project.id` | 예 | 스칼라 |
| `project.name` | 예 | 스칼라 |
| `target_profiles` | 예 | 시퀀스 |
| `target_profiles[i]` | — | 매핑, 키 `id`(필수)·`platform`(필수)·`minimum_os`·`baseline` |
| `dependencies` | 예 | 매핑, 키 = 의존성 이름(스칼라) |
| `dependencies.<name>` | — | 매핑, 키 `source`·`repository`·`version` 모두 필수, 값은 스칼라 |

- 모든 매핑에서 정의되지 않은 키는 `UnknownKey`입니다. 필수 키 누락은 `MissingField`, 종류 불일치는
  `WrongType`, 버전 형식 위반은 `InvalidSchemaVersion`입니다.
- 키는 어떤 순서로 와도 됩니다. writer는 `schema_version`, `project`(`id`, `name`), `target_profiles`
  (`id`, `platform`, `minimum_os`, `baseline`), `dependencies`(이름: `source`, `repository`, `version`)
  순서로 씁니다. 숫자로 시작하는 스칼라는 인용되므로 canonical 출력은 `schema_version: "1"`입니다.
- 새 프로젝트의 `schema_version`은 `CURRENT_SCHEMA_VERSION` = `1`입니다.
- **현재 구현**: `from_yaml`은 `schema_version` 값을 1과 비교하지 않고 모든 `u32`를 받습니다.
  `project.id` 형식도 이 단계에서 검사하지 않습니다(§5.4의 변환은 별도 함수).

canonical 예(새 프로젝트, `project::scaffold::default_manifest`):

```yaml
schema_version: "1"
project:
  id: "0195c530-3acd-7bc1-8414-c213f61cb965"
  name: Demo
target_profiles: []
dependencies: {}
```

### 14.6 `stellar.lock.yaml`

근거: `project::lock`, `project::scaffold`.

| 경로 | 필수 | 형식 |
|---|---|---|
| `schema_version` | 예 | 스칼라, `parse_u32` |
| `resolved` | 예 | 시퀀스(같은 라이브러리의 여러 항목 허용) |
| `resolved[i]` | — | 매핑, 아래 6개 키 모두 필수, 다른 키 금지 |
| `resolved[i].logical_id` | 예 | 스칼라 |
| `resolved[i].source` | 예 | 스칼라 |
| `resolved[i].exact_commit` | 예 | 스칼라 |
| `resolved[i].content_hash` | 예 | 스칼라 |
| `resolved[i].schema_version` | 예 | 스칼라, `parse_u32` |
| `resolved[i].dependency_path` | 예 | 스칼라의 시퀀스 |

- writer 키 순서는 위 표의 순서입니다. 새 프로젝트는 `schema_version: "1"`과 `resolved: []`를 씁니다
  (`CURRENT_LOCK_SCHEMA_VERSION` = `1`).
- 기술 아키텍처 §20.3 예시의 `lock_version`이 아니라 `schema_version` 키를 씁니다.
- **현재 구현**: `from_yaml`은 최상위 `schema_version`을 1과 비교하지 않습니다. `exact_commit`과
  `content_hash`의 값 형식도 검사하지 않습니다.

`stellar-dependencies`에는 다른 모델이 있습니다.

- `dependencies::declaration`(XV.1.1)은 `dependencies.<name>`에 `library_id`, `local_override`, 매핑
  형태의 `version`(`branch`·`commit`)을 둡니다.
- `dependencies::lock`(XV.4.1)은 `resolved[i]`에 10개 키(`library_id`, `repository`, `requested`,
  `dependencies` 등)를 두고 `schema_version`이 정확히 1이어야 합니다.
- 두 모델은 `project` crate의 열기·쓰기 경로에 연결되지 않았고 위 §14.5·§14.6 스키마와 호환되지
  않습니다(`library_id`는 `UnknownKey`). 어느 쪽을 파일 형식으로 할지 정하지 않았습니다. 이 명세의
  규범 스키마는 열기 경로가 실제로 쓰는 §14.5·§14.6입니다.

## 15. unknown field·unknown 요소 규칙

| 요소 | 모르는 값이 optional일 때 | required·mandatory일 때 | 근거 |
|---|---|---|---|
| tagged field(storage decoder) | 건너뜁니다. 결과에서 빠지며 다시 쓸 때 사라집니다 | `UnknownMandatoryTag`로 거부 | `storage::canonical::decode::take_fields` |
| tagged field(AssetRecord) | 보존하고, 다시 쓸 때 tag 순서대로 그대로 씁니다 | `UnknownMandatoryTag`로 거부 | `assets::record` |
| field flags의 정의되지 않은 bit | — | `InvalidFlags`로 거부 | `storage::canonical::decode` |
| chunk kind code | registry가 `SkipUnknownOptional` | `UnknownRequiredKind` | `storage::chunk::kind_registry` |
| hash 알고리즘 이름 | — | `UnknownAlgorithm`으로 거부 | `storage::hash` |
| chunk·manifest format version | — | 1이 아니면 `UnsupportedVersion` | `storage::chunk::envelope`, `storage::generation::manifest` |
| envelope·manifest·ViewTree·DocumentManifest의 추가 필드 | 확장 지점이 없습니다. 끝의 남은 bytes는 `TrailingBytes`(envelope는 header 뒤 전부가 payload이며 길이로 검사) | | §6.4, §9, §10 |
| YAML 매핑의 모르는 키 | — | `UnknownKey`로 거부 | `project::manifest`, `project::lock` |
| known field의 정의되지 않은 값(예: AssetRecord source kind 3) | — | 명시 오류로 거부(보존하지 않음) | `assets::record` |

- 결정론: 보존 정책을 쓰는 형식은 decode → encode 결과가 원래 bytes와 같아야 합니다(`assets::record`
  테스트). 건너뛰기 정책은 다시 쓰면 그 field가 사라집니다.
- 모르는 tag는 정렬·중복 검사에서 아는 tag와 똑같이 다룹니다.
- **현재 구현**: chunk kind 해석은 열기 경로에서 쓰이지 않습니다(§7). AssetRecord는 encoder·decoder만
  있고 아직 chunk로 저장되지 않습니다(`assets::catalog`는 메모리 index입니다).

AssetRecord field 표(`assets::record`):

| 레코드 | tag | field | mandatory | payload |
|---|---|---|---|---|
| AssetRecord | 1 | content hash | 예 | string(§4) |
| AssetRecord | 2 | media type | 예 | string(소문자 `type/subtype`) |
| AssetRecord | 3 | size | 예 | `u64` LE |
| AssetRecord | 4 | source metadata | 아니오 | 중첩 레코드(없으면 field 생략) |
| SourceMetadata | 1 | location kind | 예 | `u8`: 1 = 프로젝트 내부, 2 = 외부 |
| SourceMetadata | 2 | path | 예 | string: 내부는 root 기준 `/` 상대 경로, 외부는 파일 이름만 |
| SourceMetadata | 3 | origin fingerprint | 외부일 때만 필수, 내부면 금지 | string(§4) |

known tag의 mandatory flag가 위 표와 다르면 거부합니다.

## 16. 버전과 migration

근거: `storage::migration::{compat, registry, runner}`, `xtask::migration_fixtures`, ADR-0011.
버전 번호 전체 목록과 호환성 matrix는 호환성 문서에 있습니다.

### 16.1 storage 문서 스키마 버전

- generation manifest의 `schema_version`(§10.1)이 storage 문서 스키마 버전입니다. `stellar.yaml`의
  `schema_version`(§14.5)과는 별개입니다.
- 최신 storage 문서 스키마 버전은 `LATEST_SCHEMA` = `2`이며 `xtask::migration_fixtures`에
  정의되어 있습니다. 제품 registry(`product_registry`)에 등록된 step은 payload를 바꾸지 않는 noop
  v1→v2(`storage::migration::runner::noop_v1_to_v2`) 하나입니다.

### 16.2 호환성 판정 규칙

`FormatVersions { format_version, schema_version, minimum_reader_version, minimum_writer_version }`
(`storage::migration::compat`):

1. 선언 불변식 `minimum_reader_version ≤ minimum_writer_version ≤ schema_version`을 어기면
   `Rejected(InvalidDeclaration)`.
2. `format_version`이 reader가 지원하는 값과 다르면 `Rejected(FormatMismatch)`.
3. reader 스키마 버전 `A < minimum_reader_version`이면 `Rejected(ReaderTooOld)`.
4. `A < minimum_writer_version`이면 `ReadOnly`.
5. 그 밖에는 `Editable`.

**현재 구현**: manifest는 `minimum_reader_version`·`minimum_writer_version`을 기록하지 않고, 열기 경로는
이 판정을 호출하지 않습니다(ADR-0011 "정하지 않은 사항").

### 16.3 migration step registry

- step은 `from → to`이며 `to > from`이어야 합니다(`InvalidStep`). 출발 버전당 step은 하나입니다
  (`DuplicateStep`).
- 경로 계획: 현재 버전에서 목표 버전까지 step을 이어 갑니다. step이 없거나 step이 목표를 지나치면
  `NoPath`, 같은 버전을 다시 방문하면 `Cycle`입니다. 목표가 현재보다 낮으면 경로가 없으므로
  `NoPath`입니다(자동 다운그레이드 없음).
- 각 step 실행 뒤 registry가 버전을 `to`로 바꿉니다. step이 실패하면 그 지점에서 멈춥니다.

### 16.4 migration 실행

`storage::migration::runner::run_migration`:

1. current 세대를 읽기 전용으로 로드하고 모든 chunk를 §6.4 검증으로 읽습니다.
2. 메모리에서 목표 버전까지 변환합니다.
3. 결과를 §13 저장 절차로 새 generation에 씁니다. 모든 chunk pointer의 `schema_version`과 manifest의
   `schema_version`은 도달한 버전입니다. 내용이 같은 chunk는 다시 쓰지 않습니다.
4. 원본 파일은 바꾸지 않습니다. pointer 교체 전에 실패하면 원본 current가 유지됩니다.
5. Git commit을 만들지 않습니다. noop v1→v2 뒤 Git 변화는 `design/current` 수정과 새 manifest 파일
   추가뿐입니다(`crates/storage/tests/migration_git_fixture.rs`).

### 16.5 migration golden fixture

`cargo xtask test-migrations`가 과거 스키마 fixture를 실제 저장 파이프라인으로 만들고 `LATEST_SCHEMA`
까지 변환한 결과를 기대 snapshot과 비교합니다. 현재 fixture는 `v1-minimal`(DocumentManifest chunk 1개),
`v1-two-views`(DocumentManifest 1개 + ViewTree 2개)이며, 기대값은 payload가 그대로인 것입니다.

## 부록 A. 근거·fixture 대응표

| 절 | 고정하는 fixture·테스트 |
|---|---|
| §3 | `docs/fixtures/storage/canonical-primitives.yaml`(`cargo xtask validate-storage-fixtures`, `storage::canonical::encode` 테스트), `crates/storage/tests/fuzz_corpus/chunk_canonical/stream_*.hex` |
| §4 | `storage::hash` 테스트(FIPS 180-4 vector), `envelope_uppercase_hash.hex`, `*_multibyte_*.hex` |
| §5.1 | `core-types::ids` 테스트 |
| §5.3 | `project::document_io::serialize` 테스트 |
| §6 | `golden_header_bytes_are_fixed`, `chunk_golden_test.hex`, `chunk_view_tree.hex`, `envelope_bad_version.hex` |
| §7 | `storage::chunk::kinds`·`kind_registry` 테스트 |
| §8 | `storage::chunk::pointer`·`remap` 테스트, `pointer_hash_multibyte_4byte.hex` |
| §9.1 | `view_tree_minimal.hex`, `view_tree_duplicate_root.hex` |
| §9.2, §9.3 | `crates/project/tests/document_serialize.rs`, `document_round_trip.rs` |
| §10 | `manifest_minimal.hex`, `manifest_misordered_pointers.hex`, `storage::generation::manifest` 테스트 |
| §11 | `text_current_pointer.hex`, `text_hash_multibyte_2byte.hex` |
| §12, §13 | `storage::generation::{loader, scanner, open, lazy}`·`storage::save` 테스트(fault matrix 22개 지점) |
| §14 | `project::yaml` 테스트, `crates/project/tests/fuzz_corpus/restricted_yaml/*.yaml` |
| §15 | `storage::canonical::decode` 테스트, `stream_fields_unknown_mandatory.hex`, `assets::record` 테스트 |
| §16 | `storage::migration` 테스트, `cargo xtask test-migrations`, `migration_git_fixture.rs` |

## 부록 B. 기술 아키텍처 문서와의 차이

| 아키텍처 | 구현(이 명세) |
|---|---|
| §19 경로 이름(`current.stellarref`, `.stargen`, `objects/sha256/`) | §2 표 |
| §20.3 잠금 파일 `lock_version` | `schema_version`(§14.6) |
| §36 프로젝트에 네 버전 기록 | manifest에는 `schema_version`만 기록(§16.2) |
