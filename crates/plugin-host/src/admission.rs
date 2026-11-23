//! 플러그인 적재 gate (XXI.1.3, ADR-0097 결정 4).
//!
//! 서명·registry 검증은 [`SignatureGate`]로 분리합니다. 적재 판정은 [`admit`] 한 곳에서만 하고, Host session은
//! [`admit`]이 만든 [`AdmittedPlugin`]으로만 시작할 수 있습니다.
//!
//! | 서명 판정 | release | development |
//! |---|---|---|
//! | 검증됨 | 허용 | 허용 |
//! | unsigned | 거부 | 허용(개발용 표시) |
//! | invalid | 거부 | 거부 |
//!
//! development 모드는 호출자(앱)가 사용자의 명시 개발자 설정으로만 켭니다. [`LoadPolicy::default`]는 release입니다.

use std::fmt;

use crate::manifest::PluginManifest;
use crate::package::PluginPackage;
use crate::path::PackagePath;

/// package 안 서명 파일 이름(예약).
pub const SIGNATURE_FILE: &str = "plugin.sig";

/// 적재 모드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadMode {
    /// 배포 빌드 기본값: 검증된 서명만 받습니다.
    #[default]
    Release,
    /// 사용자가 켠 개발자 모드: unsigned local plugin도 받습니다.
    Development,
}

/// 적재 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoadPolicy {
    /// 모드.
    pub mode: LoadMode,
}

impl LoadPolicy {
    /// release 정책.
    pub fn release() -> LoadPolicy {
        LoadPolicy {
            mode: LoadMode::Release,
        }
    }

    /// 개발자 모드 정책(사용자의 명시 설정에서만 만듭니다).
    pub fn development() -> LoadPolicy {
        LoadPolicy {
            mode: LoadMode::Development,
        }
    }
}

/// 서명 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureVerdict {
    /// registry가 package digest의 서명을 확인했습니다.
    Verified {
        /// 확인한 registry 이름.
        registry: String,
    },
    /// 서명 파일이 없습니다.
    Unsigned,
    /// 서명이 있지만 확인할 수 없거나 틀렸습니다.
    Invalid,
}

/// 서명·registry 검증 gate. package digest([`PluginPackage::digest`])를 기준으로 판정합니다.
pub trait SignatureGate {
    /// package의 서명을 판정합니다.
    fn verify(&self, package: &PluginPackage) -> SignatureVerdict;
}

/// registry가 없는 gate: 서명 파일이 없으면 unsigned, 있으면 확인할 수 없으므로 invalid입니다.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSignatureRegistry;

impl SignatureGate for NoSignatureRegistry {
    fn verify(&self, package: &PluginPackage) -> SignatureVerdict {
        let signature = PackagePath::parse(SIGNATURE_FILE).expect("고정 이름");
        if package.files().contains_key(&signature) {
            SignatureVerdict::Invalid
        } else {
            SignatureVerdict::Unsigned
        }
    }
}

/// 적재된 플러그인의 신뢰 근거.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionTrust {
    /// 서명 확인.
    Signed {
        /// registry 이름.
        registry: String,
    },
    /// 개발자 모드의 unsigned local plugin.
    UnsignedDevelopment,
}

/// gate를 통과한 플러그인. [`admit`]으로만 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedPlugin {
    package: PluginPackage,
    trust: AdmissionTrust,
}

impl AdmittedPlugin {
    /// package.
    pub fn package(&self) -> &PluginPackage {
        &self.package
    }

    /// manifest.
    pub fn manifest(&self) -> &PluginManifest {
        self.package.manifest()
    }

    /// 신뢰 근거.
    pub fn trust(&self) -> &AdmissionTrust {
        &self.trust
    }
}

/// 적재 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    /// release 모드에서 unsigned package.
    UnsignedInRelease,
    /// 서명이 틀리거나 확인할 수 없음.
    InvalidSignature,
}

impl AdmissionError {
    /// 고정 진단 code.
    pub fn code(self) -> &'static str {
        match self {
            AdmissionError::UnsignedInRelease => "unsigned-plugin-in-release",
            AdmissionError::InvalidSignature => "invalid-plugin-signature",
        }
    }
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdmissionError::UnsignedInRelease => {
                f.write_str("서명하지 않은 플러그인은 개발자 모드에서만 적재할 수 있습니다")
            }
            AdmissionError::InvalidSignature => f.write_str("플러그인 서명을 확인할 수 없습니다"),
        }
    }
}

impl std::error::Error for AdmissionError {}

/// 정책과 서명 gate로 적재를 판정합니다(모듈 문서 표).
pub fn admit(
    package: PluginPackage,
    policy: LoadPolicy,
    gate: &dyn SignatureGate,
) -> Result<AdmittedPlugin, AdmissionError> {
    let trust = match (gate.verify(&package), policy.mode) {
        (SignatureVerdict::Verified { registry }, _) => AdmissionTrust::Signed { registry },
        (SignatureVerdict::Unsigned, LoadMode::Development) => AdmissionTrust::UnsignedDevelopment,
        (SignatureVerdict::Unsigned, LoadMode::Release) => {
            return Err(AdmissionError::UnsignedInRelease)
        }
        (SignatureVerdict::Invalid, _) => return Err(AdmissionError::InvalidSignature),
    };
    Ok(AdmittedPlugin { package, trust })
}
