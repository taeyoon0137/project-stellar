//! 허용 목록 child 환경 (XXIX.2.4, ADR-0023 결정 9, ADR-0092 결정 5).
//!
//! 대화형 터미널(XVIII.2.5)과 달리 구독 CLI 프로세스는 빈 환경에서 시작하고, launch profile의 환경 변수
//! 이름 목록에 있는 이름만 앱 프로세스 환경에서 그대로 전달합니다. Stellar가 이름을 더하거나 값을 바꾸지
//! 않습니다. 결합 token(XIX.1.8)은 CLI 환경이 아니라 MCP 설정 파일의 서버 `env`로만 넘깁니다.

use std::ffi::OsString;
use std::fmt;

/// child 프로세스 환경(이름 순, 이름 중복 없음).
#[derive(Clone, PartialEq, Eq)]
pub struct ChildEnv {
    vars: Vec<(OsString, OsString)>,
}

impl fmt::Debug for ChildEnv {
    /// 값은 쓰지 않고 이름만 씁니다.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.vars.iter().map(|(k, _)| k))
            .finish()
    }
}

impl ChildEnv {
    /// 빈 환경에서 `names`에 있는 이름만 `source`(앱 프로세스 환경)에서 골라 만듭니다. `source`에 없는
    /// 이름은 건너뜁니다.
    pub fn allowlisted<I>(names: &[String], source: I) -> ChildEnv
    where
        I: IntoIterator<Item = (OsString, OsString)>,
    {
        let mut vars: Vec<(OsString, OsString)> = source
            .into_iter()
            .filter(|(key, _)| names.iter().any(|n| key.as_os_str() == n.as_str()))
            .collect();
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        vars.dedup_by(|a, b| a.0 == b.0);
        ChildEnv { vars }
    }

    /// 앱 프로세스 환경에서 만듭니다.
    pub fn from_app_env(names: &[String]) -> ChildEnv {
        ChildEnv::allowlisted(names, std::env::vars_os())
    }

    /// 변수(이름 순).
    pub fn vars(&self) -> &[(OsString, OsString)] {
        &self.vars
    }

    /// 이름(이름 순).
    pub fn names(&self) -> Vec<String> {
        self.vars
            .iter()
            .map(|(k, _)| k.to_string_lossy().into_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_names_pass_through_unchanged() {
        let names = vec!["HOME".to_string(), "PATH".to_string(), "LANG".to_string()];
        let source = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("HOME".into(), "/Users/x".into()),
            ("SECRET_TOKEN".into(), "v".into()),
            ("DYLD_INSERT_LIBRARIES".into(), "/tmp/x.dylib".into()),
        ];
        let env = ChildEnv::allowlisted(&names, source);
        assert_eq!(env.names(), vec!["HOME", "PATH"]);
        assert_eq!(env.vars()[1].1, OsString::from("/usr/bin"));
        assert!(!format!("{env:?}").contains("/Users/x"));
    }
}
