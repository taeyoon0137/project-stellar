//! 통합 test 공용 fixture: 임시 신뢰 프로젝트와 모든 권한을 선언한 개발 모드 플러그인.

#![allow(dead_code)]

pub mod process;

use std::collections::BTreeSet;
use std::path::PathBuf;

use stellar_ops::lock::LockPolicy;
use stellar_ops::test_support::{
    large_document, project_id, save_documents, scaffold_project, select_trusted_project,
    FileTrustStore, ProjectIdentity, TempDir,
};
use stellar_plugin_host::admission::{admit, AdmittedPlugin, LoadPolicy, NoSignatureRegistry};
use stellar_plugin_host::host::{Dispatcher, HostConfig, NetworkPort};
use stellar_plugin_host::json::{parse, JsonLimits, JsonValue};
use stellar_plugin_host::package::PluginPackage;
use stellar_plugin_host::permission::Permission;
use stellar_plugin_host::HOST_API_VERSION;

pub const PLUGIN_ID: &str = "com.example.all";

pub struct Fixture {
    pub temp: TempDir,
    pub project: PathBuf,
    pub data: PathBuf,
    pub locks: PathBuf,
    pub package: PathBuf,
}

/// 고정 응답 network port.
pub struct FakeNetwork;

impl NetworkPort for FakeNetwork {
    fn get(&self, url: &str) -> Result<(u16, String), String> {
        Ok((200, format!("fetched {}", url.len())))
    }
}

impl Fixture {
    /// 문서 하나(node `nodes`개)를 저장한 신뢰 프로젝트와 모든 권한 선언 package.
    pub fn new(label: &str, nodes: usize) -> Fixture {
        let temp = TempDir::new(label);
        let project = temp.path.join("project");
        let data = temp.path.join("data");
        let locks = temp.path.join("locks");
        scaffold_project(&project);
        save_documents(&project, &[&large_document(nodes)]);
        FileTrustStore::open(&data)
            .unwrap()
            .grant(&ProjectIdentity::new(project_id(), &project).unwrap())
            .unwrap();
        let package = temp.path.join("all.stellar-plugin");
        std::fs::create_dir_all(package.join("ui")).unwrap();
        let permissions: String = Permission::ALL
            .iter()
            .map(|p| format!("  - {}\n", p.name()))
            .collect();
        std::fs::write(
            package.join("plugin.yaml"),
            format!(
                "manifest_version: 1\nid: {PLUGIN_ID}\nversion: 1.0.0\nname: All\nentry: ui/index.html\nhost_api: 1\npermissions:\n{permissions}"
            ),
        )
        .unwrap();
        std::fs::write(package.join("ui/index.html"), "<!doctype html>").unwrap();
        Fixture {
            temp,
            project,
            data,
            locks,
            package,
        }
    }

    pub fn plugin(&self) -> AdmittedPlugin {
        let package = PluginPackage::open(&self.package, HOST_API_VERSION).unwrap();
        admit(package, LoadPolicy::development(), &NoSignatureRegistry).unwrap()
    }

    pub fn config(&self) -> HostConfig {
        HostConfig {
            lock: LockPolicy::Required {
                dir: self.locks.clone(),
            },
            network: Some(Box::new(FakeNetwork)),
        }
    }

    pub fn dispatcher(&self, granted: &[Permission]) -> Dispatcher {
        let granted: BTreeSet<Permission> = granted.iter().copied().collect();
        Dispatcher::new(
            select_trusted_project(&self.project, self.data.clone()).unwrap(),
            self.plugin(),
            &granted,
            self.config(),
        )
    }

    /// 실제 Git 저장소로 바꿉니다(scaffold의 빈 `.git` 대신).
    pub fn git_init(&self) -> bool {
        let _ = std::fs::remove_dir_all(self.project.join(".git"));
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&self.project)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

pub fn json(text: &str) -> JsonValue {
    parse(text, JsonLimits::default()).unwrap()
}
