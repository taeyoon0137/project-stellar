#![cfg(test)]
//! history journal 저장 테스트 공용 fixture(XXV.1.2~): 임시 디렉터리와 hermetic 시스템 Git 실행기.
//!
//! Git 실행기는 사용자·시스템 설정(전역 exclude 등)이 결과에 섞이지 않도록 환경을 비우고 실행합니다.
//! 시스템 Git이 없으면 `None`이며, 그 경우 테스트는 Git 확인 부분만 건너뜁니다.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// drop 시 지워지는 임시 디렉터리(canonical 경로).
pub struct TempDir(PathBuf);

impl TempDir {
    /// 새 빈 디렉터리.
    pub fn new(prefix: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-history-{prefix}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("임시 디렉터리");
        TempDir(std::fs::canonicalize(&path).expect("canonical 임시 경로"))
    }

    /// 경로.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// hermetic 시스템 Git.
pub struct FixtureGit {
    git: PathBuf,
    home: PathBuf,
}

impl FixtureGit {
    /// PATH에서 git을 찾습니다. `home`은 HOME으로 쓸 디렉터리입니다.
    pub fn find(home: &Path) -> Option<FixtureGit> {
        let path = std::env::var_os("PATH")?;
        let git = std::env::split_paths(&path)
            .map(|dir| dir.join("git"))
            .find(|candidate| candidate.is_file());
        if git.is_none() {
            eprintln!("시스템 git 없음 — Git status 확인을 건너뜁니다");
        }
        Some(FixtureGit {
            git: git?,
            home: home.to_path_buf(),
        })
    }

    /// 실행하고 (성공 여부, stdout)을 돌려줍니다.
    pub fn run(&self, cwd: &Path, args: &[&str]) -> (bool, String) {
        let output = Command::new(&self.git)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .output()
            .expect("git 실행");
        (
            output.status.success(),
            String::from_utf8(output.stdout).expect("utf8 stdout"),
        )
    }

    /// 성공해야 하는 실행. stdout을 돌려줍니다.
    pub fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let (success, stdout) = self.run(cwd, args);
        assert!(success, "fixture git 실패: {args:?}");
        stdout
    }

    /// 파일 하나를 커밋한 저장소를 만듭니다.
    pub fn init_repository(&self, root: &Path) {
        self.ok(root, &["init", "-q", "-b", "main"]);
        self.ok(root, &["config", "user.name", "Fixture"]);
        self.ok(root, &["config", "user.email", "fixture@example.com"]);
        std::fs::write(root.join("stellar.yaml"), "schema_version: 1\n").expect("manifest");
        self.ok(root, &["add", "stellar.yaml"]);
        self.ok(root, &["commit", "-q", "-m", "init"]);
    }

    /// untracked·ignored 포함 전체 status(`--ignored`는 선택).
    pub fn status(&self, root: &Path, ignored: bool) -> String {
        let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all"];
        if ignored {
            args.push("--ignored");
        }
        self.ok(root, &args)
    }
}

/// 디렉터리 아래 모든 파일·디렉터리(상대 경로, 정렬).
pub fn list_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            out.push(
                path.strip_prefix(root)
                    .expect("root 아래")
                    .display()
                    .to_string(),
            );
            if path.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}
