# 작업 보고서: XIII.6.2 macOS system font discovery adapter

- WBS: XIII.6.2
- 작업명: macOS system font discovery adapter를 구현하고 실제 file/face를 FontRegistry에 등록(프로젝트에 복사하지 않음)
- 상태: 완료 (`[x]`)
- 선행: X.8.2(`[x]`), II.5.3(`[x]`), XIII.6.1(이 lane에서 완료 — sfnt probe 재사용)

## 필수 실행 환경

- `DEV-MAC` (CoreText smoke는 `cfg(target_os = "macos")`, discovery 규칙 테스트는 플랫폼 독립 합성 fixture).
  검증 기기: macOS 27.2.

## 변경 파일 목록

- `crates/project/src/fonts/system.rs`(신규) — `SystemFontLocator`, `SystemFontLocateError`,
  `SystemFontLimits`, `SystemFontDiscovery`, `SystemFontCatalog`, `SystemFontFace`,
  `SkippedSystemFont`, `SystemFontSkipReason`.
- `crates/project/src/fonts/system_macos.rs`(신규, macOS 전용) — `CoreTextFontLocator`(CoreText·
  CoreFoundation framework FFI 직접 선언).
- `crates/project/src/fonts/mod.rs` — `system`, `system_macos` 모듈 추가.
- `docs/task-reports/XIII.6.2.md`(신규), 계획서(XIII.6.2 `[ ]`→`[x]`).

## 구현 요약

- **CoreText locator**: `CTFontManagerCopyAvailableFontURLs`의 URL을 `CFURLGetFileSystemRepresentation`
  으로 경로로 바꿉니다. `locate_postscript_name`은 `CTFontCreateWithName`이 고른 font의 PostScript
  이름이 요청과 정확히 같을 때만 `kCTFontURLAttribute` 경로를 돌려줍니다(CoreText 대체 font를
  받아들이지 않음). 새 crate 없이 framework C 함수만 선언했고, CF 객체는 소유 wrapper가 drop 시
  한 번 해제합니다. 모든 `unsafe`에 SAFETY 주석을 달았습니다.
- **discovery**(플랫폼 독립): 경로를 정렬·중복 제거한 순서로, 파일마다 같은 열린 handle로 streaming
  SHA-256(`stellar-assets` hash_stream)과 container probe(XIII.6.1 `sfnt`)를 계산하고 face마다
  `FontSource::System`, `FontFaceKey(hash, index)`로 등록합니다. 읽기 전후 크기·수정 시각이 다르면
  건너뜁니다. 같은 bytes가 두 경로에 있으면 같은 handle(dedup)입니다.
- **복사 없음**: discovery는 프로젝트 root나 `AssetStore`를 받지 않고, catalog는 원래 경로를
  기록합니다.
- **비용 제한·cache**: 파일 수 16384, 파일 크기 1 GiB 상한. 크기·수정 시각이 같은 파일은 재hash하지
  않습니다(마지막 discovery 경로만 보관하는 메모리 cache). 지원하지 않는 형식(예: `.dfont`)·I/O
  오류·registry 상한은 파일 단위 skip 이유로 보고합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-project` | 종료 코드 0 |
| `cargo test -p stellar-project` | 종료 코드 0 (unit 277 passed, 1 ignored, 0 failed; 통합 3·5·5·4 passed) |
| `cargo clippy -p stellar-project --all-targets -- -D warnings` | 종료 코드 0 (경고 0) |
| `cargo fmt -p stellar-project -- --check` | 종료 코드 0 |
| `cargo test -q -p stellar-project --lib fonts::system` 3회 반복 | 3회 모두 9 passed, 1 ignored |
| `cargo test -p stellar-project --release --lib -- --ignored full_system_scan --nocapture` | 1 passed. 파일 548개 hash, face 932개(고유 932) 등록, skip 0, 첫 scan 12.97s, cache 재scan 31.6ms |

| fixture | 결과 |
|---|---|
| **알려진 system face 조회 smoke**: 후보(Helvetica, Menlo-Regular, Geneva, Courier) 중 첫 발견 face의 파일이 CoreText 가용 목록에 있고, 등록된 face가 같은 PostScript 이름·원래 경로·실제 bytes SHA-256·System 출처·registry 조회 일치, 파일 metadata 불변(`known_system_face_is_registered_from_its_original_file`). 이 환경에 `/System/Library/Fonts/Helvetica.ttc`가 있습니다 | 통과 |
| CoreText 목록이 비어 있지 않고 모두 절대 경로(`core_text_lists_absolute_font_files`) | 통과 |
| 없는 이름·빈 이름·NUL·과도한 길이는 대체 없이 `None`(`unknown_or_invalid_names_are_not_substituted`) | 통과 |
| 합성 TTF·TTC를 원래 위치에서 등록, 순서·중복 무관, 디렉터리에 새 파일 없음(`system_files_are_registered_in_place_as_system_faces`) | 통과 |
| 같은 bytes 두 경로 → 같은 handle(`same_bytes_at_two_paths_share_one_face`) | 통과 |
| skip 이유: 상대 경로·없음·디렉터리·크기 초과·폰트 아님(`unusable_files_are_skipped_with_reasons`) | 통과 |
| 파일 수 상한·registry 상한(`file_limit_and_registry_capacity_are_enforced`) | 통과 |
| 변경 없는 파일 재hash 없음, 변경 시 재hash·다른 face(`unchanged_files_are_not_rehashed`) | 통과 |
| locator 실패 전달(`locator_failure_is_reported`) | 통과 |

## 차이(작업 지시와 다른 점)

- 없음. face 이름은 CoreText가 아니라 공통 sfnt 파서로 읽어 플랫폼과 무관하게 같은 결과를 냅니다.

## 남은 위험 / 한계

- 전체 scan 첫 실행이 release에서 약 13초(약 1 GB, 548 파일)입니다. 시작 경로에서 동기로 돌리면
  성능 예산을 넘을 수 있으므로, 연결 시 background 작업과 디스크 cache(경로·크기·수정 시각 → hash)가
  필요합니다. 성능 예산 측정은 이 행 범위 밖입니다.
- cache 판정은 크기·수정 시각만 봅니다. 같은 크기·같은 수정 시각으로 내용만 바뀐 파일은 재hash하지
  않습니다(시스템 폰트 갱신은 보통 수정 시각을 바꿉니다).
- face 수는 파일 단위로 registry에 등록되므로 registry 상한 도중 실패하면 앞 face 일부가 registry에
  남을 수 있습니다(catalog에는 넣지 않음).
- Windows adapter(XIII.6.3)는 ADR-0006으로 `[!]`입니다.

## 후속 수정(리뷰 반영): cache가 같은 크기·보존된 mtime 교체를 놓치지 않음

- **문제**: cache 판정이 (크기, 수정 시각)만 봐서, 같은 크기로 다시 쓰고 mtime을 보존한 파일(`cp -p`,
  `touch -r`, 일부 installer)은 이전 hash를 그대로 써 다른 내용에 이전 `FontFaceKey`를 등록했습니다.
- **수정**(`crates/project/src/fonts/system.rs`): unix에서 cache stamp에 device·inode·ctime(초·나노초)을
  더합니다. ctime은 내용·metadata 변경 시 커널이 갱신하고 사용자 공간에서 되돌릴 수 없으며, rename 교체는
  inode가 바뀝니다. 비-unix(ADR-0006 보류)는 기존 판정 그대로입니다.
- **검증**: `same_size_rewrite_with_preserved_mtime_is_rehashed`(같은 크기 재기록 + `set_modified`로 mtime
  복원 → 재hash, 새 hash face). 실제 system scan(release, `--ignored full_system_scan`, 이 수정 포함):
  548 파일, 932 face, skip 0, 첫 scan 8.50 s, cache 재scan 34.7 ms(재hash 0, 확장 stamp로도 cache 적중).
- **남은 한계**: registry 상한 도중 실패 시 앞 face가 registry에 남는 문제는 `FontRegistry`에 해제 API가
  없어(text crate 범위) 그대로입니다.
