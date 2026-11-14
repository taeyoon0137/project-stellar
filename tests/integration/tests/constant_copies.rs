//! crate 의존 정책 때문에 두 crate가 따로 보관하는 같은 상수의 대조.
//!
//! 제품 crate graph(`tools/xtask/src/crate_graph.rs`)상 서로 의존할 수 없는 crate가 같은 계약 값을 각자 상수로
//! 둔 경우, 한쪽만 고치면 결과가 조용히 달라집니다. 이 정책 밖 crate는 두 crate를 함께 쓸 수 있으므로 여기서 값을
//! 비교합니다.
//!
//! - **환경 입력 key registry**(ADR-0026 3·4절): `stellar-target-profile` `environment`(IX.10.7)가 복제한 key
//!   1\~3, `platform` enum 타입 번호와 variant, `os_version` 인코딩을 `stellar-expressions` `environment`
//!   (VIII.7.1) registry와 대조합니다.
//! - **Display P3 ↔ sRGB 선형 행렬**: `stellar-expressions` 색 함수(VIII.4.4)와 `stellar-renderer-core` surface 색
//!   변환(XI.10.5)의 두 사본을 bit 단위로 대조합니다(두 crate의 기본값이 꺼진 `test-support` feature).

use stellar_core_types::typed_value::{EnumValue, TypedValue};
use stellar_expressions::ast::EnvironmentKey;
use stellar_expressions::environment::{
    self as registry, check_value, key_info, missing_default, EvaluationEnvironment,
};
use stellar_expressions::types::ExpressionType;
use stellar_target_profile::environment::{
    changed_keys, EnvironmentKeyCode, ProfileEnvironment, MAX_OS_MAJOR, MAX_OS_MINOR,
    PLATFORM_ENUM_TYPE, PLATFORM_GENERIC,
};
use stellar_target_profile::profile::{TargetProfileId, TargetProfileKind};
use stellar_target_profile::registry::ProfileRegistryVersion;
use stellar_target_profile::spec::{
    AndroidApiLevel, AndroidProfile, ApplePlatform, AppleProfile, CapabilitySetVersion,
    DottedVersion, ProfileSpec, TargetProfile, WebBaseline, WebProfile,
};

fn expression_key(code: EnvironmentKeyCode) -> EnvironmentKey {
    EnvironmentKey::new(code.0)
}

fn profile(id: &str, spec: ProfileSpec) -> TargetProfile {
    TargetProfile::new(
        TargetProfileId::new(id).expect("profile id"),
        ProfileRegistryVersion::parse("1").expect("registry version"),
        spec,
    )
}

fn apple(major: u16, minor: u16) -> TargetProfile {
    profile(
        "ios",
        ProfileSpec::Apple(AppleProfile {
            platform: ApplePlatform::Ios,
            minimum_os: DottedVersion::new(major, minor, 0),
        }),
    )
}

fn android(level: u16) -> TargetProfile {
    profile(
        "android",
        ProfileSpec::Android(AndroidProfile {
            api_level: AndroidApiLevel::new(level).expect("api level"),
            ui_system_version: DottedVersion::new(1, 7, 0),
        }),
    )
}

fn web() -> TargetProfile {
    profile(
        "web",
        ProfileSpec::Web(WebProfile {
            baseline: WebBaseline::new(2024).expect("baseline"),
            capability_set_version: CapabilitySetVersion::new(1).expect("capability set"),
        }),
    )
}

fn number(env: &ProfileEnvironment, key: EnvironmentKeyCode) -> f32 {
    match env.get(key) {
        Some(TypedValue::Number(value)) => value.get(),
        other => panic!("{key:?}가 Number가 아님: {other:?}"),
    }
}

#[test]
fn profile_environment_keys_match_expression_registry() {
    // key 번호와 값 타입.
    let expected = [
        (EnvironmentKeyCode::PLATFORM, registry::PLATFORM, "platform"),
        (
            EnvironmentKeyCode::OS_VERSION,
            registry::OS_VERSION,
            "os_version",
        ),
        (
            EnvironmentKeyCode::API_LEVEL,
            registry::API_LEVEL,
            "api_level",
        ),
    ];
    assert_eq!(
        EnvironmentKeyCode::PROFILE_KEYS,
        expected.map(|(code, _, _)| code),
        "profile key 목록(번호 오름차순)"
    );
    for (code, key, name) in expected {
        assert_eq!(expression_key(code), key, "{name} key 번호");
        let info = key_info(key).expect("등록 key");
        assert_eq!(info.name, name, "{name} 이름");
    }
    assert_eq!(
        key_info(registry::PLATFORM).map(|i| i.value_type),
        Some(ExpressionType::Enum(registry::PLATFORM_TYPE))
    );
    assert_eq!(
        key_info(registry::OS_VERSION).map(|i| i.value_type),
        Some(ExpressionType::Number)
    );
    assert_eq!(
        key_info(registry::API_LEVEL).map(|i| i.value_type),
        Some(ExpressionType::Number)
    );

    // platform enum 타입 번호와 variant.
    assert_eq!(PLATFORM_ENUM_TYPE, registry::PLATFORM_TYPE.raw());
    assert_eq!(PLATFORM_GENERIC, registry::platform::GENERIC);
    assert_eq!(
        u32::from(TargetProfileKind::Web.code()),
        registry::platform::WEB
    );
    assert_eq!(
        u32::from(TargetProfileKind::Apple.code()),
        registry::platform::APPLE
    );
    assert_eq!(
        u32::from(TargetProfileKind::Android.code()),
        registry::platform::ANDROID
    );
    assert_eq!(
        key_info(registry::PLATFORM).map(|i| i.variants),
        Some(4),
        "platform variant 수(generic·web·apple·android)"
    );
}

#[test]
fn profile_environment_values_pass_expression_registry_checks() {
    let cases = [
        (None, registry::platform::GENERIC),
        (Some(web()), registry::platform::WEB),
        (Some(apple(17, 4)), registry::platform::APPLE),
        (Some(android(35)), registry::platform::ANDROID),
    ];
    for (profile, variant) in cases {
        let env = ProfileEnvironment::for_profile(profile.as_ref());
        let mut evaluation = EvaluationEnvironment::new();
        for (code, value) in env.values() {
            let key = expression_key(*code);
            assert_eq!(
                check_value(key, value),
                Ok(()),
                "{code:?} 값이 registry 타입·variant 검사를 통과해야 함"
            );
            evaluation = evaluation.with(key, value.clone()).expect("환경 값");
        }
        assert_eq!(
            evaluation.value(registry::PLATFORM),
            Some(TypedValue::Enum(EnumValue::new(
                registry::PLATFORM_TYPE.raw(),
                variant
            )))
        );
    }

    // profile 없음은 expressions missing 기본값과 같습니다(ADR-0026 5절).
    let generic = ProfileEnvironment::for_profile(None);
    for code in EnvironmentKeyCode::PROFILE_KEYS {
        assert_eq!(
            generic.get(code).cloned(),
            missing_default(expression_key(code)),
            "{code:?} profile 없음 = missing 기본값"
        );
    }
}

#[test]
fn os_version_encoding_is_order_preserving_integer() {
    // ADR-0026 3절: Apple `minimum_os`의 `major × 1000 + minor`(patch 제외), 그 밖의 종류는 0.
    let ios_17_4 = ProfileEnvironment::for_profile(Some(&apple(17, 4)));
    assert_eq!(number(&ios_17_4, EnvironmentKeyCode::OS_VERSION), 17_004.0);
    assert_eq!(number(&ios_17_4, EnvironmentKeyCode::API_LEVEL), 0.0);

    let code = |major, minor| {
        number(
            &ProfileEnvironment::for_profile(Some(&apple(major, minor))),
            EnvironmentKeyCode::OS_VERSION,
        )
    };
    // 10진 소수 해석이면 같아지거나 뒤집히는 쌍.
    assert!(code(17, 10) > code(17, 1));
    assert!(code(10, 15) > code(10, 9));
    // 인코딩 상한(9999·999)의 최댓값도 f32로 정확합니다.
    assert_eq!(code(MAX_OS_MAJOR, MAX_OS_MINOR), 9_999_999.0);

    let droid = ProfileEnvironment::for_profile(Some(&android(35)));
    assert_eq!(number(&droid, EnvironmentKeyCode::OS_VERSION), 0.0);
    assert_eq!(number(&droid, EnvironmentKeyCode::API_LEVEL), 35.0);
    let browser = ProfileEnvironment::for_profile(Some(&web()));
    assert_eq!(number(&browser, EnvironmentKeyCode::OS_VERSION), 0.0);

    // 바뀐 key만 무효화 대상입니다(번호는 expressions registry와 같음).
    let changed: Vec<EnvironmentKey> = changed_keys(&ios_17_4, &droid)
        .into_iter()
        .map(expression_key)
        .collect();
    assert_eq!(
        changed,
        vec![
            registry::PLATFORM,
            registry::OS_VERSION,
            registry::API_LEVEL
        ]
    );
}

#[test]
fn display_p3_matrices_match_between_expressions_and_renderer() {
    let pairs = [
        (
            "P3_TO_SRGB",
            stellar_expressions::test_support::P3_TO_SRGB,
            stellar_renderer_core::test_support::P3_TO_SRGB,
        ),
        (
            "SRGB_TO_P3",
            stellar_expressions::test_support::SRGB_TO_P3,
            stellar_renderer_core::test_support::SRGB_TO_P3,
        ),
    ];
    for (name, expressions, renderer) in pairs {
        for (row, (a, b)) in expressions.iter().zip(renderer.iter()).enumerate() {
            for (col, (x, y)) in a.iter().zip(b.iter()).enumerate() {
                assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "{name}[{row}][{col}]: expressions {x} ≠ renderer-core {y}"
                );
            }
        }
    }
}
