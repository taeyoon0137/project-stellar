# 작업 보고서: XIII.5.2 외부 파일 streaming import

- WBS: XIII.5.2
- 작업명: 외부 파일 import 시 streaming hash를 계산하고 `assets/<prefix>/<hash>.<safe-ext>`로 복사
- 상태: 완료 (`[x]`)
- 선행: XIII.5.1

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 시스템 검증).

## 변경 파일 목록

- `crates/assets/src/hash_stream.rs`(신규) — `StreamingHasher`, `hash_reader_with`,
  `StreamDigest`, `StreamHashError`, `STREAM_BUFFER_SIZE`.
- `crates/assets/src/path.rs`(신규) — `AssetObjectPath`, `SafeExtension`, `ExtensionError`,
  `ASSETS_DIR`.
- `crates/assets/src/store.rs`(신규) — `AssetStore::import_file`, `ImportRequest`,
  `ImportedAsset`, `ImportError`, `ImportOperation`.
- `crates/assets/src/lib.rs` — `hash_stream`·`path`·`store` 모듈 등록.
- `crates/assets/tests/import_memory.rs`(신규) — heap 최고치 측정 integration test.
- `docs/task-reports/XIII.5.2.md`(신규), 계획서(XIII.5.2 `[ ]`→`[x]`).

## 구현 요약

- **streaming hash**: ADR-0004 SHA-256을 증분 계산하는 `StreamingHasher`를 assets에
  두었습니다(상태 32 bytes + 미완성 block 64 bytes 고정). storage의 `ContentHash::compute`는
  전체 slice만 받고 SHA-256 구현이 storage 내부(private)라 재사용할 수 없기 때문입니다.
  결과는 0~300 byte 모든 길이 × 8가지 분할에서 `ContentHash::compute`와 같고, NIST 예제
  vector와도 일치합니다. digest → `ContentHash`는 storage가 공개한 정규 문자열 parse로만
  만들 수 있어 `finish()`가 `Result`를 반환합니다(프로덕션 경로 panic 금지, II.1.3).
- **경로**: `assets/<digest hex 앞 2자>/sha256-<hex64>.<safe-ext>`. chunk object 경로(V.2.3)와
  같은 규칙으로 `ContentHash`·검증된 `SafeExtension`(`[a-z0-9]` 1~16자, 소문자 정규화)으로만
  만들어 path traversal이 타입상 불가능합니다. 원본 확장자가 없거나 규칙 밖이면 `bin`입니다.
- **import(2-pass)**: ① hash pass — 64 KiB buffer로 읽으며 hash·크기 계산, ② copy pass —
  같은 buffer로 같은 shard 디렉터리의 temp(`.<file>.tmp-<pid>-<n>`)에 복사하며 다시 hash.
  두 pass의 hash·크기가 다르면(import 도중 원본 변경) temp를 지우고 `SourceChanged`로
  실패합니다. 검증된 temp만 최종 이름으로 rename합니다. 실패 경로에서는 temp를 정리합니다.
  hash를 먼저 확정하므로 최종 경로를 복사 전에 알 수 있고, dedup(XIII.5.3)이 copy pass를
  건너뛸 수 있습니다.
- **원본 위치**: optional입니다(`without_source_metadata`). 기록 시 XIII.5.1 후속 수정의
  `SourceMetadata::from_import_path`(프로젝트 상대 경로 또는 파일 이름+출처 지문)를 쓰며,
  기록할 수 없는 경로면 아무 것도 쓰기 전에 `ImportError::SourceMetadata`로 실패합니다.
  디렉터리 원본은 `NotAFile`입니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 62 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 16 MiB 파일 import 중 heap 최고치 증가 66,773 bytes ≤ 상한 1 MiB(전용 counting global allocator, `large_file_import_does_not_load_whole_file_into_memory`) | 통과 |
| 합성 reader가 받은 최대 읽기 요청 = 64 KiB 고정, `Interrupted` 재시도(`reader_is_consumed_with_fixed_buffer`) | 통과 |
| streaming hash = `ContentHash::compute`(모든 길이·분할), NIST vector | 통과 |
| 외부 원본 복사·내용 일치·record(hash·size·media type·파일 이름+지문)·temp 잔존 없음 | 통과 |
| 내부 원본 상대 경로 기록, metadata 생략, 확장자 정규화(`PNG`→`png`, 없음→`bin`) | 통과 |
| 0·1·63·64·65·64Ki±1 byte 파일 hash·내용 | 통과 |
| 디렉터리·없는 원본은 `assets/`에 아무 것도 쓰지 않음, pass 사이 원본 변경 시 최종 파일·temp 없음 | 통과 |
| golden 경로(`SHA-256("abc")`), 모든 경로 문자 `[a-z0-9/.-]` 고정 형태 | 통과 |

## 차이(작업 지시와 다른 점)

- 경로의 `<hash>`는 ADR-0004 문자열의 `:`를 `-`로 바꾼 `sha256-<hex64>`입니다(Windows 금지
  문자 회피, V.2.3 chunk 경로와 동일). 아키텍처 문서 폴더 예시(`assets/sha256/<prefix>/...`)
  와는 다르고 계획서 형식(`assets/<prefix>/<hash>.<safe-ext>`)과 storage 선례를 따랐습니다.
- SHA-256 증분 구현이 storage 내부 구현과 중복됩니다(storage를 이 lane에서 수정할 수 없음).
  두 구현의 일치는 테스트로 고정했고, storage가 증분 hasher를 공개하면 그쪽으로 옮겨야 합니다.

## 남은 위험 / 한계

- 이 작업의 temp → rename은 내구성(fsync·부모 디렉터리 sync)과 단계별 fault injection을
  아직 포함하지 않습니다. XIII.5.4에서 storage V.2.5와 같은 protocol로 보강합니다.
- 같은 content의 재import는 아직 매번 copy pass를 수행합니다(dedup은 XIII.5.3).
- 원본을 두 번 읽으므로 대형 파일 import 시간은 1-pass 대비 약 2배입니다(pass 간 변경 검출과
  dedup 시 복사 생략을 위한 선택).
- media type 허용 목록·최대 크기 검사는 XIII.5.7에서 연결합니다.

## 후속 수정(리뷰 반영): symlink `assets/`·shard 디렉터리 거부

- **문제**: import의 copy pass가 `create_dir_all(assets/<prefix>)` 뒤 그 안에 썼으므로, Git으로
  commit된 symlink `assets/` 또는 shard 디렉터리가 content 쓰기를 프로젝트 밖으로 돌릴 수
  있었습니다. dedup 조회도 symlink shard를 따라가 밖의 파일을 읽었습니다.
- **수정**: `assets/`와 shard를 `lstat`(`path::assets_dir_state`)으로 한 단계씩 확인합니다.
  symlink면 따라가지 않고 새 typed 오류 `ImportError::SymlinkedAssetsDirectory { path }`로
  아무 것도 읽거나 쓰지 않습니다. 없으면 `create_dir`로 한 단계씩 만들고(동시 생성 경합은 다시
  확인), 디렉터리가 아니면 `Io { CreateDirectory, NotADirectory }`입니다.
  `AssetStore::locate_content`의 반환 타입은 `Result<_, ImportError>`로 바뀌었습니다(같은 typed
  오류, I/O는 `LocateContent`).
- **검증**: `symlinked_assets_root_or_shard_is_never_followed`(store) — symlink shard 안의 같은
  content를 dedup으로 쓰지 않고 `locate_content`·import·copy pass 모두 typed 오류, 밖 디렉터리에
  아무 것도 쓰지 않음, symlink `assets/`도 같은 오류, 실제 디렉터리로 되돌리면 정상 import.
  `cargo test -p stellar-assets` lib 120 passed + integration 1 passed + doctest 7 passed,
  clippy 경고 0, fmt 통과, `validate-crate-graph` 통과.
- **남은 위험**: 확인과 쓰기 사이에 디렉터리를 symlink로 바꾸는 동시 공격(TOCTOU)은 `openat`
  계열 없이 완전히 막지 못합니다. project root 자체의 symlink는 사용자가 연 경로이므로 허용합니다.
  project root가 없으면 이제 `assets/`를 만들지 않고 `CreateDirectory` I/O 오류입니다.
