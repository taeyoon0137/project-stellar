# 작업 보고서: XIII.5.5 Update from Source

- WBS: XIII.5.5
- 작업명: `UpdateFromSource` — source metadata 경로를 다시 읽어 새 hash asset을 만들고
  AssetRecord 참조를 command로 변경
- 상태: 부분 완료 (`[~]`) — 원본 재import·새 record·`AssetRecordUpdate`(apply/invert 값)와
  "source 없으면 기존 asset 유지"는 구현·검증했지만, 참조 변경을 **command로** 적용하는 연결은
  command 타입이 아직 없어(Phase VII, `stellar-commands`는 골격) 하지 않았습니다.
- 선행: XIII.5.2, Phase VII(phase 단위 선행 — 2026-11-14 사용자 지시에 따라 command 비의존
  API로 완화. 단 command 연결 자체는 Phase VII 이후 작업)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 파일 시스템 검증).

## 변경 파일 목록

- `crates/assets/src/update_source.rs`(신규) — `AssetStore::update_from_source`,
  `UpdateFromSourceOutcome`, `AssetRecordUpdate`(`inverted`), `SourceUnavailable`,
  `AssetCatalog::apply_update`.
- `crates/assets/src/catalog.rs` — `CatalogError::StaleUpdate`, crate 내부 `replace`.
- `crates/assets/src/lib.rs` — `update_source` 모듈 등록.
- `docs/task-reports/XIII.5.5.md`(신규), 계획서(XIII.5.5 `[ ]`→`[~]`).

## 구현 요약

- **원본 경로 결정**: 프로젝트 내부 원본은 record의 상대 경로를 project root에 붙여 씁니다.
  외부 원본은 절대 경로를 기록하지 않으므로(XIII.5.1 후속 수정) 호출자가 후보 경로를 주고,
  canonicalize한 경로의 출처 지문이 기록과 같아야 합니다(다르면 `OriginMismatch`).
- **source 없으면 유지**: metadata 없음(`NoSourceMetadata`)·외부 후보 경로 없음
  (`ExternalPathRequired`)·출처 불일치(`OriginMismatch`)·원본 없음(`NotFound`)·일반 파일
  아님(`NotAFile`)은 오류가 아니라 `SourceUnavailable` 결과이며 record·content 파일을 전혀
  바꾸지 않습니다. 확인과 재import 사이에 원본이 사라진 경우도 같은 결과로 바꿉니다.
- **재import**: XIII.5.2~5.4 경로(streaming hash·dedup·atomic write)를 그대로 씁니다. media
  type은 현재 record 값을 유지하고, source metadata는 같은 원본에서 다시 만듭니다(보존
  field 비상속 정책, XIII.5.1 후속 수정). hash·크기가 같으면 `Unchanged`.
- **command 경계**: 참조를 직접 바꾸지 않고 `Updated { update: AssetRecordUpdate { asset_id,
  previous, next }, object_path, import }`를 돌려줍니다. `inverted()`가 undo 값입니다.
  `AssetCatalog::apply_update`는 command apply/invert가 호출할 효과 함수로, 현재 record가
  `previous`와 같을 때만 적용합니다(아니면 `StaleUpdate`). 이전 content 파일은 undo 대상이라
  지우지 않습니다(참조가 끊긴 뒤 GC, XIII.5.6).

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-assets` | lib 84 passed + integration 1 passed + doctest 2 passed(compile_fail), 0 failed |
| `cargo clippy -p stellar-assets --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-assets -- --check` | 통과 |

| 검사 | 결과 |
|---|---|
| 내부 원본 변경 → 새 hash record, media type·source 유지, 새 content 기록, 이전 content 유지, invert 왕복(`project_source_change_produces_new_record_update`) | 통과 |
| 같은 내용 → `Unchanged`, 파일 변화 없음 | 통과 |
| **원본 삭제·디렉터리로 교체 → `SourceUnavailable`, record·content 파일·내용 불변**(`missing_source_keeps_existing_asset`) | 통과 |
| metadata 없는 record 유지 | 통과 |
| 외부 원본: 경로 미지정·다른 출처 거부, 일치 경로로 갱신, 삭제 시 유지(`external_source_requires_matching_candidate_path`) | 통과 |
| 이미 있는 content로 갱신 시 `Deduplicated` | 통과 |
| catalog apply → 재적용 `StaleUpdate` → invert 적용 → 원래 record, 없는 id 거부 | 통과 |

## 차이(작업 지시와 다른 점)

- 지시의 "AssetRecord 참조를 command로 변경"은 command 타입이 없어 구현하지 않았습니다.
  대신 command가 그대로 쓸 수 있는 변경 값(`AssetRecordUpdate`, invert 포함)과 낙관적 검사를
  하는 효과 함수(`AssetCatalog::apply_update`)까지만 제공합니다. 그래서 상태는 `[~]`입니다.

## 남은 위험 / 한계

- Phase VII에서 `AssetRecordUpdate`를 closed command enum(VII.1.2)의 variant로 감싸고
  validate/apply/invert와 history에 연결해야 `[x]`가 됩니다.
- 외부 원본의 후보 경로를 어디서 얻을지(파일 선택 UI, Git 밖 로컬 캐시 등)는 셸·project
  계층의 결정입니다. \~\~기록된 지문은 비밀이 아니므로 후보 경로가 맞는지만 확인합니다.\~\~
  → 정정: 지문은 기기별 비밀 key의 HMAC입니다(아래 "후속 수정: 기기 key 출처 지문").
- media type 허용·최대 크기 정책은 XIII.5.7에서 import 경로에 연결합니다(이 작업 시점에는
  미적용).
- undo가 가능한 동안 이전 content가 GC되지 않도록 history가 참조하는 record도 GC mark 입력에
  넣어야 합니다(XIII.5.6 문서에 명시).

## 후속 수정(리뷰 반영): 내부 원본 symlink 탈출 차단

- **문제**: 프로젝트 내부 원본(`SourceLocation::Project`)은 canonicalize 뒤 project root
  안에 있는지 확인하지 않았습니다. record는 Git에서 온 신뢰할 수 없는 입력이므로, 저장소 안
  symlink(예: `link → ../home/.ssh`)와 `link/id_rsa` 같은 record로 `Update from Source`가
  프로젝트 밖 파일을 `assets/`로 복사할 수 있었고, 새 record의 위치도 조용히 외부로 바뀌었습니다.
- **수정**: `resolve_source`가 canonicalize한 원본이 canonical project root 안(구성 요소 단위
  `starts_with`)이 아니면 `SourceUnavailable::OutsideProject`를 돌려주고 아무 것도 읽거나
  쓰지 않습니다. 방어 심층화로 내부 원본 재import 요청에 crate 내부 플래그
  (`ImportRequest::requiring_project_source`)를 붙여, import가 쓰기 전에 다시 계산한 위치가
  외부면 `SourceMetadataError::OutsideProject`로 실패합니다(확인 뒤 경로가 바뀌는 경우).
  새 record는 기록된 상대 경로를 그대로 유지합니다(내부 → 외부 전환 없음, 보존 field 비상속).
- **검증**: `project_source_symlink_escaping_project_is_rejected`(디렉터리·파일 symlink 모두
  `OutsideProject`, `assets/` 비어 있음), `import_guard_rejects_outside_source_before_writing`,
  `project_source_symlink_inside_project_keeps_recorded_location`. `cargo test -p stellar-assets`
  lib 100 passed + integration 1 passed + doctest 2 passed, clippy 경고 0, fmt 통과,
  `validate-crate-graph` 통과.
- **남은 위험**: 확인과 파일 열기 사이에 경로 구성 요소를 symlink로 바꾸는 동시 공격(TOCTOU)은
  `openat`/`O_NOFOLLOW` 없이 완전히 막지 못합니다. 위협 모델(커밋된 symlink)은 차단합니다.

## 후속 수정(리뷰 반영): 기기 key 출처 지문과 다른 기기 원본

- 외부 원본 지문이 기기별 비밀 key의 HMAC으로 바뀌었습니다(XIII.5.1 후속 수정 2차).
  `resolve_source`는 store의 key로 지문을 확인합니다.
  - 같은 key·같은 경로: 기존과 같이 갱신. 같은 key·다른 경로: `OriginMismatch`.
  - **다른 key**(다른 기기에서 기록, 또는 key 교체): 어떤 후보 경로로도 확인할 수 없으므로
    후보 경로 유무와 관계없이 `SourceUnavailable::OriginUnverifiable`을 돌려줍니다. 셸은
    사용자에게 원본 파일을 다시 선택받아 새 `AssetStore::update_from_selected_source`
    (정책 버전 `update_from_selected_source_with_policy`)를 호출합니다. 명시 선택이므로
    지문 확인 없이 쓰고, 새 record에는 이 기기 key의 지문을 기록합니다. record·Git·캐시에서
    온 경로를 이 API에 넘기지 않도록 문서에 명시했습니다.
  - store에 key가 없으면 외부 원본 갱신은 `ImportError::SourceMetadata(OriginKey(
    Unconfigured))` 오류입니다.
- 검증: `external_source_from_other_machine_requires_user_selection`(정확한 원래 경로로도
  `OriginUnverifiable`, 파일 변경 없음 → 사용자 선택으로 갱신 → 이후 이 기기 후보 경로로 확인,
  없는 경로·디렉터리 선택은 `NotFound`·`NotAFile`), `external_source_without_configured_key_is_an_error`.

## 후속 수정(리뷰 반영): 정책 필수 인자

- `update_from_source(asset_id, current, external_source, policy)`와
  `update_from_selected_source(asset_id, current, selected, policy)`가 import 정책을 필수로
  받습니다. 정책 없는 변형과 `update_from_source_with_policy`·
  `update_from_selected_source_with_policy`는 제거했습니다(XIII.5.7 후속 수정).

## 후속 수정(보안 리뷰 반영): 내부 원본 경로 경계 — 보호 영역과 호출자 정책

- **문제**: 내부 원본 판정이 "canonical project root 안"뿐이었습니다. commit된(신뢰할 수 없는)
  record가 `.git/config`(credential URL)·Git이 무시하는 `.env` 같은 root 안 로컬 비밀을 원본으로
  지정하면 `Update from Source`가 그 내용을 tracked `assets/`로 복사했습니다(리뷰 재현:
  `SourceMetadata::project(".git/config")` → `Updated`, credential이 든 content 파일 생성).
- **수정** (`crates/assets/src/project_source.rs` 신규, `update_source.rs`·`store.rs` 변경):
  - **고정 거부**(`protected_project_path`·`protected_area`): 기록 경로(파일 시스템 접근 전)와
    canonicalize한 해석 경로(project root 기준) 양쪽에서 어느 위치든 `.git` 구성 요소,
    첫 구성 요소 `design/`·`.stellar/`(Stellar 저장 디렉터리), `assets/`를 거부합니다
    → `SourceUnavailable::ProtectedPath(ProtectedArea)`. 비교는 ASCII 대소문자 무시, HFS+ 무시
    code point(U+200C 등) 제거, 끝 `.`·공백 제거 뒤 합니다. 프로젝트 안 symlink로 보호 영역을
    가리키는 경우도 해석 경로에서 걸립니다.
  - **호출자 정책 필수**: `update_from_source`가 `project_sources: &dyn ProjectSourcePolicy`를
    받습니다. 고정 거부를 통과한 내부 원본은 정책이 `ProjectSourceDecision::Allow`를 돌려줘야
    읽고, 아니면 `SourceUnavailable::DeniedByPolicy`입니다. 정책에는 기록 경로·해석 경로·
    canonical 절대 경로를 넘깁니다(`ProjectSourceCandidate`). 신뢰되지 않은 프로젝트용
    `DenyProjectSources`(모두 거부)를 제공하고, project 계층이 신뢰 매트릭스와
    `git check-ignore`를 이 trait로 연결합니다(이 crate는 Git을 실행하지 않음, 판정 실패는
    `Deny`로 처리하도록 문서화).
  - **판정 경로 고정**: import 요청에 판정을 통과한 해석 상대 경로를 넘기고
    (`requiring_project_source(approved)`), import 시점에 다른 내부 파일로 해석되면
    `ImportError::SourceChanged`, 밖이면 `OutsideProject`로 아무 것도 쓰지 않습니다.
  - 사용자가 직접 고른 경로(첫 import, `update_from_selected_source`)와 외부 원본(기기 key
    지문)은 이 판정 밖이며, 그 경계를 모듈 문서에 적었습니다.
- **검증**: `committed_record_cannot_copy_git_metadata_or_storage_into_assets`(`.git/config`·
  `.git/HEAD`·`.GIT/config`·`.g\u{200c}it/config`·중첩 `vendor/lib/.git/config`·`design/`·
  `Design/`·`.stellar/`·기존 `assets/` content 모두 `ProtectedPath`, 허용 정책이어도 거부, 정책
  미호출, `assets/` 불변), `project_symlink_into_protected_area_is_rejected`(파일·디렉터리
  symlink로 `.git`·`design/` 지정), `project_sources_require_explicit_policy_allow`
  (`DenyProjectSources`는 일반 이미지도 거부, check-ignore 모사 정책이 symlink 해석 경로
  `.env`를 거부, 허용 경로만 복사), `import_guard_rejects_outside_source_before_writing`(판정 경로와
  다른 내부 파일은 `SourceChanged`), `project_source` 모듈 단위 테스트 4건과 doctest 1건.
  `cargo test -p stellar-assets` lib 129 passed + integration 1 passed + doctest 8 passed,
  `cargo clippy -p stellar-assets --all-targets -- -D warnings` 경고 0, `cargo fmt -p
  stellar-assets -- --check` 통과, `cargo check -p stellar-project` 통과.
- **남은 위험**: project 계층의 실제 정책(신뢰 매트릭스·`git check-ignore`) 연결은 아직 없습니다.
  NTFS 8.3 짧은 이름·ADS 표기는 다루지 않습니다(Windows 지원 보류). 판정과 열기 사이 경로 교체
  (TOCTOU)는 별도 후속 수정(S4)에서 다룹니다.

## 후속 수정(보안 리뷰 반영): 판정한 원본 항목 고정(TOCTOU 완화)

- 기록된 원본(내부·외부)은 판정 때 canonical 경로를 `lstat`해 일반 파일인지 확인하고 그 identity를
  import 요청에 고정합니다(`pinned_to`). import는 경로 전체 symlink를 따라가지 않고 열며(macOS
  `O_NOFOLLOW_ANY`) 연 파일이 판정한 항목일 때만 읽으므로, 판정(보호 영역·호출자 정책·출처 지문) 뒤
  경로를 바꿔도 다른 파일을 `assets/`로 복사하지 않습니다(`ImportError::SourceChanged`). 사용자가
  고른 경로(`update_from_selected_source`)는 고정하지 않습니다. 세부와 남은 한계는 XIII.5.2 후속 수정
  (TOCTOU 완화) 절에 있습니다.
