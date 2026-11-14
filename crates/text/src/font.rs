//! 공통 font face record. 기존 shaping API 경로와 동일한 타입입니다.
pub use stellar_core_types::font::*;

#[cfg(test)]
pub(crate) mod tests {
    use stellar_core_types::value::FiniteF32;

    use super::*;

    /// 테스트용 axis tag.
    pub(crate) fn tag(s: &str) -> OpenTypeTag {
        OpenTypeTag::parse(s).expect("4글자 ASCII tag")
    }

    /// 테스트용 axis 설정.
    pub(crate) fn axis(s: &str, value: f32) -> FontVariation {
        FontVariation {
            tag: tag(s),
            value: FiniteF32::new(value).expect("유한 값"),
        }
    }

    /// 모든 byte가 `fill`인 테스트용 file hash.
    pub(crate) fn hash(fill: u8) -> FontFileHash {
        FontFileHash::from_sha256_digest([fill; 32])
    }

    /// 빈 입력의 SHA-256(FIPS 180-4 공개 test vector). 호출자가 계산한 값을 받는 경로 확인용.
    const EMPTY_SHA256_HEX: &str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sample_ref() -> FontFaceRef {
        FontFaceRef::new(
            FontSource::Project,
            hash(0xAB),
            2,
            vec![axis("wght", 700.0), axis("opsz", 14.0)],
        )
    }

    #[test]
    fn source_tags_are_stable_golden() {
        // source tag는 저장 포맷 계약입니다. 이 golden이 바뀌면 기존 파일과 호환이 깨집니다.
        let tags: Vec<(FontSource, u8, &str)> = FontSource::ALL
            .iter()
            .map(|s| (*s, s.tag(), s.name()))
            .collect();
        assert_eq!(
            tags,
            vec![
                (FontSource::Project, 1, "project"),
                (FontSource::System, 2, "system"),
                (FontSource::External, 3, "external"),
            ]
        );
        for source in FontSource::ALL {
            assert_eq!(FontSource::from_tag(source.tag()), Some(source));
        }
        assert_eq!(FontSource::from_tag(0), None);
        assert_eq!(FontSource::from_tag(4), None);
    }

    #[test]
    fn face_ref_round_trips_for_every_source() {
        for source in FontSource::ALL {
            for variations in [
                vec![],
                vec![axis("wght", 350.5)],
                vec![axis("wdth", 87.5), axis("wght", 700.0), axis("slnt", -12.0)],
            ] {
                let face = FontFaceRef::new(source, hash(0x5C), 7, variations);
                let bytes = face.encode();
                assert_eq!(FontFaceRef::decode(&bytes), Ok(face.clone()));
                // 재인코딩도 같은 bytes(canonical).
                assert_eq!(FontFaceRef::decode(&bytes).expect("decode").encode(), bytes);
            }
        }
    }

    #[test]
    fn face_ref_golden_bytes() {
        let face = FontFaceRef::new(
            FontSource::System,
            hash(0x11),
            1,
            vec![axis("wght", 1.0), axis("opsz", -2.0)],
        );
        let mut expected = vec![0x02];
        expected.extend_from_slice(&[0x11; 32]);
        expected.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // face index 1
        expected.extend_from_slice(&[0x02, 0x00, 0x00, 0x00]); // axis 2개
        expected.extend_from_slice(b"opsz");
        expected.extend_from_slice(&[0x00, 0x00, 0x00, 0xC0]); // -2.0f32 LE
        expected.extend_from_slice(b"wght");
        expected.extend_from_slice(&[0x00, 0x00, 0x80, 0x3F]); // 1.0f32 LE
        assert_eq!(face.encode(), expected);
    }

    #[test]
    fn constructor_canonicalizes_variation_order_and_duplicates() {
        let a = FontFaceRef::new(
            FontSource::Project,
            hash(1),
            0,
            vec![axis("wght", 400.0), axis("opsz", 12.0), axis("wght", 700.0)],
        );
        let b = FontFaceRef::new(
            FontSource::Project,
            hash(1),
            0,
            vec![axis("opsz", 12.0), axis("wght", 700.0)],
        );
        assert_eq!(a, b);
        assert_eq!(a.encode(), b.encode());
        assert_eq!(a.variations(), &[axis("opsz", 12.0), axis("wght", 700.0)]);
        // -0.0은 0.0으로 정규화되어 같은 bytes.
        let neg = FontFaceRef::new(FontSource::Project, hash(1), 0, vec![axis("slnt", -0.0)]);
        let pos = FontFaceRef::new(FontSource::Project, hash(1), 0, vec![axis("slnt", 0.0)]);
        assert_eq!(neg.encode(), pos.encode());
    }

    #[test]
    fn decode_rejects_truncated_and_trailing_input() {
        let bytes = sample_ref().encode();
        for len in 0..bytes.len() {
            assert_eq!(
                FontFaceRef::decode(&bytes[..len]),
                Err(FontRecordError::Truncated),
                "prefix 길이 {len}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            FontFaceRef::decode(&trailing),
            Err(FontRecordError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn decode_rejects_non_canonical_records() {
        let base = sample_ref().encode();
        // 알 수 없는 source tag.
        for bad in [0u8, 4, 0xFF] {
            let mut bytes = base.clone();
            bytes[0] = bad;
            assert_eq!(
                FontFaceRef::decode(&bytes),
                Err(FontRecordError::UnknownSourceTag { tag: bad })
            );
        }
        // 첫 axis는 offset 41(tag 1 + hash 32 + index 4 + count 4)에서 시작합니다.
        let first_axis = 41;
        // 정렬 위반: 두 axis 순서를 바꿉니다.
        let mut swapped = base.clone();
        let (head, tail) = swapped.split_at_mut(first_axis + 8);
        head[first_axis..].swap_with_slice(&mut tail[..8]);
        assert_eq!(
            FontFaceRef::decode(&swapped),
            Err(FontRecordError::NonCanonicalVariations)
        );
        // 중복 tag: 두 번째 axis tag를 첫 번째와 같게 만듭니다.
        let mut duplicate = base.clone();
        let first_tag: [u8; 4] = duplicate[first_axis..first_axis + 4]
            .try_into()
            .expect("4바이트");
        duplicate[first_axis + 8..first_axis + 12].copy_from_slice(&first_tag);
        assert_eq!(
            FontFaceRef::decode(&duplicate),
            Err(FontRecordError::NonCanonicalVariations)
        );
        // 비유한 axis 값(NaN, +Inf).
        for bits in [f32::NAN.to_bits(), f32::INFINITY.to_bits()] {
            let mut bytes = base.clone();
            bytes[first_axis + 4..first_axis + 8].copy_from_slice(&bits.to_le_bytes());
            assert_eq!(
                FontFaceRef::decode(&bytes),
                Err(FontRecordError::NonFiniteAxisValue { tag: tag("opsz") })
            );
        }
        // 제어 문자 axis tag.
        let mut control = base.clone();
        control[first_axis] = 0x00;
        assert_eq!(
            FontFaceRef::decode(&control),
            Err(FontRecordError::InvalidAxisTag {
                bytes: [0x00, b'p', b's', b'z']
            })
        );
    }

    #[test]
    fn decode_rejects_negative_zero_axis_value() {
        // 생성자는 -0.0을 +0.0으로 정규화하므로 canonical bytes에는 -0.0(00 00 00 80)이 없습니다.
        // decode가 이를 조용히 +0.0으로 바꿔 받아들이면 같은 값이 두 bytes 형태를 갖게 됩니다.
        let face = FontFaceRef::new(FontSource::Project, hash(3), 0, vec![axis("slnt", 1.0)]);
        let mut bytes = face.encode();
        bytes[45..49].copy_from_slice(&(-0.0f32).to_le_bytes());
        assert_eq!(&bytes[45..49], &[0x00, 0x00, 0x00, 0x80]);
        assert_eq!(
            FontFaceRef::decode(&bytes),
            Err(FontRecordError::NonCanonicalAxisValue { tag: tag("slnt") })
        );
        // +0.0은 canonical이며 round-trip합니다.
        bytes[45..49].copy_from_slice(&0.0f32.to_le_bytes());
        let decoded = FontFaceRef::decode(&bytes).expect("+0.0은 canonical");
        assert_eq!(decoded.encode(), bytes);
        assert!(FontRecordError::NonCanonicalAxisValue { tag: tag("slnt") }
            .to_string()
            .contains("slnt"));
    }

    #[test]
    fn decode_with_huge_axis_count_fails_without_large_allocation() {
        let mut bytes = FontFaceRef::new(FontSource::External, hash(9), 0, vec![]).encode();
        bytes[37..41].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(FontFaceRef::decode(&bytes), Err(FontRecordError::Truncated));
    }

    #[test]
    fn file_hash_string_round_trips() {
        let parsed: FontFileHash = format!("sha256:{EMPTY_SHA256_HEX}")
            .parse()
            .expect("유효한 hash");
        assert_eq!(parsed.to_string(), format!("sha256:{EMPTY_SHA256_HEX}"));
        assert_eq!(parsed.sha256_digest()[0], 0xE3);
        assert_eq!(parsed.sha256_digest()[31], 0x55);
        let back: FontFileHash = parsed.to_string().parse().expect("round-trip");
        assert_eq!(back, parsed);
        assert_eq!(parsed.to_string().len(), 71);
        assert_eq!(
            format!("{:?}", hash(0)),
            format!("FontFileHash(sha256:{})", "0".repeat(64))
        );
    }

    #[test]
    fn file_hash_parse_rejects_invalid_strings() {
        assert_eq!(
            EMPTY_SHA256_HEX.parse::<FontFileHash>(),
            Err(ParseFontFileHashError::MissingSeparator)
        );
        assert_eq!(
            format!("blake3:{EMPTY_SHA256_HEX}").parse::<FontFileHash>(),
            Err(ParseFontFileHashError::UnknownAlgorithm {
                name: "blake3".to_string()
            })
        );
        assert_eq!(
            "sha256:abcd".parse::<FontFileHash>(),
            Err(ParseFontFileHashError::InvalidLength { actual: 4 })
        );
        assert_eq!(
            format!("sha256:{}", EMPTY_SHA256_HEX.to_uppercase()).parse::<FontFileHash>(),
            Err(ParseFontFileHashError::InvalidHexDigit { found: 'E' })
        );
        assert_eq!(
            format!("sha256:{}é", &EMPTY_SHA256_HEX[..63]).parse::<FontFileHash>(),
            Err(ParseFontFileHashError::InvalidHexDigit { found: 'é' })
        );
    }

    #[test]
    fn open_type_tag_validation_and_display() {
        assert_eq!(tag("wght").to_string(), "wght");
        assert_eq!(format!("{:?}", tag("opsz")), "OpenTypeTag(opsz)");
        assert!(OpenTypeTag::parse("wgh").is_none());
        assert!(OpenTypeTag::parse("wghtx").is_none());
        assert!(OpenTypeTag::new([0x00, b'a', b'b', b'c']).is_none());
        assert!(OpenTypeTag::new([0x7F, b'a', b'b', b'c']).is_none());
        assert!(OpenTypeTag::parse("é12").is_none());
        // OpenType은 짧은 tag를 공백으로 채우므로 공백을 포함한 tag도 유효합니다.
        assert!(OpenTypeTag::parse("ss1 ").is_some());
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert!(FontRecordError::Truncated.to_string().contains("부족"));
        assert!(FontRecordError::NonFiniteAxisValue { tag: tag("wght") }
            .to_string()
            .contains("wght"));
        assert!(ParseFontFileHashError::InvalidLength { actual: 3 }
            .to_string()
            .contains("64"));
    }
}
