//! XVI.3.4 검증: sparse synthetic asset provider가 크기·stream 동작을 재현하고, 기준 장비
//! manifest가 generator와 일치합니다.
//!
//! 20 GiB 전체 stream은 `#[ignore]`이며 `cargo test -p stellar-bench-fixtures --release --test
//! asset_provider -- --ignored`로 실행합니다. CI 테스트는 같은 provider의 축소 set과 20 GiB set의
//! 무작위 접근(데이터 미생성)만 확인합니다.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use stellar_assets::media_type::MediaType;
use stellar_assets::origin::OriginKey;
use stellar_assets::policy::AssetImportPolicy;
use stellar_assets::store::AssetStore;
use stellar_bench_fixtures::asset_provider::{
    materialize, materialize_set, stream_digest, AssetFixtureError, AssetManifest,
    SyntheticAssetSet, ASSET_CLASSES, BLOCK_BYTES, REFERENCE_MANIFEST_PATH, REFERENCE_TOTAL_BYTES,
};
use stellar_storage::hash::ContentHash;

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REFERENCE_MANIFEST_PATH)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path =
            std::env::temp_dir().join(format!("stellar-bench-assets-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_all(reader: &mut impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    out
}

#[test]
fn reference_manifest_file_matches_generator() {
    let text = std::fs::read_to_string(manifest_path()).expect("manifest 파일");
    let manifest = AssetManifest::parse(&text).expect("manifest 해석");
    let set = manifest.verify().expect("재생성 결과와 일치");
    assert_eq!(set, SyntheticAssetSet::reference().unwrap());
    // 총량 20 GiB(정확히), 종류 5개 모두 포함.
    assert_eq!(manifest.total_bytes, REFERENCE_TOTAL_BYTES);
    assert_eq!(set.sum_bytes(), REFERENCE_TOTAL_BYTES);
    assert_eq!(manifest.assets, set.assets.len() as u64);
    assert_eq!(manifest.classes.len(), ASSET_CLASSES.len());
    assert!(manifest.classes.iter().all(|c| c.count > 0));
    // 저장소의 파일은 generator 출력과 byte 단위로 같습니다(바뀌면 `assets-manifest`로 재생성).
    assert_eq!(manifest.to_yaml().unwrap(), text);
}

#[test]
fn reference_set_is_sparse_and_randomly_accessible() {
    let set = SyntheticAssetSet::reference().unwrap();
    let largest = set.assets.iter().max_by_key(|a| a.size).unwrap();
    assert!(largest.size > 512 * 1024 * 1024, "대형 영상 에셋 포함");
    // 메모리는 입력 크기와 무관하게 block 하나.
    let mut reader = largest.reader();
    assert_eq!(reader.len(), largest.size);
    assert_eq!(reader.buffer_bytes(), BLOCK_BYTES);
    // 끝 부근을 바로 읽습니다(앞 내용을 만들지 않음).
    reader.seek(SeekFrom::End(-10)).unwrap();
    let tail = read_all(&mut reader);
    assert_eq!(tail.len(), 10);
    let mut again = largest.reader();
    again.seek(SeekFrom::Start(largest.size - 10)).unwrap();
    assert_eq!(read_all(&mut again), tail, "같은 offset은 같은 bytes");
    // 한 번의 read는 block 경계를 넘지 않습니다(bounded).
    let mut reader = largest.reader();
    reader
        .seek(SeekFrom::Start(BLOCK_BYTES as u64 - 5))
        .unwrap();
    let mut buf = vec![0u8; 3 * BLOCK_BYTES];
    assert_eq!(reader.read(&mut buf).unwrap(), 5);
    assert_eq!(reader.read(&mut buf).unwrap(), BLOCK_BYTES);
    // magic bytes로 시작합니다.
    let mut head = vec![0u8; 16];
    let mut first = largest.reader();
    first.read_exact(&mut head).unwrap();
    assert!(head.starts_with(largest.class().magic));
}

#[test]
fn small_set_streams_through_the_real_import_path() {
    // CI 축소 변형: 같은 provider, 총량 6 MiB.
    let total = 6 * 1024 * 1024 + 123;
    let set = SyntheticAssetSet::generate(7, total).unwrap();
    assert_eq!(set.sum_bytes(), total);
    assert_eq!(set, SyntheticAssetSet::generate(7, total).unwrap());
    assert_ne!(
        set.entries_digest(),
        SyntheticAssetSet::generate(8, total)
            .unwrap()
            .entries_digest()
    );

    let project = TempDir::new("import");
    let staging = project.0.join("staging");
    std::fs::create_dir_all(project.0.join("assets")).unwrap();
    // 에셋 보안 수정(XIII.5.x) 이후 공개 import 경로는 import 정책과 기기별 출처 key를 요구합니다.
    let store = AssetStore::new(&project.0).with_origin_key(OriginKey::generate().unwrap());
    let policy = AssetImportPolicy::from_config_file(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/asset-import.yaml"),
    )
    .unwrap();
    let mut seen_classes = std::collections::BTreeSet::new();
    for asset in &set.assets {
        if !seen_classes.insert(asset.class) {
            continue;
        }
        // provider stream hash = 전체 bytes hash = 실제 import(streaming hash + 복사) 결과.
        let streamed = stream_digest(asset).unwrap();
        assert_eq!(streamed.size, asset.size);
        let bytes = read_all(&mut asset.reader());
        assert_eq!(bytes.len() as u64, asset.size);
        assert_eq!(ContentHash::compute(&bytes), streamed.hash);
        let path = materialize(asset, &staging).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let imported = store
            .import_file(
                &policy
                    .request(&path, &MediaType::new(asset.media_type()).unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(imported.record.content_hash, streamed.hash);
        assert_eq!(imported.record.size, asset.size);
    }
    assert_eq!(seen_classes.len(), ASSET_CLASSES.len());

    // set 전체 materialize: 쓴 bytes = 총량, 비어 있지 않은 디렉터리는 거부.
    let out = TempDir::new("materialize");
    assert_eq!(materialize_set(&set, &out.0).unwrap(), total);
    assert!(matches!(
        materialize_set(&set, &out.0),
        Err(AssetFixtureError::InvalidSpec { .. })
    ));
}

#[test]
fn generation_is_exact_and_bounded() {
    for total in [1u64, 511, 12_345, 1 << 30] {
        let set = SyntheticAssetSet::generate(3, total).unwrap();
        assert_eq!(set.sum_bytes(), total);
        assert!(set.assets.iter().all(|a| a.size > 0));
        for asset in &set.assets {
            assert!(asset.size <= asset.class().max_bytes);
        }
    }
    assert!(SyntheticAssetSet::generate(1, 0).is_err());
    assert!(SyntheticAssetSet::generate(1, u64::MAX).is_err());
}

#[test]
fn manifest_parser_is_strict_and_detects_tampering() {
    let text = std::fs::read_to_string(manifest_path()).unwrap();
    let parse = |t: &str| AssetManifest::parse(t);

    // 형식 오류: 알 수 없는·누락 키, schema, 비정규 숫자, 중복 키, anchor, 과대 입력.
    let replaced = |from: &str, to: &str| text.replacen(from, to, 1);
    for bad in [
        format!("{text}extra: \"1\"\n"),
        replaced("block_bytes: \"65536\"\n", ""),
        replaced("stellar-bench-assets/1", "stellar-bench-assets/2"),
        replaced("seed: \"1\"", "seed: \"01\""),
        replaced("seed: \"1\"", "seed: \"-1\""),
        replaced("seed: \"1\"", "seed: \"1\"\nseed: \"2\""),
        replaced("environment: REF-MAC", "environment: &a REF-MAC"),
        "x".repeat(2 * 1024 * 1024),
    ] {
        assert!(
            matches!(parse(&bad), Err(AssetFixtureError::Manifest { .. })),
            "{}",
            &bad[..bad.len().min(80)]
        );
    }

    // 내용 변조: 형식은 맞지만 재생성 결과와 다르면 verify가 거부합니다.
    let tampered_digest = {
        let mut m = parse(&text).unwrap();
        m.entries_digest = ContentHash::compute(b"other");
        m
    };
    assert_eq!(
        tampered_digest.verify(),
        Err(AssetFixtureError::Mismatch {
            field: "entries_digest"
        })
    );
    let mut tampered = parse(&text).unwrap();
    tampered.classes[0].count += 1;
    assert_eq!(
        tampered.verify(),
        Err(AssetFixtureError::Mismatch { field: "classes" })
    );
    let mut tampered = parse(&text).unwrap();
    tampered.generator_version += 1;
    assert_eq!(
        tampered.verify(),
        Err(AssetFixtureError::Mismatch {
            field: "generator_version"
        })
    );
    let mut tampered = parse(&text).unwrap();
    tampered.total_bytes += 1;
    assert!(tampered.verify().is_err());
}

#[test]
#[ignore = "20 GiB 전체 stream: --release -- --ignored로 실행"]
fn reference_set_streams_fully_with_declared_sizes() {
    let set = SyntheticAssetSet::reference().unwrap();
    let mut total = 0u64;
    for asset in &set.assets {
        let digest = stream_digest(asset).unwrap();
        assert_eq!(digest.size, asset.size);
        total += digest.size;
    }
    assert_eq!(total, REFERENCE_TOTAL_BYTES);
}
