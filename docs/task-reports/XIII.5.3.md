# 작업 보고서: XIII.5.3 content hash dedup

- WBS: XIII.5.3
- 작업명: 동일 content hash는 재복사하지 않고 새 AssetId가 같은 content를 참조
- 상태: 완료 (`[x]`)
- 선행: XIII.5.2

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 시스템 검증).

## 변경 파일 목록

- `crates/assets/src/store.rs` — `AssetStore::locate_content`, `ImportOutcome`,
  `ImportedAsset::outcome`, `ImportOperation::LocateContent`, import의 dedup 분기.
- `crates/assets/src/path.rs` — `AssetObjectPath::parse`·`has_hash`·`shard_dir_for`.
- `crates/assets/src/catalog.rs`(신규) — `AssetCatalog`, `CatalogError`.
- `crates/assets/src/lib.rs` — `catalog` 모듈 등록.
- `docs/task-reports/XIII.5.3.md`(신규), 계획서(XIII.5.3 `[ ]`→`[x]`).

## 구현 요약

- **dedup 조회**: hash pass 뒤 `assets/<prefix>/`에서 같은 content hash의 content 파일을
  찾습니다(`locate_content`). 이름은 `AssetObjectPath::parse`로 canonical 규칙에 정확히
  맞는 것만 인정합니다(temp `.…tmp-…`, 대문자 확장자·hex, prefix 불일치, 알 수 없는
  알고리즘, 다른 파일은 무시). 확장자는 무관하며 여러 개면 파일 이름 사전순 첫 번째를
  고릅니다(결정론). 크기가 다른 파일은 손상으로 보고 재사용하지 않습니다.
- **재복사 없음**: 같은 hash·크기의 파일이 있으면 copy pass(temp 쓰기·rename)를 아예
  수행하지 않고 `ImportOutcome::Deduplicated`와 기존 파일 경로를 돌려줍니다. 없거나
  손상이면 `Copied`(XIII.5.2 copy pass, 손상 파일은 atomic rename으로 교체).
- **새 AssetId 참조**: record에 AssetId가 없으므로 import마다 새 record를 만들고, 호출자가
  새 AssetId로 등록합니다. `AssetCatalog`(AssetId 정렬 `BTreeMap`)는 같은 content hash의
  여러 AssetId를 허용하고 `ids_for_content`·`content_hashes`(GC mark 입력, XIII.5.6)를
  제공합니다. AssetId 중복 등록·없는 id 제거는 명시 오류입니다. 한 AssetId를 지워도 공유
  content는 다른 AssetId가 계속 참조합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 68 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| dedup fixture: 다른 경로·이름·확장자(`.png`·`.jpeg`, 외부·내부)의 같은 bytes 3개 → 1회 `Copied`, 2회 `Deduplicated`, content 파일 1개, inode·mtime·내용 불변(재기록 없음), 새 AssetId 3개가 같은 hash 참조, 다른 bytes는 별도 파일(`same_content_is_stored_once_and_shared_by_new_asset_ids`) | 통과 |
| 같은 이름·다른 크기(손상) 파일은 재사용하지 않고 `Copied`로 교체(`damaged_existing_content_is_replaced_not_reused`) | 통과 |
| shard 없음·temp·무관 파일 무시, 여러 확장자 중 사전순 첫 번째(`locate_ignores_temp_foreign_and_missing_shards`) | 통과 |
| 비canonical 이름 8종 parse 거부, canonical round-trip(`parse_round_trips_canonical_names_only`) | 통과 |
| catalog 공유 content·정렬·제거 후 live 유지, 중복·없는 id 오류 | 통과 |

## 차이(작업 지시와 다른 점)

- 없음.

## 남은 위험 / 한계

- ~~dedup 판정은 이름(hash)과 크기만 확인하고 기존 파일 내용을 다시 hash하지 않습니다. 같은
  크기로 손상된 파일은 재사용될 수 있으며, 무결성 검사는 별도 검증 작업(예: open 시 sample
  hash)이 필요합니다.~~ → 후속 수정으로 재사용 전 재hash합니다(아래 절).
- 같은 content가 다른 확장자로 요청되면 기존 파일의 확장자를 그대로 씁니다. 파일 확장자가
  record의 media type과 다를 수 있으므로 소비자는 확장자가 아니라 record의 media type을
  기준으로 해석해야 합니다.
- `AssetCatalog`는 메모리 index이며 문서 저장·command 연결은 상위 계층 작업입니다.
- 동시에 두 import가 같은 content를 처음 쓰면 둘 다 `Copied`가 될 수 있습니다(같은 bytes의
  atomic rename이라 결과는 안전).

## 후속 수정(리뷰 반영): dedup 재사용 전 내용 재hash

- **문제**: `locate_content`가 canonical 이름과 크기만 보고 기존 파일을 재사용했습니다. 같은
  크기로 손상된 파일(bit rot, 협업자가 잘못 commit한 파일)이 그 hash의 content로 참조될 수
  있었습니다.
- **수정**: 이름·크기가 맞는 후보를 파일 이름 사전순으로 **다시 hash**해(import hash pass
  buffer 재사용, 64 KiB streaming) hash·크기가 모두 같은 첫 파일만 재사용합니다. 내용이 다르거나
  읽을 수 없는 후보는 손상으로 보고 건너뛰며, 확인된 파일이 없으면 copy pass로 새로 기록합니다
  (요청 확장자와 같은 이름의 손상 파일은 atomic rename으로 교체되어 복구). 공개
  `AssetStore::locate_content` 시그니처는 그대로이고 같은 검증을 합니다.
- **검증**: `same_size_corrupted_content_is_not_reused` — 1 bit만 바꾼 같은 크기 파일을
  `locate_content`가 거부, import는 `Copied`로 복구, 사전순으로 앞선 손상 `.bin` 후보를 건너뛰고
  확인된 `.png`를 `Deduplicated`로 재사용. 기존 dedup fixture(inode·mtime 불변)도 통과.
  `cargo test -p stellar-assets` lib 117 passed + integration 1 passed + doctest 4 passed,
  clippy 경고 0, fmt 통과, `validate-crate-graph` 통과.
- **남은 위험**: dedup hit마다 기존 파일을 한 번 더 읽습니다(대형 에셋은 시간 증가, 쓰기·fsync는
  여전히 생략). 요청과 다른 확장자의 손상 파일은 복구하지 않고 남깁니다(재사용 대상에서만 제외).
