//! texture cache key (XI.7.3).
//!
//! decode된 이미지 texture를 공유하는 cache의 key입니다.
//!
//! **key = [`AssetContentHash`] + [`DecodeSettings`] + [`ColorTransform`]**
//!
//! - **asset content hash**: asset 원본 bytes의 content hash(V.1.5, ADR-0004: SHA-256, 문자열
//!   `sha256:<64자리 소문자 16진수>`). 프로젝트는 asset을 `assets/`에 ContentHash 기반으로
//!   저장하므로(아키텍처 §29.1) 같은 bytes는 어떤 프로젝트 경로·`AssetRef`(III.2.3)·외부 원본
//!   경로로 들어와도 같은 hash를 갖습니다. key에는 경로·`AssetRef`·파일 이름을 **넣지 않으므로**
//!   같은 content는 하나의 entry를 공유합니다(source path가 달라도 dedup).
//!   crate 의존 정책(II.2.8)상 renderer-core는 storage를 import할 수 없어, algorithm tag +
//!   32-byte digest를 같은 의미로 담는 [`AssetContentHash`]를 둡니다. asset 계층은 storage
//!   `ContentHash`의 algorithm과 digest를 그대로 넘깁니다.
//! - **decode settings**: texture bytes를 바꾸는 decode 선택 — pixel format(XI.7.1)과 mipmap
//!   여부(XI.7.2). sampling mode(Linear/Nearest, VI.8.1)는 sampler state라 texture bytes를
//!   바꾸지 않으므로 제외합니다. decode 상한([`DecodeLimits`])은 성공 여부만 바꾸고 출력은
//!   바꾸지 않으며, 실패한 decode는 cache하지 않으므로 역시 제외합니다.
//! - **color transform**: decode된 RGB를 어떤 색공간으로 해석하는지([`SourceColorSpace`]: asset
//!   embedded 또는 Image node의 `color_space_override`, VI.8.1)와 texture를 저장할 working
//!   색공간(target: sRGB 또는 Display P3, 아키텍처 §10.3)입니다. 같은 content라도 해석·target이
//!   다르면 texture bytes가 다르므로 다른 entry입니다.
//!
//! key는 모두 고정 크기 `Copy` 값이라 `Eq`/`Hash`가 정확합니다(digest 충돌 개념 없음).
//! [`TextureCacheKey::canonical_bytes`]는 고정 37 bytes 배치로 fixture·진단에 씁니다.
//!
//! [`DecodeLimits`]: crate::image_decode::DecodeLimits

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use stellar_scene::ColorSpace;

use crate::digest::fnv1a64;
use crate::image_decode::DecodedPixelFormat;
use crate::tessellation::{CacheOutcome, CacheStats};

/// content hash 알고리즘(ADR-0004 registry와 같은 이름).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContentHashAlgorithm {
    /// SHA-256 (FIPS 180-4). 문자열 prefix `sha256`.
    Sha256,
}

impl ContentHashAlgorithm {
    /// 문자열 형식에 쓰는 안정 이름(ADR-0004).
    pub const fn name(self) -> &'static str {
        match self {
            ContentHashAlgorithm::Sha256 => "sha256",
        }
    }

    /// canonical bytes tag.
    const fn tag(self) -> u8 {
        match self {
            ContentHashAlgorithm::Sha256 => 1,
        }
    }
}

/// asset 원본 bytes의 content hash(V.1.5 `ContentHash`와 같은 의미).
///
/// 경로·`AssetRef`와 무관하게 bytes만으로 정해지는 값입니다. renderer는 hash를 계산하지 않고
/// asset 계층이 저장 시 계산한 값을 받습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetContentHash {
    algorithm: ContentHashAlgorithm,
    digest: [u8; 32],
}

impl AssetContentHash {
    /// SHA-256 digest로 만듭니다.
    pub const fn sha256(digest: [u8; 32]) -> AssetContentHash {
        AssetContentHash {
            algorithm: ContentHashAlgorithm::Sha256,
            digest,
        }
    }

    /// 알고리즘.
    pub const fn algorithm(&self) -> ContentHashAlgorithm {
        self.algorithm
    }

    /// digest bytes.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

impl fmt::Display for AssetContentHash {
    /// ADR-0004 문자열 형식 `sha256:<64자리 소문자 16진수>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.algorithm.name())?;
        f.write_str(":")?;
        for byte in &self.digest {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// mipmap 생성 여부(생성은 XI.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MipmapMode {
    /// base level만.
    None,
    /// 1×1까지 전체 mip chain(기본).
    #[default]
    Full,
}

impl MipmapMode {
    const fn tag(self) -> u8 {
        match self {
            MipmapMode::None => 0,
            MipmapMode::Full => 1,
        }
    }
}

/// texture bytes를 바꾸는 decode 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecodeSettings {
    /// decode 출력 pixel format(XI.7.1).
    pub format: DecodedPixelFormat,
    /// mipmap 여부.
    pub mipmaps: MipmapMode,
}

impl DecodeSettings {
    /// 기본: RGBA8 unorm + 전체 mip chain.
    pub const DEFAULT: DecodeSettings = DecodeSettings {
        format: DecodedPixelFormat::Rgba8Unorm,
        mipmaps: MipmapMode::Full,
    };
}

impl Default for DecodeSettings {
    fn default() -> DecodeSettings {
        DecodeSettings::DEFAULT
    }
}

/// decoded pixel format의 canonical tag.
const fn format_tag(format: DecodedPixelFormat) -> u8 {
    match format {
        DecodedPixelFormat::Rgba8Unorm => 1,
    }
}

/// decode된 RGB를 해석할 색공간.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SourceColorSpace {
    /// asset 자체 색공간(decoder header, 정보가 없으면 sRGB — XI.7.1).
    #[default]
    Embedded,
    /// Image node의 `color_space_override`(VI.8.1).
    Override(ColorSpace),
}

impl SourceColorSpace {
    /// Image payload의 `color_space_override` 값으로 만듭니다.
    pub const fn from_override(color_space_override: Option<ColorSpace>) -> SourceColorSpace {
        match color_space_override {
            Some(space) => SourceColorSpace::Override(space),
            None => SourceColorSpace::Embedded,
        }
    }

    /// 실제 해석 색공간. `embedded`는 decoder header의 색공간입니다.
    pub const fn resolve(self, embedded: ColorSpace) -> ColorSpace {
        match self {
            SourceColorSpace::Embedded => embedded,
            SourceColorSpace::Override(space) => space,
        }
    }

    /// canonical tag: Embedded는 0, Override는 core-types `ColorSpace::tag()`(0이 아님).
    const fn tag(self) -> u8 {
        match self {
            SourceColorSpace::Embedded => 0,
            SourceColorSpace::Override(space) => space.tag(),
        }
    }
}

/// 원본 해석 색공간 → texture 저장 working 색공간 변환.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorTransform {
    /// 원본 RGB 해석.
    pub source: SourceColorSpace,
    /// texture를 저장할 working 색공간.
    pub target: ColorSpace,
}

impl ColorTransform {
    /// 변환을 만듭니다.
    pub const fn new(source: SourceColorSpace, target: ColorSpace) -> ColorTransform {
        ColorTransform { source, target }
    }

    /// `embedded` 색공간 asset에 적용할 때 색공간 변환이 없으면 참입니다.
    pub const fn is_identity_for(&self, embedded: ColorSpace) -> bool {
        matches!(
            (self.source.resolve(embedded), self.target),
            (ColorSpace::Srgb, ColorSpace::Srgb) | (ColorSpace::DisplayP3, ColorSpace::DisplayP3)
        )
    }
}

/// texture cache key 길이(bytes).
pub const TEXTURE_KEY_BYTES: usize = 1 + 32 + 1 + 1 + 1 + 1;

/// texture cache key = asset content hash + decode settings + color transform.
///
/// 경로·`AssetRef`·sampling·decode 상한은 key가 아닙니다(모듈 문서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureCacheKey {
    content: AssetContentHash,
    settings: DecodeSettings,
    color: ColorTransform,
}

impl TextureCacheKey {
    /// key를 만듭니다.
    pub const fn new(
        content: AssetContentHash,
        settings: DecodeSettings,
        color: ColorTransform,
    ) -> TextureCacheKey {
        TextureCacheKey {
            content,
            settings,
            color,
        }
    }

    /// asset content hash.
    pub const fn content(&self) -> AssetContentHash {
        self.content
    }

    /// decode settings.
    pub const fn settings(&self) -> DecodeSettings {
        self.settings
    }

    /// color transform.
    pub const fn color(&self) -> ColorTransform {
        self.color
    }

    /// 고정 배치 canonical bytes.
    ///
    /// `[0]` algorithm tag, `[1..33]` digest, `[33]` pixel format tag, `[34]` mipmap tag,
    /// `[35]` source 색공간 tag(Embedded 0, Override는 `ColorSpace::tag()`), `[36]` target
    /// `ColorSpace::tag()`.
    pub fn canonical_bytes(&self) -> [u8; TEXTURE_KEY_BYTES] {
        let mut out = [0u8; TEXTURE_KEY_BYTES];
        out[0] = self.content.algorithm.tag();
        out[1..33].copy_from_slice(&self.content.digest);
        out[33] = format_tag(self.settings.format);
        out[34] = self.settings.mipmaps.tag();
        out[35] = self.color.source.tag();
        out[36] = self.color.target.tag();
        out
    }

    /// canonical bytes의 FNV-1a 64 digest(로그·fixture용, 동등성 판정에는 쓰지 않음).
    pub fn digest(&self) -> u64 {
        fnv1a64(&self.canonical_bytes())
    }
}

/// cache에서 얻은 texture와 조회 결과.
#[derive(Debug)]
pub struct CachedTexture<T> {
    /// 공유 texture.
    pub texture: Arc<T>,
    /// hit/miss.
    pub outcome: CacheOutcome,
}

/// content 기반 texture cache.
///
/// value 타입은 backend 중립입니다(CPU decode 결과, 이후 GPU handle 등). 실패한 생성은
/// 저장하지 않습니다(다음 조회에서 재시도). 예산·eviction은 XVI.5 범위입니다.
#[derive(Debug)]
pub struct TextureCache<T> {
    entries: HashMap<TextureCacheKey, Arc<T>>,
    hits: u64,
    misses: u64,
}

impl<T> Default for TextureCache<T> {
    fn default() -> TextureCache<T> {
        TextureCache {
            entries: HashMap::new(),
            hits: 0,
            misses: 0,
        }
    }
}

impl<T> TextureCache<T> {
    /// 빈 cache.
    pub fn new() -> TextureCache<T> {
        TextureCache::default()
    }

    /// key로 조회하고 hit/miss를 기록합니다.
    pub fn lookup(&mut self, key: &TextureCacheKey) -> Option<Arc<T>> {
        match self.entries.get(key) {
            Some(texture) => {
                self.hits = self.hits.saturating_add(1);
                Some(Arc::clone(texture))
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    /// texture를 저장하고 공유 handle을 반환합니다. 같은 key가 있으면 교체합니다.
    pub fn insert(&mut self, key: TextureCacheKey, texture: T) -> Arc<T> {
        let texture = Arc::new(texture);
        self.entries.insert(key, Arc::clone(&texture));
        texture
    }

    /// 조회하고, 없으면 `create`로 만들어 저장합니다. `create` 실패는 저장하지 않습니다.
    pub fn get_or_create<E>(
        &mut self,
        key: TextureCacheKey,
        create: impl FnOnce(&TextureCacheKey) -> Result<T, E>,
    ) -> Result<CachedTexture<T>, E> {
        if let Some(texture) = self.lookup(&key) {
            return Ok(CachedTexture {
                texture,
                outcome: CacheOutcome::Hit,
            });
        }
        let texture = create(&key)?;
        Ok(CachedTexture {
            texture: self.insert(key, texture),
            outcome: CacheOutcome::Miss,
        })
    }

    /// entry를 제거합니다.
    pub fn remove(&mut self, key: &TextureCacheKey) -> Option<Arc<T>> {
        self.entries.remove(key)
    }

    /// 모든 entry를 제거합니다(통계는 유지).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// entry가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 통계.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// FIPS 180-4 SHA-256 test vector: "abc".
    const SHA256_ABC: [u8; 32] = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];

    /// FIPS 180-4 SHA-256 test vector: "" (빈 입력).
    const SHA256_EMPTY: [u8; 32] = [
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ];

    /// FIPS 180-4 SHA-256 test vector: 448-bit 메시지.
    const MSG_448: &[u8] = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    const SHA256_448: [u8; 32] = [
        0x24, 0x8d, 0x6a, 0x61, 0xd2, 0x06, 0x38, 0xb8, 0xe5, 0xc0, 0x26, 0x93, 0x0c, 0x3e, 0x60,
        0x39, 0xa3, 0x3c, 0xe4, 0x59, 0x64, 0xff, 0x21, 0x67, 0xf6, 0xec, 0xed, 0xd4, 0x19, 0xdb,
        0x06, 0xc1,
    ];

    /// asset 계층 모사: bytes만으로 content hash를 정합니다(경로를 보지 않음).
    /// renderer-core는 SHA-256을 구현하지 않으므로 FIPS vector 입력만 다룹니다.
    fn content_hash_of(bytes: &[u8]) -> AssetContentHash {
        let digest = match bytes {
            b"abc" => SHA256_ABC,
            b"" => SHA256_EMPTY,
            other if other == MSG_448 => SHA256_448,
            _ => panic!("테스트 vector 밖 입력"),
        };
        AssetContentHash::sha256(digest)
    }

    /// 프로젝트 asset 하나: 경로·AssetRef id·원본 bytes.
    struct AssetRecord {
        source_path: &'static str,
        asset_ref: u128,
        bytes: &'static [u8],
    }

    /// Image node 하나의 texture 요청.
    struct ImageUse<'a> {
        asset: &'a AssetRecord,
        color_space_override: Option<ColorSpace>,
    }

    /// 테스트용 decode 결과.
    #[derive(Debug, PartialEq, Eq)]
    struct FakeTexture {
        content: AssetContentHash,
        first_path: &'static str,
    }

    fn key_for(image: &ImageUse<'_>, target: ColorSpace) -> TextureCacheKey {
        TextureCacheKey::new(
            content_hash_of(image.asset.bytes),
            DecodeSettings::DEFAULT,
            ColorTransform::new(
                SourceColorSpace::from_override(image.color_space_override),
                target,
            ),
        )
    }

    #[test]
    fn same_content_from_different_source_paths_dedups() {
        // 검증 기준(XI.7.3): source path가 달라도 같은 content dedup.
        let original = AssetRecord {
            source_path: "assets/photos/hero.png",
            asset_ref: 1,
            bytes: b"abc",
        };
        let copied = AssetRecord {
            source_path: "imports/marketing/hero-copy.png",
            asset_ref: 2,
            bytes: b"abc",
        };
        let other = AssetRecord {
            source_path: "assets/photos/hero.png.bak",
            asset_ref: 3,
            bytes: MSG_448,
        };
        assert_ne!(original.source_path, copied.source_path);
        assert_ne!(original.asset_ref, copied.asset_ref);

        let uses = [
            ImageUse {
                asset: &original,
                color_space_override: None,
            },
            ImageUse {
                asset: &copied,
                color_space_override: None,
            },
            ImageUse {
                asset: &original,
                color_space_override: None,
            },
            ImageUse {
                asset: &other,
                color_space_override: None,
            },
        ];

        let mut cache = TextureCache::new();
        let mut decodes = 0usize;
        let mut results = Vec::new();
        for image in &uses {
            let key = key_for(image, ColorSpace::Srgb);
            let cached = cache
                .get_or_create(key, |key| {
                    decodes += 1;
                    Ok::<_, ()>(FakeTexture {
                        content: key.content(),
                        first_path: image.asset.source_path,
                    })
                })
                .unwrap();
            results.push(cached);
        }

        assert_eq!(
            key_for(&uses[0], ColorSpace::Srgb),
            key_for(&uses[1], ColorSpace::Srgb)
        );
        assert_eq!(results[0].outcome, CacheOutcome::Miss);
        assert_eq!(results[1].outcome, CacheOutcome::Hit);
        assert_eq!(results[2].outcome, CacheOutcome::Hit);
        assert_eq!(results[3].outcome, CacheOutcome::Miss);
        // 다른 경로의 같은 content는 처음 decode한 texture를 그대로 공유합니다.
        assert!(Arc::ptr_eq(&results[0].texture, &results[1].texture));
        assert!(Arc::ptr_eq(&results[0].texture, &results[2].texture));
        assert_eq!(results[1].texture.first_path, "assets/photos/hero.png");
        assert!(!Arc::ptr_eq(&results[0].texture, &results[3].texture));
        assert_eq!(decodes, 2);
        assert_eq!(
            cache.stats(),
            CacheStats {
                hits: 2,
                misses: 2,
                entries: 2
            }
        );
    }

    #[test]
    fn each_key_component_changes_the_key() {
        let base = TextureCacheKey::new(
            AssetContentHash::sha256(SHA256_ABC),
            DecodeSettings::DEFAULT,
            ColorTransform::new(SourceColorSpace::Embedded, ColorSpace::Srgb),
        );
        let variants = [
            base,
            // content.
            TextureCacheKey::new(
                AssetContentHash::sha256(SHA256_EMPTY),
                base.settings(),
                base.color(),
            ),
            // decode settings: mipmap.
            TextureCacheKey::new(
                base.content(),
                DecodeSettings {
                    mipmaps: MipmapMode::None,
                    ..DecodeSettings::DEFAULT
                },
                base.color(),
            ),
            // color transform: source override.
            TextureCacheKey::new(
                base.content(),
                base.settings(),
                ColorTransform::new(
                    SourceColorSpace::Override(ColorSpace::DisplayP3),
                    ColorSpace::Srgb,
                ),
            ),
            TextureCacheKey::new(
                base.content(),
                base.settings(),
                ColorTransform::new(
                    SourceColorSpace::Override(ColorSpace::Srgb),
                    ColorSpace::Srgb,
                ),
            ),
            // color transform: target.
            TextureCacheKey::new(
                base.content(),
                base.settings(),
                ColorTransform::new(SourceColorSpace::Embedded, ColorSpace::DisplayP3),
            ),
        ];
        let keys: HashSet<TextureCacheKey> = variants.iter().copied().collect();
        assert_eq!(keys.len(), variants.len());
        // canonical bytes도 key마다 다릅니다(injective).
        let bytes: HashSet<[u8; TEXTURE_KEY_BYTES]> = variants
            .iter()
            .map(TextureCacheKey::canonical_bytes)
            .collect();
        assert_eq!(bytes.len(), variants.len());

        let mut cache = TextureCache::new();
        for (index, key) in variants.iter().enumerate() {
            let cached = cache.get_or_create(*key, |_| Ok::<_, ()>(index)).unwrap();
            assert_eq!(cached.outcome, CacheOutcome::Miss, "variant {index}");
        }
        // 같은 값으로 다시 만든 key는 hit.
        let rebuilt = TextureCacheKey::new(
            AssetContentHash::sha256(SHA256_ABC),
            DecodeSettings::default(),
            ColorTransform::new(SourceColorSpace::from_override(None), ColorSpace::Srgb),
        );
        let hit = cache.get_or_create(rebuilt, |_| Ok::<_, ()>(99)).unwrap();
        assert_eq!(hit.outcome, CacheOutcome::Hit);
        assert_eq!(*hit.texture, 0);
        assert_eq!(cache.len(), variants.len());
    }

    #[test]
    fn canonical_bytes_are_fixed() {
        let key = TextureCacheKey::new(
            AssetContentHash::sha256(SHA256_ABC),
            DecodeSettings::DEFAULT,
            ColorTransform::new(
                SourceColorSpace::Override(ColorSpace::DisplayP3),
                ColorSpace::Srgb,
            ),
        );
        let mut expected = vec![0x01];
        expected.extend_from_slice(&SHA256_ABC);
        expected.extend_from_slice(&[0x01, 0x01, 0x02, 0x01]);
        assert_eq!(key.canonical_bytes().as_slice(), expected.as_slice());
        assert_eq!(key.canonical_bytes().len(), TEXTURE_KEY_BYTES);
        assert_eq!(key.digest(), fnv1a64(&expected));

        let embedded = TextureCacheKey::new(
            AssetContentHash::sha256(SHA256_EMPTY),
            DecodeSettings {
                mipmaps: MipmapMode::None,
                ..DecodeSettings::DEFAULT
            },
            ColorTransform::new(SourceColorSpace::Embedded, ColorSpace::DisplayP3),
        );
        let bytes = embedded.canonical_bytes();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(&bytes[1..33], &SHA256_EMPTY);
        assert_eq!(&bytes[33..], &[0x01, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn content_hash_display_matches_adr_0004_format() {
        let hash = AssetContentHash::sha256(SHA256_ABC);
        assert_eq!(hash.algorithm(), ContentHashAlgorithm::Sha256);
        assert_eq!(hash.digest(), &SHA256_ABC);
        // storage `ContentHash::compute(b"abc")`의 문자열과 같은 형식·값.
        assert_eq!(
            hash.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash.to_string().len(), 71);
    }

    #[test]
    fn source_color_space_resolution() {
        let embedded = SourceColorSpace::from_override(None);
        assert_eq!(embedded, SourceColorSpace::Embedded);
        assert_eq!(
            embedded.resolve(ColorSpace::DisplayP3),
            ColorSpace::DisplayP3
        );
        let forced = SourceColorSpace::from_override(Some(ColorSpace::Srgb));
        assert_eq!(forced.resolve(ColorSpace::DisplayP3), ColorSpace::Srgb);

        let to_srgb = ColorTransform::new(SourceColorSpace::Embedded, ColorSpace::Srgb);
        assert!(to_srgb.is_identity_for(ColorSpace::Srgb));
        assert!(!to_srgb.is_identity_for(ColorSpace::DisplayP3));
        let p3_as_srgb = ColorTransform::new(
            SourceColorSpace::Override(ColorSpace::Srgb),
            ColorSpace::Srgb,
        );
        assert!(p3_as_srgb.is_identity_for(ColorSpace::DisplayP3));
    }

    #[test]
    fn failed_creation_is_not_cached() {
        let key = TextureCacheKey::new(
            AssetContentHash::sha256(SHA256_ABC),
            DecodeSettings::DEFAULT,
            ColorTransform::new(SourceColorSpace::Embedded, ColorSpace::Srgb),
        );
        let mut cache: TextureCache<u32> = TextureCache::new();
        let failed = cache.get_or_create(key, |_| Err("손상"));
        assert_eq!(failed.unwrap_err(), "손상");
        assert!(cache.is_empty());
        let retried = cache.get_or_create(key, |_| Ok::<_, &str>(7)).unwrap();
        assert_eq!(retried.outcome, CacheOutcome::Miss);
        assert_eq!(*retried.texture, 7);
        assert_eq!(cache.remove(&key).as_deref(), Some(&7));
        cache.insert(key, 8);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(
            cache.stats(),
            CacheStats {
                hits: 0,
                misses: 2,
                entries: 0
            }
        );
    }
}
