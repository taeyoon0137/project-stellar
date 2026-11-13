# 작업 보고서: XVI.6.2 chunk envelope·canonical decoder fuzz target

- WBS: XVI.6.2
- 작업명: chunk envelope/canonical decoder fuzz target 추가(malformed input panic 0)
- 상태: 완료 (`[x]`)
- 선행: V.2.6(chunk read 전체 검증, `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증, debug 빌드 `cargo test`).

## 변경 파일 목록

- `crates/storage/tests/chunk_canonical_fuzz.rs` (신규: 결정론적 fuzz harness, 계측 할당자, `#[test]` 5개)
- `crates/storage/tests/fuzz_corpus/chunk_canonical/*.hex` (신규 seed corpus 19개: 대상·기대 결과
  지시문 + hex 본문. 정상 chunk 파일·manifest·ViewTree·op stream·current pointer, 정렬·중복·비canonical
  float·길이 overflow·미지원 version·대문자 hash, 발견 회귀 3개)
- `crates/storage/src/hash/mod.rs` (fuzz 발견 수정: `ContentHash` parse의 문자 선검사, 단위 테스트 1개 추가)
- `docs/adr/ADR-0015-storage-hardening.md` (신규: 보호 경로 `crates/storage/` 변경 기록)
- `docs/task-reports/XVI.6.2.md`(신규), 계획서(XVI.6.2 `[ ]`→`[x]`)

## 구현 요약

### fuzz harness (`tests/chunk_canonical_fuzz.rs`)

XVI.6.1과 같은 방식(고정 seed, `#[test]`, 새 의존성·nightly 없음)입니다.

- 입력: seed corpus 19개 + 구조를 아는 생성기의 유효 bytes 64개(canonical op stream, chunk 파일,
  pointer, manifest, ViewTree payload, hash 문자열) + 고정 stress 입력 48개(최대 count·길이
  프리픽스, 1 MiB map entry 87,380개·field 116,507개·`u8` op 524,288개, 1 MiB payload chunk,
  pointer 10,000개·root 60,000개 manifest, subtree 30,000개 ViewTree, manifest 의미 검증 위반 5종,
  다중 byte 문자를 hex 각 위치에 둔 hash 문자열 21개 등).
- 변형: SplitMix64로 bit 뒤집기·경계 byte·삽입·구간 삭제·`u32`/`u64` 경계값 덮어쓰기·token 삽입
  (magic, `sha256:`, 비UTF-8 sequence, NaN·±Inf·-0.0 bits 등)·구간 복제·splice·구간 반복·절단·
  인접 구간 교환·같은 길이 다중 byte UTF-8 덮어쓰기·hex 자리 손상(입력당 1~8회, 64 KiB 상한).
  같은 seed는 같은 입력열을 만듭니다(`campaign_inputs_are_deterministic`).
- 대상(입력마다 7개 전부):
  1. canonical op stream: 입력의 op byte(`% 16`)가 `take_u8`~`take_fields`·`take_raw` 호출 순서를
     정합니다. map value와 field payload는 중첩 stream으로 다시 디코딩합니다(깊이 3).
  2. `ChunkEnvelope::decode` → kind registry 해석, object 경로 생성.
  3. `ChunkPointer::decode`, 4. `GenerationManifest::decode`, 5. `ViewTreeChunk::decode`.
  6. `ContentHash`·`ChunkId` 문자열 parse(current pointer 형식 `hash + LF` 포함).
  7. 디스크: 입력을 chunk object 경로에 기록 → `ChunkStore::read_chunk` → 같은 주소의 원래 payload로
     `write_chunk`(기존 파일 검증).
- 판정(입력·대상마다):
  - **panic**: `catch_unwind`(fuzz thread에서만 기본 panic 출력 생략, 위치는 기록).
  - **hang**: 입력당 3 s 상한 + 60 s 진행 없음 watchdog(기본 2 MiB stack worker thread).
  - **과도한 할당**: 테스트 바이너리 전용 `#[global_allocator]`(System 위임)로 **제품 decoder 호출
    하나**의 peak heap을 재고 `64 × 입력 길이 + 256 KiB` 이하인지 확인합니다(harness 재인코딩·중첩
    재귀는 제외).
  - **canonical 성질**: 수용한 op·envelope·pointer는 다시 인코딩하면 소비 bytes와 byte-identical,
    manifest·ViewTree는 입력 전체와 byte-identical이고 manifest hash = 입력 hash, map key·field tag는
    엄격 증가, 결과에 모르는 tag 없음, f32/f64는 유한·`-0.0` 아님, hash 문자열은 표기 그대로 재기록,
    object 경로는 고정 형식(구성 요소 4개, `[0-9a-z.-]`), chunk 읽기 성공은 요청 주소의 canonical
    파일일 때만, `read_chunk`·`write_chunk`는 손상 파일을 수정하지 않음(`Reused`는 canonical 파일일
    때만, 그 외 `CorruptExisting`).
- 발견 입력과 설명은 `CARGO_TARGET_TMPDIR/chunk-canonical-fuzz/`에 저장하고 테스트를 실패시킵니다.
  기본 반복 25,000회, `STELLAR_STORAGE_FUZZ_ITERATIONS`로 늘릴 수 있습니다. 모든 대상이 수용·거부
  경로를 각각 1회 이상 실행했는지도 확인합니다.
- 그 밖의 테스트: corpus 지시문(`# target`, `# expect: ok|err`) 결과 일치, 할당 계측기 자체 검사,
  발견 회귀(아래)를 모든 진입 경로(parse, envelope, pointer, `read_current_pointer`,
  `scan_latest_valid_generation`)에서 확인.

### 수정 전 harness 결과(실제 측정)

최종 harness를 수정 전 `hash/mod.rs`(HEAD 내용)에 그대로 적용해 확인했습니다. 임시로 되돌린 내용은
수정본으로 복원했고 커밋하지 않았습니다.

| 발견 | 입력 | 결과 |
|---|---|---|
| panic 626건(입력 415개: 변형 394, seed 3, stress 18) | hash 문자열 hex 자리에 다중 byte UTF-8 문자(예: `sha256:` + `€` + `a`×61, 71 byte) | 전부 `crates/storage/src/hash/mod.rs:151` "byte index N is not a char boundary". 대상별: envelope 211, chunk_file 211, pointer 108, text 64, manifest 32 |
| 같은 원인 | `design/current` 내용 `sha256:a€…\n`, `design/generations/sha256-a€….starmanifest` 파일 이름 | `read_current_pointer`, `scan_latest_valid_generation` 모두 panic(임시 probe 테스트로 확인 후 삭제) |
| 그 밖 | hang, 과도한 할당, canonical 성질 위반 | 0건 |

release profile은 `panic = "abort"`이므로 손상·조작된 chunk 하나나 파일 이름 하나로 프로젝트 열기
경로가 프로세스를 끝낼 수 있었습니다.

### 수정(`hash/mod.rs`, ADR-0015)

- `ContentHash::from_str`은 byte 길이(64) 확인 뒤 2 byte slice 전에 hex의 모든 문자를 앞에서부터
  검사하고, 소문자 ASCII 16진수가 아닌 첫 문자를 `InvalidHexDigit { found }`로 보고합니다. 통과한 hex는
  ASCII뿐이라 slice가 항상 문자 경계입니다.
- 문자열 형식(ADR-0004), 오류 종류, 수용 집합은 그대로입니다. ASCII 입력의 오류 결과도 같습니다
  (기존 단위 테스트 무수정 통과). 이전에 panic하던 입력만 오류로 바뀝니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-storage` | 종료 코드 0 |
| `cargo test -p stellar-storage` | 종료 코드 0 (unit 193 passed, 0 failed; 통합 `chunk_canonical_fuzz` 5 passed; `migration_git_fixture` 1 passed; doc 2 passed) |
| `STELLAR_STORAGE_FUZZ_ITERATIONS=200000 cargo test -p stellar-storage --test chunk_canonical_fuzz chunk_canonical_fuzz_campaign_has_no_findings -- --nocapture` | 종료 코드 0 |
| `cargo clippy -p stellar-storage --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-storage -- --check` | 종료 코드 0 |
| `cargo run -q -p xtask -- validate-dependencies` | 종료 코드 0 (새 의존성 없음) |
| `cargo run -q -p xtask -- validate-crate-graph` | 종료 코드 0 |
| `cargo run -q -p xtask -- validate-adr <변경 경로>` | 종료 코드 0 |

| 캠페인 | 실행 입력 | 발견(panic/hang/할당/canonical) | 시간 | 측정값 |
|---|---|---|---|---|
| 기본(25,000회) | 25,131 | **0** | 7.8 s | 제품 decoder 호출 peak 최대 9,699,332 B(1 MiB map entry stress), 256 KiB 초과 입력의 최대 비율 9.3 B/byte, 1 KiB 이상 입력 최대 비율 55.4 B/byte(1,036 B manifest — pointer 1,024개 선할당 고정 몫), 입력당 최대 0.24 s |
| 확장(200,000회) | 200,131 | **0** | 53.2 s | 같은 최대 peak·256 KiB 초과 비율, 1 KiB 이상 최대 비율 55.5 B/byte(manifest 선할당), 입력당 최대 0.24 s |

기본 캠페인의 대상별 결과(수용/거부): stream 1,253/23,878, envelope 1,593/23,538, pointer
264/24,867, manifest 56/25,075, view_tree 279/24,852, text 49/25,082, chunk_file 110/25,021.
`DecodeError` 12개 variant 전부, `EnvelopeError`의 BadMagic·UnsupportedVersion·InvalidHash·Decode,
manifest 의미 오류(ZeroGeneration·InvalidParent·DuplicateRootDocument·MisorderedPointers·
ConflictingChunkId·DuplicateChunkId), chunk 읽기의 KindMismatch·LengthMismatch·AddressMismatch·
PayloadHashMismatch 경로가 모두 실행되었습니다. 중첩 stream은 90,694회(수용 89,047) 디코딩되었습니다.

| 단위·회귀 검사 | 결과 |
|---|---|
| corpus 19개가 지시문의 기대 결과(ok/err)와 일치, panic·canonical 위반 없음 | 통과 |
| 다중 byte 문자 2·3·4 byte × hex 모든 위치: parse `InvalidHexDigit`, envelope·pointer 오류 | 통과 |
| `read_current_pointer` → `InvalidFormat`, `scan_latest_valid_generation` → `Ok(None)` | 통과 |
| `hash` 단위 테스트 `parse_rejects_multibyte_character_without_panic` | 통과 |
| 같은 seed 같은 입력열, 다른 seed 다른 입력열 | 통과 |
| 계측 할당자 자체 검사(1 MiB 할당 감지, 무할당 0) | 통과 |

## 차이(작업 지시와 다른 점)

- "fuzz target"을 cargo-fuzz/libFuzzer로 만들지 않았습니다(새 의존성과 nightly toolchain 필요).
  XVI.6.1과 같이 저장소 안의 결정론적 harness를 `#[test]`로 실행합니다. coverage-guided가 아닙니다.
- 검증 기준(panic 0)을 만족시키려고 fuzz가 찾은 결함을 V.1.5 `ContentHash` parse에서 수정했고,
  보호 경로 규칙에 따라 ADR-0015를 함께 추가했습니다.
- 대상을 envelope·canonical primitive decoder뿐 아니라 그 위에 만든 저장 decoder(pointer, manifest,
  ViewTree payload, hash·ChunkId 문자열, 디스크 chunk 읽기·기존 파일 검증 쓰기)까지 넓혔습니다.
  검증 기준(panic 0) 외에 hang·과도한 할당·canonical 왕복도 함께 판정합니다.
- 과도한 할당 판정을 위해 테스트 바이너리에만 `unsafe impl GlobalAlloc`(System 위임) 계측 할당자를
  두었습니다(XVI.6.1과 같음). 제품 코드에는 unsafe가 없습니다.
- 선행 V.2.6은 명시 행 ID이며 `[x]`입니다. 완화한 phase·gate 선행은 없습니다.
- 보고서 구조는 지시대로 `docs/task-reports/VI.9.5.md`를 따랐습니다.

## 남은 위험 / 한계

- coverage-guided fuzzing이 아니므로 발견력이 libFuzzer보다 낮습니다. 장기 실행과 crash artifact
  보관은 XVI.6.7(nightly job) 범위입니다.
- 문서 의미 payload decoder(document loader/validator, VI.10.x)는 아직 없어 이 harness 대상이 아닙니다
  (XVI.6.3 범위). ViewTree 외 chunk kind payload는 opaque bytes로만 다룹니다.
- `ChunkStore::read_chunk`·manifest 읽기는 파일 전체를 메모리로 읽고 크기 상한이 없습니다. 입력 크기에
  선형이라 이 harness의 할당 판정(입력 길이 기준)은 통과하지만, 매우 큰 파일에 대한 크기 정책
  (§37.2 깊이·개수 제한)은 별도 결정이 필요합니다.
- 할당 상한 계수(64 B/byte)는 이론 최대(map entry 48 B × 증설 3배 ÷ 최소 9 B ≈ 16)의 4배 여유입니다.
  decoder 자료 구조가 바뀌면 다시 측정해야 합니다. 시간 상한은 debug 빌드 wall-clock 기준입니다.
- 디스크 대상은 OS 임시 디렉터리에 입력마다 파일을 쓰고 지웁니다. 느린 파일시스템에서는 실행 시간이
  늘어납니다.
- 작업 시작 시 기준선 `cargo test -p stellar-storage`에서 이 작업과 무관한
  `gc::scheduler::tests::concurrent_saves_block_gc`(V.7.4)가 1회 실패했고, 이후 단독 재실행 5회와 최종
  검증 실행에서는 통과했습니다. 저장 guard가 실행 중 GC를 막지 않으므로 GC 시작 직후 시작된 저장을
  위반으로 세는 테스트 경쟁으로 보입니다. 이 작업에서는 수정하지 않았습니다.
