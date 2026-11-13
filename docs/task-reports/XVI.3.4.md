# 작업 보고서: XVI.3.4 20GB 프로젝트 에셋 sparse synthetic provider와 기준 장비 manifest

- WBS: XVI.3.4
- 작업명: project asset 20GB를 CI에 포함하지 않고 sparse synthetic asset provider와 별도 reference device test data manifest로 구성
- 상태: 완료 (`[x]`)
- 선행: XIII.5.2 (`[x]`)

## 필수 실행 환경

- `DEV-MAC` (로컬 검증. 20 GiB 전체 stream·materialize도 같은 장비에서 실행했습니다. manifest의 대상 환경
  코드는 `REF-MAC`이며, 기준 장비 측정 자체는 XVI.4.6 몫입니다).

## 변경 파일 목록

- `benchmarks/fixtures/src/asset_provider.rs`(신규) — `ASSET_CLASSES`, `SyntheticAsset`, `SyntheticAssetReader`
  (`Read + Seek`), `SyntheticAssetSet`, `stream_digest`, `materialize`, `materialize_set`, `AssetManifest`
  (`to_yaml`·`parse`·`verify`), `AssetFixtureError`.
- `benchmarks/fixtures/data/reference-assets-20gib.yaml`(신규, 생성 결과 40줄) — 기준 장비 test data manifest.
  `cargo run -p stellar-bench-fixtures --release -- assets-manifest`의 출력입니다.
- `benchmarks/fixtures/src/cli.rs`, `benchmarks/fixtures/src/main.rs`(수정) — `assets-20gib`(요약, `--out <dir>`이면
  파일 생성), `assets-manifest`(manifest YAML 출력, 출력 전 재해석·검증), `Output`/`execute`.
- `benchmarks/fixtures/src/error.rs`, `benchmarks/fixtures/src/lib.rs`(수정) — `Asset` 오류, 모듈 등록.
- `benchmarks/fixtures/Cargo.toml`(수정) — 내부 crate `stellar-assets` path 의존 추가.
- `Cargo.lock`(수정, 생성 결과) — `stellar-bench-fixtures` 의존 목록에 `stellar-assets` 한 줄. 외부 crate 변경 없음.
- `benchmarks/fixtures/tests/asset_provider.rs`(신규) — provider·manifest 검증.
- `docs/task-reports/XVI.3.4.md`(신규), 계획서(XVI.3.4 `[ ]`→`[x]`).

## 구현 요약

- **sparse provider**: 에셋은 `(경로, class, 크기, seed)`뿐이고 내용은 읽을 때 64 KiB block을 seed와 block
  번호에서 즉석 생성합니다. 저장 공간을 쓰지 않고, 어떤 offset도 그 block만 만들어 바로 읽으며(seek),
  reader 메모리는 크기와 무관하게 block 하나입니다. read 한 번은 block 경계를 넘지 않습니다. 내용은
  media 종류의 magic bytes로 시작합니다(디코딩 가능한 media는 아님 — I/O·hash·복사 경로용).
- **asset set**: 20GB는 보수적으로 20 GiB(21,474,836,480 bytes)로 해석했습니다. class별 byte 비중(PNG 30%,
  JPEG 35%, SVG 1%, 폰트 4%, 영상 30%)과 크기 범위로 seed에서 목록을 만들고, 마지막 class가 나머지를 받아
  합이 요청 총량과 정확히 같습니다. 꼬리 에셋이 하한보다 작아지지 않게 조정합니다. 총량 1 byte..=1 TiB,
  에셋 20만 개 상한으로 오입력을 막습니다. 기준 set은 3,821개, 최대 1,062,594,454 bytes입니다.
- **manifest**: 에셋 목록 자체가 아니라 생성 매개변수(schema, generator version, 환경 `REF-MAC`, seed, 총량,
  block 크기), class별 개수·bytes·최대 크기, 전체 목록 digest(경로·media type·크기·seed의 SHA-256), 재현
  명령을 담은 40줄 YAML입니다. 제한 YAML writer/loader(IV.1, `stellar_project::yaml`)로 쓰고 읽습니다.
  `parse`는 정확한 키 집합·순서, 비정규 숫자(선행 0·부호) 거부, 1 MiB 입력 상한(loader)을 적용하고,
  `verify`는 현재 generator로 같은 목록을 재생성해 version·block 크기·환경·개수·class 요약·digest가 모두
  같아야 통과합니다.
- **실제 import 경로 연결**: `stream_digest`는 assets crate의 import streaming hasher(XIII.5.2, 같은 buffer
  크기)로 읽습니다. `materialize`/`materialize_set`은 block 단위로 파일을 쓰며(경로 component 화이트리스트,
  비어 있지 않은 출력 디렉터리 거부) 기준 장비의 파일 import benchmark를 준비합니다.

## 실행한 명령 / 검증 (`DEV-MAC`)

| 명령 | 결과 |
|---|---|
| `cargo fmt -p stellar-bench-fixtures` | 완료 |
| `cargo test -p stellar-bench-fixtures` | unit 13, `asset_provider` 5 passed(+1 ignored), `limit_fixtures` 3, `synthetic_scale` 2(+2 ignored), `workload_fixtures` 5 passed, 0 failed |
| `cargo test -p stellar-bench-fixtures --release --test asset_provider -- --ignored` | 1 passed(20 GiB 전체 stream, 3,821개 크기 일치, 합 21,474,836,480), 157.98s |
| `cargo clippy -p stellar-bench-fixtures --all-targets -- -D warnings` | 경고 0 |
| `cargo fmt -p stellar-bench-fixtures -- --check` | 통과 |
| `cargo check -p stellar-bench-fixtures --locked` | 통과 |
| `cargo run -q -p xtask -- validate-dependencies` | exit 0 |
| `git diff --check` | 통과 |
| `cargo run -q -p stellar-bench-fixtures --release -- assets-20gib` | 3,821 에셋, 21,474,836,480 bytes, png 1,515 / jpeg 580 / svg 1,623 / font 92 / video 11, digest `sha256:f9762859f553222a46f3fa9b6804634e038d19ecb1918f89f110885896afaa20`, 1 ms |
| `cargo run -q -p stellar-bench-fixtures --release -- assets-20gib --out /tmp/x` | 20 GiB 전체 파일 생성 완료(`du -sh` 20G). 출력 JSON은 확인하지 못했고, 생성물은 바로 삭제했습니다 |
| `cargo run -q -p stellar-bench-fixtures --release -- nodes-10k --out /tmp/x` | exit 2(`--out`은 `assets-20gib` 전용) |

| 검사 | 결과 |
|---|---|
| 저장소 manifest 해석·재생성 검증 통과, 총량 정확히 20 GiB, 5 class 모두 존재, generator 출력과 byte 단위 동일(`reference_manifest_file_matches_generator`) | 통과 |
| 1 GiB급 영상 에셋: reader 메모리 = block 하나, 끝 offset 즉시 읽기·같은 offset 같은 bytes, read가 block 경계에서 끊김, magic 시작(`reference_set_is_sparse_and_randomly_accessible`) | 통과 |
| 축소 set(6 MiB): 합 정확, seed 재현·다른 seed digest 다름, class마다 stream hash = 전체 bytes hash = 실제 `AssetStore::import_file` 결과 hash·크기, materialize 파일 = stream 내용, set materialize bytes = 총량, 비어 있지 않은 디렉터리 거부(`small_set_streams_through_the_real_import_path`) | 통과 |
| 1 byte·511·12,345·1 GiB 총량에서 합 정확·0 크기 없음·상한 이하, 0·과대 총량 거부(`generation_is_exact_and_bounded`) | 통과 |
| manifest: 추가·누락 키, schema, `01`·`-1` 숫자, 중복 키, anchor, 2 MiB 입력 거부, digest·class 개수·generator version·총량 변조를 verify가 거부(`manifest_parser_is_strict_and_detects_tampering`) | 통과 |

## 차이(작업 지시와 다른 점)

- "20GB"를 20 GiB로 해석했습니다(10진 20 GB보다 큰 보수적 값). manifest에 정확한 bytes를 기록합니다.
- manifest는 에셋 목록 전체가 아니라 생성 매개변수·요약·digest만 담습니다(40줄). 목록은 generator가
  재생성하고 digest로 일치를 확인합니다.
- 이 crate가 import streaming hasher와 `AssetStore`를 쓰도록 내부 path 의존 `stellar-assets`를 추가해
  `Cargo.lock`의 이 crate 항목이 한 줄 바뀌었습니다(생성 결과, 외부 의존성 변화 없음).
- 의도한 검증이 아니었지만 `--out /tmp/x` 실행으로 20 GiB 전체가 실제로 생성되는 것을 확인했고, 생성물은
  즉시 삭제했습니다(scratch 밖 경로를 쓴 실수).

## 남은 위험 / 한계

- 내용은 magic bytes + 난수라 이미지·폰트·영상 디코더 benchmark에는 쓸 수 없습니다. 디코딩 경로에는 별도의
  실제 sample set이 필요합니다.
- 20 GiB 전체 stream hash는 이 장비 release 빌드에서 약 158초(약 136 MB/s, 소프트웨어 SHA-256 위주)입니다.
  기준 장비 측정 시간 예산에 반영해야 합니다.
- class 비중·크기 범위는 일반적인 UI 디자인 프로젝트를 가정한 값입니다. 실제 사용자 프로젝트 분포를
  근거로 조정하면 `GENERATOR_VERSION`을 올리고 manifest를 다시 생성해야 합니다.
