//! 적재 gate fixture (XXI.1.3): release 모드 unsigned load 거부.

use std::path::Path;

use stellar_ops::test_support::TempDir;
use stellar_plugin_host::admission::{
    admit, AdmissionTrust, LoadMode, LoadPolicy, NoSignatureRegistry, SignatureGate,
    SignatureVerdict, SIGNATURE_FILE,
};
use stellar_plugin_host::package::PluginPackage;
use stellar_plugin_host::HOST_API_VERSION;

fn fixture() -> PluginPackage {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.stellar-plugin");
    PluginPackage::open(&root, HOST_API_VERSION).unwrap()
}

/// package digest가 기대값과 같으면 검증됨으로 보는 테스트 registry.
struct PinnedRegistry(String);

impl SignatureGate for PinnedRegistry {
    fn verify(&self, package: &PluginPackage) -> SignatureVerdict {
        if package.digest().to_string() == self.0 {
            SignatureVerdict::Verified {
                registry: "test".to_owned(),
            }
        } else {
            SignatureVerdict::Invalid
        }
    }
}

#[test]
fn default_policy_is_release() {
    assert_eq!(LoadPolicy::default().mode, LoadMode::Release);
}

#[test]
fn release_rejects_unsigned_local_plugin() {
    let error = admit(fixture(), LoadPolicy::release(), &NoSignatureRegistry).unwrap_err();
    assert_eq!(error.code(), "unsigned-plugin-in-release");
}

#[test]
fn development_admits_unsigned_local_plugin() {
    let admitted = admit(fixture(), LoadPolicy::development(), &NoSignatureRegistry).unwrap();
    assert_eq!(admitted.trust(), &AdmissionTrust::UnsignedDevelopment);
    assert_eq!(admitted.manifest().id.as_str(), "xyz.taeyoon.fixture.hello");
}

#[test]
fn signature_without_registry_is_rejected_in_every_mode() {
    let dir = TempDir::new("plugin-signed");
    let root = dir.path.join("s.stellar-plugin");
    std::fs::create_dir_all(root.join("ui")).unwrap();
    std::fs::write(
        root.join("plugin.yaml"),
        "manifest_version: 1\nid: a.b\nversion: 1.0.0\nname: S\nentry: ui/i.html\nhost_api: 1\npermissions: []\n",
    )
    .unwrap();
    std::fs::write(root.join("ui/i.html"), "x").unwrap();
    std::fs::write(root.join(SIGNATURE_FILE), "sig").unwrap();
    let package = PluginPackage::open(&root, HOST_API_VERSION).unwrap();
    for policy in [LoadPolicy::release(), LoadPolicy::development()] {
        let error = admit(package.clone(), policy, &NoSignatureRegistry).unwrap_err();
        assert_eq!(error.code(), "invalid-plugin-signature");
    }
}

#[test]
fn verified_signature_is_admitted_in_release() {
    let package = fixture();
    let gate = PinnedRegistry(package.digest().to_string());
    let admitted = admit(package, LoadPolicy::release(), &gate).unwrap();
    assert_eq!(
        admitted.trust(),
        &AdmissionTrust::Signed {
            registry: "test".to_owned()
        }
    );
    let wrong = PinnedRegistry("sha256:00".to_owned());
    assert_eq!(
        admit(fixture(), LoadPolicy::release(), &wrong)
            .unwrap_err()
            .code(),
        "invalid-plugin-signature"
    );
}
