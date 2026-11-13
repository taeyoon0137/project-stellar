# 작업 보고서: XI.7.3 texture cache key(asset content hash, decode settings, color transform)

- WBS: XI.7.3
- 작업명: texture cache key를 asset content hash, decode settings, color transform으로 정의
- 상태: 완료 (`[x]`)
- 선행: V.1.5, XI.7.1 (`b703259`에서 모두 `[x]`)

## 필수 실행 환경

- `DEV-MAC` (순수 Rust, 로컬 검증).

## 변경 파일 목록

- `crates/renderer-core/src/texture_cache.rs`(신규) — `ContentHashAlgorithm`, `AssetContentHash`,
  `MipmapMode`, `DecodeSettings`, `SourceColorSpace`, `ColorTransform`, `TextureCacheKey`
  (`TEXTURE_KEY_BYTES`, canonical bytes, FNV-1a 64 digest), `TextureCache<T>`, `CachedTexture<T>`.
- `crates/renderer-core/src/lib.rs` — 모듈 등록.
- `docs/task-reports/XI.7.3.md`(신규), 계획서(XI.7.3 `[ ]`→`[x]`).

## 구현 요약

- **key 구성**: `TextureCacheKey = AssetContentHash + DecodeSettings + ColorTransform`. 경로,
  `AssetRef`, 파일 이름은 key에 넣지 않습니다. 프로젝트 asset은 ContentHash 기반으로
  저장되므로(아키텍처 §29.1) 같은 bytes는 어느 경로로 들어와도 같은 key가 되고 entry 하나를
  공유합니다.
- **asset content hash**: crate 의존 정책(II.2.8)상 renderer-core는 `stellar-storage`를 import할 수
  없어서, V.1.5 `ContentHash`와 의미가 같은 `AssetContentHash`(algorithm tag + 32-byte digest)를
  두었습니다. `Display`는 ADR-0004 문자열 형식 `sha256:<64자리 소문자 hex>`를 그대로 따릅니다.
  hash는 asset 계층이 계산해서 넘기고, renderer는 직접 계산하지 않습니다.
- **decode settings**: texture bytes를 바꾸는 값만 넣었습니다. pixel format(`Rgba8Unorm`, XI.7.1)과
  mipmap 여부(`MipmapMode::None`/`Full`, 생성은 XI.7.2)입니다. sampling mode는 sampler state라
  제외했습니다. decode 상한은 성공 여부만 바꾸고 출력은 바꾸지 않으며, 실패한 decode는
  cache하지 않으므로 역시 제외했습니다.
- **color transform**: 원본 RGB 해석(`SourceColorSpace::Embedded` 또는 Image node
  `color_space_override` → `Override`, VI.8.1)과 texture 저장 working 색공간(target: sRGB/Display
  P3)으로 구성합니다. 해석 결과와 target이 같은지는 `is_identity_for`로 확인합니다.
- **canonical bytes**: 37 bytes 고정 배치입니다. 순서는 algorithm tag, digest 32, format tag,
  mipmap tag, source tag(Embedded 0, Override는 core-types `ColorSpace::tag()`), target tag입니다.
  key 필드가 모두 고정 크기 `Copy` 값이라 `Eq`/`Hash`가 정확하고 digest 충돌 문제가 없습니다.
  digest는 로그와 fixture에만 씁니다.
- **cache**: backend 중립 generic `TextureCache<T>`를 두었습니다. CPU decode 결과를 담을 수 있고,
  이후에는 GPU handle도 담을 수 있습니다. `get_or_create`는 조회 후 miss면 생성해서 `Arc<T>`로
  저장하며, 생성에 실패하면 저장하지 않습니다. hit/miss 통계는 tessellation cache의
  `CacheOutcome`/`CacheStats`를 재사용합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo test -p stellar-renderer-core` | 30 passed, 0 failed; doctest 1 passed(compile_fail) |
| `cargo clippy -p stellar-renderer-core --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-renderer-core -- --check` | 통과 |
| `cargo run -q -p xtask -- validate-crate-graph` | exit 0 |
| `git diff --check` | 출력 없음 |

| 검사 | 결과 |
|---|---|
| 같은 bytes를 가진 서로 다른 경로·AssetRef의 asset 두 개가 같은 key를 만들고 첫 decode 결과(`Arc`)를 공유함. decode는 content별로 1회씩, 총 2회만 실행됨. 통계 hits 2, misses 2, entries 2(`same_content_from_different_source_paths_dedups`) | 통과 |
| content, mipmap, source override(P3/sRGB), target 중 하나라도 바꾸면 다른 key가 되고 canonical bytes도 서로 다름. 각각 miss이고, 같은 값으로 다시 만든 key는 hit(`each_key_component_changes_the_key`) | 통과 |
| canonical bytes 37 bytes golden과 digest 일치(`canonical_bytes_are_fixed`) | 통과 |
| FIPS "abc" vector의 문자열이 ADR-0004 형식·값과 같고 길이가 71자(`content_hash_display_matches_adr_0004_format`) | 통과 |
| Embedded/Override 해석, identity 판정(`source_color_space_resolution`) | 통과 |
| 생성 실패는 저장하지 않고 재시도함. remove/clear 동작과 통계가 맞음(`failed_creation_is_not_cached`) | 통과 |

## 차이(작업 지시와 다른 점)

- 선행 행은 모두 명시 row-ID이고 `[x]`라서 완화 적용은 없습니다.
- renderer-core는 SHA-256을 구현하지 않으므로, 테스트의 asset 계층 모사는 FIPS 180-4 공개 test
  vector(`"abc"`, `""`, 448-bit 메시지) 입력만 다룹니다. bytes → hash의 결정론은 V.1.5 테스트가
  이미 보장하고, 이 작업의 테스트는 key에 경로가 들어가지 않는다는 점과 cache 공유를 검증합니다.
- 실제 decode, mip 생성, upload는 연결하지 않았습니다(XI.7.2). value 타입은 generic입니다.

## 남은 위험 / 한계

- `Override(X)`가 asset의 embedded 색공간 X와 같으면 결과 texture는 `Embedded`와 같지만 key는
  다릅니다(보수적 miss). embedded 색공간은 probe 전에는 알 수 없으므로 정규화하지 않았습니다.
- cache에 예산과 eviction이 없습니다(XVI.5.2 priority 정책에서 추가 예정).
- asset 계층(`stellar-assets`)이 storage `ContentHash`를 `AssetContentHash`로 넘기는 변환은 아직
  없습니다. asset → Scene 연결 작업에서 추가하고, 그때 양쪽 문자열이 같은지 교차 검증해야
  합니다.
