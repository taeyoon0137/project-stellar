# Stellar 파일 포맷 보안 한도와 parser 요구사항

- 문서 종류: 규범(normative) 명세. 구현된 한도와 동작을 코드에서 모았습니다.
- WBS: XXVIII.1.4
- 기준 commit: `78a574a` 위의 구현. 형식 자체는 [`stellar-format.md`](stellar-format.md)(XXVIII.1.2,
  이하 "형식 명세")가 정합니다. 이 문서의 절 번호 앞의 "형식 §"는 그 문서의 절입니다.
- 표기: 형식 명세 §0과 같습니다("해야 합니다", "하면 안 됩니다", "현재 구현", 근거 표기
  `storage::chunk::store` 등).

## 1. 위협 모델

기술 아키텍처 §37.1은 신뢰되지 않은 프로젝트를 전제합니다. clone한 저장소, 받은 압축 파일, 다른
사람이 수정한 working tree의 모든 파일은 공격자가 만들 수 있다고 봅니다. 신뢰 상태와 무관하게 아래
parser는 이런 입력을 처리합니다.

| 입력 | 공격자 제어 | 처리 코드 |
|---|---|---|
| `stellar.yaml`, `stellar.lock.yaml` | 예 | `project::yaml::loader`, `project::manifest`, `project::lock` |
| `design/current` | 예 | `storage::generation::current` |
| `design/generations/*` 파일 내용·파일 이름 | 예 | `storage::generation::{manifest, loader, scanner}` |
| `design/objects/**` 파일 내용·파일 이름·디렉터리 구조 | 예 | `storage::chunk::{envelope, store}`, `storage::gc::sweep` |
| chunk payload(DocumentManifest, ViewTree, subtree) | 예(hash가 맞게 다시 만들 수 있음) | `project::document_io::{manifest, load}`, `storage::chunk::view_tree`, `document::record_codec` |
| 프로젝트 안의 symlink | 예 | 모든 파일 접근(§6) |
| 복구 스냅샷, 신뢰 목록(앱 로컬 데이터 영역) | 아니오(같은 사용자 계정) | `project::recovery_payload`, `project::trust_store` |
| 제품 저장소의 `config/*.yaml` | 아니오(제품과 함께 배포) | `storage::chunk::budget`, `storage::gc::retention`, `document::open_limits`, `assets::policy` |

content hash는 무결성 검증용입니다. 공격자는 payload를 바꾸고 hash와 파일 이름을 맞게 다시 만들 수
있으므로, hash 일치는 "의도한 내용"이 아니라 "주소와 내용이 서로 맞음"만 보장합니다(ADR-0004
Security).

## 2. 공통 parser 요구

모든 프로젝트 파일 parser는 다음을 지켜야 합니다.

| ID | 요구 | 근거 |
|---|---|---|
| P1 | malformed 입력에서 panic하면 안 됩니다. release profile은 `panic = "abort"`(II.1.3)라 panic은 프로세스 종료, 곧 서비스 거부입니다. 오류는 구조화된 타입으로 반환합니다. | ADR-0009, ADR-0015, `docs/architecture/panic-policy.md` |
| P2 | 길이 프리픽스는 남은 입력과 비교한 뒤에만 slice합니다. 큰 길이는 할당 없이 `LengthOverflow`입니다. | `storage::canonical::decode::take_bytes` |
| P3 | 입력에 적힌 count로 메모리를 미리 크게 잡으면 안 됩니다. 선할당은 아래 §3.3의 상한까지만 하고, 나머지는 실제로 읽은 항목만큼 늘립니다. | §3.3 |
| P4 | 중첩 구조는 반복(iterative)으로 처리하거나 깊이 상한을 둡니다. 재귀 descent로 stack을 소모하면 안 됩니다. | `project::yaml::loader`, `project::yaml::writer::check_structure` |
| P5 | 컨테이너가 끝을 정하는 형식은 정확히 끝까지 소비합니다(`TrailingBytes`). | 형식 §3.7 |
| P6 | 손상 데이터를 자동으로 고치거나 지우면 안 됩니다. 열기는 디스크에 쓰지 않습니다. | 형식 §6.4, §12 |
| P7 | 파일 경로는 `ContentHash` 등 검증된 타입에서만 만듭니다. 파일 이름을 해석할 때도 같은 parser를 씁니다. | `storage::chunk::path`, `storage::generation::loader::manifest_path`, `assets::path` |
| P8 | 첫 오류에서 멈추고 그 오류를 보고합니다. 나머지 입력은 결과에 영향을 주지 않습니다. | `project::yaml::loader::load_restricted`, 각 decoder |

P1 보충: 제품 decoder의 `expect`는 앞에서 증명한 불변식에만 씁니다. 예를 들어 `take_array`는
길이를 확인한 slice를 배열로 바꾸고, `ContentHash::from_str`은 모든 문자가 ASCII 16진수임을 확인한 뒤
2 byte씩 slice합니다(ADR-0015가 고친 결함).

## 3. binary 형식 한도

### 3.1 고정 크기

| 항목 | 값 | 근거 |
|---|---|---|
| content hash 문자열 | 정확히 71 byte(`sha256:` + 64) | `storage::hash` |
| chunk envelope header | 99 byte(SHA-256) | `storage::chunk::envelope` |
| chunk pointer | 99 byte(SHA-256) | `storage::chunk::pointer` |
| generation manifest 최소 크기 | 49 byte(parent 없음, pointer·root 0개). parent가 있으면 57 byte | `storage::generation::manifest` |
| manifest의 pointer·root 항목 | 99 byte, 16 byte | 같음 |
| ViewTree payload | 20 byte + 항목당 32 byte | `storage::chunk::view_tree` |
| DocumentManifest View 항목 | 32 byte | `project::document_io::manifest` |
| `design/current` | 정확히 72 byte(SHA-256) | `storage::generation::current` |

### 3.2 값 범위

| 항목 | 범위 | 위반 시 |
|---|---|---|
| 길이 프리픽스 | `u32`(최대 4,294,967,295), 그리고 남은 입력 이하 | `LengthOverflow` |
| count(map, field, pointer, root, subtree, View) | `u32`. 항목마다 입력을 소비하므로 실제 상한은 입력 길이입니다 | 입력이 모자라면 `UnexpectedEof`·`LengthOverflow` |
| `bool` | `00`·`01` | `InvalidBool` |
| float | 유한 값, `-0.0` 아님 | `InvalidFloat` |
| field flags | `00`·`01` | `InvalidFlags` |
| generation | 1 이상 | `ZeroGeneration` |
| parent generation | generation 미만 | `InvalidParent` |
| chunk·manifest format version | 1 | `UnsupportedVersion` |

### 3.3 선할당 상한

| decoder | count 필드 | 선할당 상한 | 근거 |
|---|---|---|---|
| canonical map·field 레코드 | entry·field 수 | 0(선할당 없음) | `storage::canonical::decode` |
| generation manifest | pointer 수, root 수 | 각각 1,024 | `storage::generation::manifest` |
| ViewTree payload | subtree 수 | 1,024 | `storage::chunk::view_tree` |
| DocumentManifest payload | View 수 | 1,024 | `project::document_io::manifest` |
| `DocumentDefinition` | View·page·document 수 | 각각 1,024 | `document::definitions` |
| `SubtreeRecords` | node 수, modifier stack 수 | 각각 4,096 | `document::record_codec` |
| 복구 스냅샷 | 미저장 chunk·탭 수 / 선택 수 | 1,024 / 4,096 | `project::recovery_payload` |

### 3.4 정하지 않은 한도(현재 구현)

- chunk 파일, manifest 파일, `design/current` 파일에는 최대 크기가 없습니다. reader는 파일 전체를
  메모리로 읽은 뒤 검증합니다(`std::fs::read`, `std::fs::read_to_string`). 메모리 사용량은 파일 크기에
  비례합니다.
- envelope의 uncompressed length는 `u64`이며 상한이 없습니다. 실제 payload 길이와 같은지만 봅니다.
- chunk 분할 임계치 256 KiB(`config/performance-budgets.yaml`의 `storage.chunk_split_threshold_bytes`,
  ADR-0012)는 저장 최적화이지 입력 한도가 아닙니다. 임계치를 넘는 chunk(`oversized`)도 정상입니다.
- manifest의 pointer 수, 문서·View·node 수의 형식 상한은 없습니다(입력 길이가 상한). 문서 트리 깊이는
  문서 validator가 View root 기준 64(`document::validate::MAX_NODE_DEPTH`)로 제한합니다.
- fallback scanner가 검증하는 후보 manifest 수에 상한이 없습니다. 후보마다 manifest 전체를 읽고 모든
  pointer의 chunk 존재를 확인하므로, `design/generations/`의 파일 수와 크기에 비례해 시간이 듭니다.

## 4. 제한 YAML 한도

근거: `project::yaml::loader`, `project::yaml::writer`.

| 한도 | 값 | 판정 | 오류 코드 |
|---|---|---|---|
| 입력 크기 `MAX_INPUT_BYTES` | 1,048,576 byte(1 MiB) | UTF-8 byte 길이 > 상한이면 거부. 정확히 1 MiB는 허용 | `input-too-large` |
| 문자 집합 | YAML 1.2 `c-printable` | TAB, LF, CR, U+0020~U+007E, U+0085, U+00A0~U+D7FF, U+E000~U+FFFD, U+10000~U+10FFFF만 허용. 위반 문자의 줄·열 보고 | `non-printable` |
| 중첩 깊이 `MAX_NESTING_DEPTH` | 64 | 매핑·시퀀스 중첩 64단계까지 허용, 65단계째 거부 | `nesting-too-deep` |
| 이벤트 선읽기 `MAX_EVENT_LOOKAHEAD_CHARS` | 32,768 문자 | 이벤트 하나를 만들려고 scanner가 직전 이벤트 이후 상한을 넘는 문자를 읽어야 하면 거부(너무 긴 스칼라, 버퍼링되는 flow collection, 이어진 주석·공백) | `lookahead-too-large` |
| implicit key 길이(writer) | 1,024 문자 | 넘으면 explicit key(`? `)로 씀 | — |

- 크기와 문자 검사는 하위 parser를 부르기 전에 합니다. NUL은 `non-printable`로 거부합니다. 하위
  scanner는 NUL을 입력 끝으로 다루므로, 거부하지 않으면 Git이 보는 내용과 Stellar가 읽는 내용이
  달라집니다.
- 이벤트는 하나씩 당겨 처리합니다(재귀 `load` 사용 안 함). 깊은 block 중첩에서도 stack overflow가
  없습니다.
- 중복 키는 매핑마다 무작위 seed(`RandomState`) hash 집합으로 선형 시간에 판정하고, hash가 겹칠 때만
  실제 키를 비교합니다. 입력이 hash 충돌을 고를 수 없습니다.
- 파일로 쓰는 writer(`write_canonical_checked`)는 같은 한도를 기록 전에 강제합니다. 출력이 1 MiB를
  넘으면 `OutputTooLarge`, loader가 다시 읽지 못하면 `NotReloadable`, 다시 읽은 값이 다르면
  `RoundTripMismatch`이며, 이때 텍스트를 반환하지 않습니다.
- **현재 구현**: `probe_project`는 `stellar.yaml` 전체를 `read_to_string`으로 읽은 뒤 크기를
  검사합니다. 1 MiB 상한은 parser 비용의 상한이지 읽기 메모리의 상한이 아닙니다. UTF-8이 아닌 파일은
  `ManifestRead`(읽기 실패)입니다.

## 5. malformed 입력 동작

각 표의 "효과"는 열기 경로(형식 §12)에서 생기는 결과입니다.

### 5.1 canonical primitive

| 입력 | 오류 | 근거 |
|---|---|---|
| 필요한 byte보다 짧음 | `UnexpectedEof { needed, remaining }` | `storage::canonical::decode` |
| 길이 프리픽스 > 남은 입력 | `LengthOverflow { len, remaining }` | 같음 |
| bool `02`~`ff` | `InvalidBool` | 같음 |
| 비 UTF-8 문자열 | `InvalidUtf8` | 같음 |
| NaN·±Inf·`-0.0` bit | `InvalidFloat` | 같음 |
| map key 중복 / 역순 | `DuplicateKey` / `MisorderedKey` | 같음 |
| field tag 중복 / 역순 | `DuplicateTag` / `MisorderedTag` | 같음 |
| field flags bit1~7 | `InvalidFlags` | 같음 |
| 모르는 mandatory tag | `UnknownMandatoryTag` | 같음 |
| 끝의 남은 bytes | `TrailingBytes` | 같음 |

field 하나의 판정 순서는 tag 읽기 → flags 읽기 → flags 검사 → payload 읽기 → 순서·중복 검사 →
known/unknown 판정입니다.

### 5.2 content hash 문자열

| 입력 | 오류 |
|---|---|
| `:` 없음 | `MissingSeparator` |
| `sha256` 아닌 이름(대소문자 구분) | `UnknownAlgorithm` |
| hex 부분 byte 길이 ≠ 64 | `InvalidLength` |
| hex 부분에 `[0-9a-f]` 아닌 문자(대문자, 다중 byte UTF-8 포함) | `InvalidHexDigit { found }`(첫 위반 문자) |

ADR-0015 발견(XVI.6.2): byte 길이 64인 hex 자리에 2·3·4 byte UTF-8 문자가 있으면 이전에는 문자 경계
안쪽을 slice해 panic했습니다. 지금은 slice 전에 모든 문자를 검사합니다. 각 진입점의 결과:

| 진입점 | 결과 |
|---|---|
| chunk envelope payload hash | `EnvelopeError::InvalidHash` |
| chunk pointer content hash | `PointerCodecError::InvalidHash` → `ManifestError::Pointer` |
| `design/current` 내용 | `CurrentPointerError::InvalidFormat` |
| `design/generations/` 파일 이름 | 후보에서 제외(오류 아님) |
| `design/objects/` 파일 이름(GC) | sweep 대상에서 제외 |

### 5.3 chunk 파일 읽기

| 입력 | 오류(`ChunkStoreError`) | 효과 |
|---|---|---|
| 파일 없음 | `MissingChunk` | 열기 시점이면 `LoadError::MissingChunk`(존재만 확인), 읽기 시점이면 `LazyLoadError::Chunk` |
| 읽기 실패 | `Io` | 같음 |
| magic·version·hash 형식 위반, header 절단 | `Envelope(...)` | 읽기 실패 |
| header kind ≠ pointer kind | `KindMismatch` | 읽기 실패 |
| payload 길이 ≠ uncompressed length | `LengthMismatch` | 읽기 실패 |
| header hash ≠ 요청 주소 | `AddressMismatch` | 읽기 실패 |
| payload 재해시 불일치(bit 손상) | `PayloadHashMismatch` | 읽기 실패 |
| 쓰기 대상에 다른 내용의 파일이 이미 있음 | `CorruptExisting` | 저장 실패, 덮어쓰지 않음 |

### 5.4 generation manifest

| 입력 | 오류(`ManifestError`, `LoadError`) |
|---|---|
| 파일 없음 | `LoadError::MissingManifest` |
| 파일 bytes hash ≠ 파일 이름의 hash | `LoadError::ManifestHashMismatch` |
| magic 불일치 / version ≠ 1 | `BadMagic` / `UnsupportedVersion` |
| generation 0 / parent ≥ generation | `ZeroGeneration` / `InvalidParent` |
| pointer 역순, 완전 중복 | `MisorderedPointers` |
| 같은 ChunkId, 다른 hash(오름차순) | `PointerSet(ConflictingChunkId)` |
| 같은 ChunkId, 같은 hash, 다른 kind | `PointerSet(DuplicateChunkId)` |
| root document id 중복 | `DuplicateRootDocument` |
| pointer 안 hash 문자열 위반 | `Pointer(InvalidHash)` |
| 끝의 남은 bytes, 절단 | `Decode(TrailingBytes)`, `Decode(UnexpectedEof)` 등 |
| project id ≠ 기대값 | `LoadError::WrongProjectId` |

### 5.5 current pointer

| 입력 | 오류(`CurrentPointerError`) |
|---|---|
| 파일 없음 | `Missing`(저장 경로에서는 "첫 저장" 상태) |
| 마지막 LF 없음 | `InvalidFormat` |
| LF 두 개 이상 | `InvalidFormat` |
| CRLF | `InvalidFormat`(hex 길이 65) |
| hash 형식 위반 | `InvalidFormat` |
| 비 UTF-8 | `Io` |

### 5.6 열기 결과

| 상황 | 결과(`storage::generation::open`) |
|---|---|
| current 세대 검증 통과 | `OpenOutcome::Current` |
| current 실패, 검증을 통과한 다른 세대 있음 | `OpenOutcome::ReadOnlyRecovery { fallback, current_error }`. current pointer는 그대로 |
| current 실패, 통과한 세대 없음 | `OpenError::NoValidGeneration` |
| `design/generations/` 읽기 실패 | `OpenError::Scan` |

fallback scanner는 이름이 형식에 맞지 않는 파일과 검증에 실패한 후보를 조용히 건너뜁니다. 디스크를
바꾸지 않습니다.

### 5.7 문서 chunk payload

| 입력 | 오류(`LoadDocumentError`) |
|---|---|
| DocumentManifest payload 형식 위반, View 항목 불일치 | `Manifest(...)` |
| 요청 문서의 DocumentManifest 없음 | `UnknownDocument` |
| 참조 chunk 읽기 실패(§5.3) | `Read` |
| ViewTree payload 형식 위반, subtree root 중복 | `ViewTree` |
| ViewTree root ≠ View root frame | `ViewRootMismatch` |
| subtree payload 형식 위반, 알 수 없는 node kind | `Subtree` |
| subtree root ≠ ViewTree 항목 | `SubtreeRootMismatch` |
| 합친 레코드의 불변식 위반(ID 중복·NIL, root, 부모 cycle, OrderKey 중복, 참조 종류, modifier cycle, 깊이 > 64 등) | `Model(...)` |

대상이 없는 참조는 오류가 아니라 미해결 진단(`DocumentDiagnostics`)입니다(`document::validate`).

### 5.8 YAML과 매니페스트

| 입력 | 결과(`project::probe`) |
|---|---|
| root에 `stellar.yaml`·`.git`이 함께 없음 | `ProbeError::Root` |
| 파일 읽기 실패(비 UTF-8 포함) | `ProbeError::ManifestRead` |
| §4 한도 위반, 형식 §14.2 거부 구조, 문법 오류 | `ProbeError::ManifestSyntax(LoaderError)`(줄·열·코드) |
| 스키마 위반(누락·모르는 키·종류·버전 형식) | `ProbeError::ManifestSchema` |
| 신뢰되지 않은 프로젝트에서 `stellar.yaml`이 symlink | `SafeProbeError::UntrustedSymlinkRefused`(`safe_probe`) |

신뢰되지 않은 프로젝트의 safe probe는 Git과 필터를 실행하지 않습니다(`project::probe::safe_probe`).

## 6. 파일 시스템 경계

- 경로 생성(P7): chunk·manifest·에셋 경로는 hash 타입에서만 만들고 경로 문자는 `[a-z0-9]`, `-`, `.`,
  `/`뿐입니다. `..`나 구분자를 넣을 방법이 없습니다.
- 파일 이름 해석: scanner와 GC는 `<alg>-<hex64>.<확장자>` 이름만 content hash parser로 해석하고 나머지
  이름은 무시합니다. 임시 파일(`.<name>.tmp-<pid>-<n>`)은 GC 대상이 아닙니다. 강제 종료로 남은 임시
  파일은 현재 정리하지 않습니다.
- **현재 구현 — symlink**: storage는 symlink를 검사하지 않고 따라갑니다. chunk·manifest·current 읽기는
  `std::fs::read`·`read_to_string`, 존재 확인은 `is_file`, scanner는 `read_dir`를 씁니다.
- **현재 구현 — GC와 symlink(알려진 위험 R1)**: `storage::gc::sweep::collect_files`는
  `design/objects/`와 `design/generations/` 아래를 `path.is_dir()`(symlink를 따름)로 재귀 순회합니다.
  `design/objects/` 안에 프로젝트 밖 디렉터리를 가리키는 symlink가 있으면, 그 디렉터리의
  `sha256-<hex64>.starchunk` 이름 파일 중 mark되지 않고 grace period가 지난 파일을 지웁니다. 순회에는
  방문 집합이 없어서 symlink 순환도 따라갑니다. 자기 자신을 가리키는 symlink가 하나면 OS의 symlink 해석
  한도에서 멈추지만, 둘이면 경로 수가 단계마다 두 배가 되어 sweep이 끝나지 않습니다. 두 동작 모두 기준
  commit에서 재현했습니다(`docs/task-reports/XXVIII.1.4.md`).
- 신뢰되지 않은 프로젝트의 외부 symlink 정책(기술 아키텍처 §19.3, §37.1)은 `safe_probe`의
  `stellar.yaml` 검사 외에는 storage 경로에 적용되어 있지 않습니다.

## 7. 앱 로컬 형식

프로젝트 밖 앱 데이터 영역의 형식도 같은 decoder 규칙을 씁니다.

- 복구 스냅샷(`project::recovery_payload`): 첫 필드 `RECOVERY_PAYLOAD_VERSION`(`u32`, 현재 1)이 다르면
  `UnsupportedVersion`. canonical primitive를 쓰고 비유한 viewport를 거부합니다. 선할당 상한은 §3.3과
  같습니다.
- 신뢰 목록(`project::trust_store`): 제한 YAML loader로 읽으며 `TRUST_STORE_SCHEMA_VERSION` = 1입니다.

## 8. 에셋 import 한도

에셋 import 입력은 사용자가 고른 파일이며 프로젝트 parser는 아니지만, 결과가 프로젝트에 들어가므로
적어 둡니다(`assets::{store, policy, path, media_type}`, `config/asset-import.yaml`).

| 한도 | 값 |
|---|---|
| 단일 파일 최대 크기 `max_file_size_bytes` | 536,870,912 byte(512 MiB, 잠정값). 같은 크기까지 허용 |
| 읽기 buffer `STREAM_BUFFER_SIZE` | 65,536 byte. 파일 전체를 메모리에 올리지 않음 |
| media type의 type·subtype 길이 `MAX_MEDIA_TYPE_PART_LEN` | 각 127자 |
| 저장 확장자 `MAX_EXTENSION_LEN` | 1~16자 `[a-z0-9]`, 위반이면 `bin` |
| 모르는 media type | `unknown_media_type: store_as_binary`(`application/octet-stream`, `bin`) |
| 두 번 읽은 hash·크기 불일치(import 중 원본 변경) | `ImportError::SourceChanged`, 임시 파일 삭제 |

## 9. fuzz corpus 연결

두 harness 모두 외부 fuzz 도구 없이 고정 seed SplitMix64로 결정론적으로 실행하는 `#[test]`입니다.
입력마다 (1) panic 없음(`catch_unwind`), (2) hang 없음(입력당 3초, 전체 watchdog 60초), (3) 과도한
할당 없음(thread-local 할당 계측)을 판정합니다. 기본 반복 수는 25,000이고 변형 입력은 64 KiB까지입니다.

### 9.1 chunk·canonical decoder harness (XVI.6.2)

`crates/storage/tests/chunk_canonical_fuzz.rs`. 대상 7개를 모든 입력에 적용합니다.

| target | 대상 | 명세 |
|---|---|---|
| `stream` | 입력 byte가 정한 순서의 canonical op(`take_*`) 16종, map value·field payload는 중첩 stream(깊이 3) | 형식 §3, 이 문서 §5.1 |
| `envelope` | `ChunkEnvelope::decode`, kind registry 해석, object 경로 생성 | 형식 §6.1, §6.2, §7 |
| `pointer` | `ChunkPointer::decode` | 형식 §8 |
| `manifest` | `GenerationManifest::decode` | 형식 §10, 이 문서 §5.4 |
| `view_tree` | `ViewTreeChunk::decode` | 형식 §9.1 |
| `text` | `ContentHash`·`ChunkId` 문자열 parse(current pointer 형식 포함) | 형식 §4, §5.2, §11, 이 문서 §5.2 |
| `chunk_file` | 입력을 chunk 경로에 쓴 뒤 `read_chunk`, `write_chunk`(기존 파일 검증) | 형식 §6.3, §6.4, 이 문서 §5.3 |

추가 판정: 수용한 입력은 다시 인코딩하면 소비한 bytes와 같아야 하고, hash 문자열은 표기 그대로
다시 써져야 하며, chunk 읽기 성공은 요청 주소의 canonical 파일일 때뿐이고, 읽기·쓰기는 파일을
바꾸면 안 됩니다. 할당 상한은 `64 × 입력 길이 + 256 KiB`입니다.

seed corpus `crates/storage/tests/fuzz_corpus/chunk_canonical/`(19개, 파일마다 `target`·`expect` 지시문,
`corpus_seeds_have_expected_outcomes`가 기대 결과를 검사):

| 파일 | target | 기대 | 명세 |
|---|---|---|---|
| `chunk_golden_test.hex` | chunk_file | ok | 형식 §6.1 golden header |
| `chunk_view_tree.hex` | chunk_file | ok | 형식 §6, §9.1 |
| `envelope_bad_version.hex` | envelope | err | 형식 §6.1, 이 문서 §3.2 |
| `envelope_hash_multibyte_3byte.hex` | envelope | err | 이 문서 §5.2(ADR-0015 회귀) |
| `envelope_uppercase_hash.hex` | envelope | err | 형식 §4 |
| `manifest_minimal.hex` | manifest | ok | 형식 §10.1 |
| `manifest_misordered_pointers.hex` | manifest | err | 형식 §10.2, 이 문서 §5.4 |
| `pointer_hash_multibyte_4byte.hex` | pointer | err | 이 문서 §5.2(ADR-0015 회귀) |
| `stream_all_ops.hex` | stream | ok | 형식 §3, unknown optional tag 건너뛰기(형식 §15) |
| `stream_f32_nan.hex` | stream | err | 형식 §3.2 |
| `stream_f64_negative_zero.hex` | stream | err | 형식 §3.2 |
| `stream_fields_unknown_mandatory.hex` | stream | err | 형식 §15 |
| `stream_length_overflow.hex` | stream | err | 이 문서 §2 P2, §3.2 |
| `stream_map_duplicate_key.hex` | stream | err | 형식 §3.5 |
| `stream_map_misordered.hex` | stream | err | 형식 §3.5 |
| `text_current_pointer.hex` | text | ok | 형식 §11 |
| `text_hash_multibyte_2byte.hex` | text | err | 이 문서 §5.2, §5.5(ADR-0015 회귀) |
| `view_tree_duplicate_root.hex` | view_tree | err | 형식 §9.1 |
| `view_tree_minimal.hex` | view_tree | ok | 형식 §9.1 |

고정 stress 입력 48개(모든 target에 적용):

| 범주 | 입력 이름 | 명세 |
|---|---|---|
| count·길이 최대값과 빈 본문 | `stream-map-count-max-no-entries`, `stream-fields-count-max-no-entries`, `stream-bytes-len-max`, `stream-string-len-over-by-one`, `envelope-len-u64-max`, `envelope-hash-len-u32-max`, `manifest-pointer-count-max`, `manifest-root-count-max`, `view-tree-count-max` | 이 문서 §2 P2·P3, §3.2, §3.3 |
| 대량 항목(1 MiB) | `stream-many-map-entries`, `stream-many-fields`, `stream-many-u8-ops`, `stream-nested-maps`, `chunk-large-payload`, `manifest-many-pointers`, `manifest-many-roots`, `view-tree-many-subtrees` | 이 문서 §3.3, §3.4 |
| manifest 구조 규칙 | `manifest-zero-generation`, `manifest-invalid-parent`, `manifest-duplicate-roots`, `manifest-conflicting-pointers`, `manifest-duplicate-pointers` | 형식 §10.2, 이 문서 §5.4 |
| hash 문자열 | `envelope-hash-uppercase`, `current-pointer-valid` | 형식 §4, §11 |
| 다중 byte hash 문자열(ADR-0015) | `multibyte-2-even`, `multibyte-2-odd`, `multibyte-3-even`, `multibyte-3-odd`, `multibyte-4-even`, `multibyte-4-odd`, `multibyte-3-last` 7종. 각각 `envelope-`·`pointer-`·`text-` 접두어가 붙은 입력 3개(envelope hash, pointer hash, current pointer 형식)로 만들어 21개 | 이 문서 §5.2 |
| 무의미 대량 입력 | `zeros`, `all-ff`, `magic-repeated` | 이 문서 §2 P1, P5 |

변형 연산: bit 뒤집기, 경계 byte, 삽입, 구간 삭제, `u32`·`u64` 경계값 덮어쓰기, token 삽입, 구간 복제,
splice, 구간 반복, 절단, 인접 구간 교환, 같은 길이 다중 byte UTF-8 덮어쓰기.

current pointer 파일 읽기와 fallback scanner의 파일 이름 해석은 mutation campaign의 target이 아니며,
고정 회귀 테스트 `multibyte_hash_text_is_rejected_on_every_path`만 다룹니다. 두 경로가 쓰는
`ContentHash` parse는 `text` target이 다룹니다.

### 9.2 제한 YAML harness (XVI.6.1)

`crates/project/tests/restricted_yaml_fuzz.rs`. 대상은 `load_restricted`이고, 수용한 입력은
`Manifest::from_yaml`, `Lockfile::from_yaml`, 엄격 스칼라 해석, `write_canonical` 재기록 후 재로드 일치까지
실행합니다. 할당 상한은 `64 × 입력 길이 + 384 × min(입력 길이, 32,768) + 256 KiB`입니다.

입력 bytes는 `String::from_utf8_lossy`로 문자열로 바꾼 뒤 넣습니다(loader API가 `&str`). 판정은 공통 세
기준과, 수용한 입력의 재기록·재로드 값 일치(round-trip)입니다.

seed corpus `crates/project/tests/fuzz_corpus/restricted_yaml/`(12개, 파일별 기대 결과 지시문은 없음):

| 파일 | 내용 | 명세 |
|---|---|---|
| `bom_tabs.yaml` | 선행 BOM, TAB, 깊은 들여쓰기 | 형식 §14.1, 이 문서 §4 문자 집합 |
| `duplicate_keys.yaml` | block·중첩·flow·인용 키 중복 | 형식 §14.2 `duplicate-key` |
| `forbidden_anchor_alias.yaml` | anchor, alias, 없는 alias | 형식 §14.2 `anchor`·`alias` |
| `forbidden_merge_key.yaml` | block·flow `<<` 키, 값·항목 `<<` | 형식 §14.2 `merge-key` |
| `forbidden_tags.yaml` | core schema·verbatim·custom tag | 형식 §14.2 `custom-tag` |
| `lock_multi_version.yaml` | 같은 라이브러리 두 버전 lock | 형식 §14.6 |
| `manifest_full.yaml` | 모든 선택 필드가 있는 매니페스트 | 형식 §14.5 |
| `manifest_yaml11_confusables.yaml` | `yes`, `on`, `010`, 날짜 값 | 형식 §14.3 |
| `multiple_documents.yaml` | `---`, `...` 뒤 document | 형식 §14.2 `multiple-documents` |
| `non_scalar_keys.yaml` | 시퀀스·매핑 explicit key | 형식 §14.2 `non-scalar-key` |
| `scalar_styles_directives.yaml` | `%YAML`·`%TAG`, null 표기, 인용·literal·folded 스칼라, escape | 형식 §14.2, §14.3 |
| `yaml11_scalars.yaml` | YAML 1.1 불리언·진법·부호·`_`·`.inf`·`.nan`·timestamp | 형식 §14.3 |

고정 stress 입력 22개:

| 범주 | 입력 이름 | 명세 |
|---|---|---|
| 깊은 중첩 | `deep-block-sequence`, `deep-block-mapping`, `deep-flow-over-u8`, `deep-flow-within-u8`, `deep-flow-mapping`, `nesting-at-limit` | 이 문서 §4 `nesting-too-deep`, P4 |
| 많은 키·항목 | `many-block-keys`, `many-flow-keys`, `many-sequence-items` | 이 문서 §4 중복 키 선형 판정 |
| 긴 token | `long-plain-scalar`, `long-folded-plain-scalar`, `many-escapes`, `long-simple-key`, `whitespace-run`, `comment-run` | 이 문서 §4 `lookahead-too-large` |
| 다중 document·alias | `many-documents`, `alias-amplification` | 형식 §14.2 |
| 문자 집합·줄바꿈 | `nul-run`, `tab-run`, `crlf-lines` | 이 문서 §4 `non-printable`, 형식 §14.1 |
| 크기 경계 | `exact-size-limit`, `over-size-limit` | 이 문서 §4 `input-too-large` |

변형 연산: byte 뒤집기, 삽입, 구간 삭제, YAML token 삽입, 구간 복제, splice, 구간 반복, 절단,
들여쓰기 변경, CRLF 변환.

### 9.3 fuzz 대상이 아닌 parser

다음은 아직 fuzz target이 없습니다(XVI.6.3~XVI.6.7 미착수): 문서 loader·validator(`document_io::load`,
`document::record_codec`, `document::validate`), DocumentManifest payload, 표현식, Semantic Diff·Merge,
command 열. 복구 스냅샷 decoder와 AssetRecord decoder도 fuzz target이 없습니다(단위 테스트만 있음).

## 10. 알려진 위험과 차이

| ID | 내용 | 위치 | 상태 |
|---|---|---|---|
| R1 | GC sweep이 symlink 디렉터리를 따라가 프로젝트 밖의 content-addressed 이름 파일을 지울 수 있고, 자기 참조 symlink 둘이면 끝나지 않음 | `storage::gc::sweep::collect_files` | 미해결, 재현됨 |
| R2 | chunk·manifest·`design/current`·`stellar.yaml` 읽기에 파일 크기 상한이 없음(전체를 읽은 뒤 검증) | `storage::chunk::store`, `storage::generation::{loader, current}`, `project::probe` | 미해결 |
| R3 | 열기 경로가 모르는 required chunk kind와 manifest `schema_version`으로 거부하지 않음 | `storage::generation::{loader, lazy}` | 미해결(호환성 문서 §4) |
| R4 | 선행 BOM이 첫 키에 붙어 `stellar.yaml`이 `UnknownKey`로 거부됨(조용한 오독은 아님) | `project::yaml::loader` | 동작 기록 |
| R5 | `stellar.lock.yaml`의 `exact_commit`·`content_hash` 값 형식과 최상위 버전을 검사하지 않음 | `project::lock` | 미해결 |
| R6 | `StableId`의 version·variant bit를 검사하지 않음 | `core-types::ids` | 동작 기록(형식 §5.1) |
| R7 | fallback scanner 후보 수 상한 없음 | `storage::generation::scanner` | 미해결(§3.4) |
| R8 | 강제 종료로 남은 임시 파일을 정리하지 않음 | `storage::atomic_file`, `storage::gc::sweep` | 미해결 |

R1·R2·R3·R5·R7·R8의 수정은 보호 경로(`crates/storage/`) 또는 해당 crate의 코드 변경이며, 이 문서
작업의 범위 밖입니다.
