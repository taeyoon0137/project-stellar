//! WebView inventory (XXI.2.4): native app UI와 canvas rendering 경로는 WebView를 쓰지 않습니다.
//!
//! 저장소의 앱·crate·도구 소스와 manifest에서 WebView 사용 표지를 찾습니다. 허용 위치는 플러그인 panel host(앞으로
//! `crates/plugin-host`와 앱의 plugin panel 파일) 뿐입니다. 지금은 plugin panel이 없으므로 허용 위치 밖 0이어야 합니다.

use std::path::{Path, PathBuf};

/// WebView 사용 표지.
const MARKERS: [&str; 6] = [
    "WKWebView",
    "import WebKit",
    "WebKit.framework",
    "WebView2",
    "wry",
    "webview",
];

/// 검사할 확장자.
const EXTENSIONS: [&str; 8] = ["swift", "m", "mm", "h", "rs", "toml", "pbxproj", "plist"];

/// 허용 위치(plugin panel host). 앱 plugin panel 파일은 WebView 연결(XXI.2.1) 때 여기에 더합니다.
const ALLOWED: [&str; 1] = ["crates/plugin-host/"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "build" || name == "DerivedData" {
            continue;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => walk(&path, out),
            Ok(kind) if kind.is_file() => {
                if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| EXTENSIONS.contains(&e))
                {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

fn marker_in(text: &str) -> Option<&'static str> {
    MARKERS.into_iter().find(|marker| {
        text.match_indices(marker).any(|(at, _)| {
            // 소문자 표지(`wry`, `webview`)는 식별자 경계에서만 셉니다(`-webkit-` 같은 CSS 값 제외).
            let before = text[..at].chars().next_back();
            let after = text[at + marker.len()..].chars().next();
            let boundary =
                |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_');
            marker.starts_with(char::is_uppercase)
                || marker.contains(' ')
                || (boundary(before) && boundary(after))
        })
    })
}

#[test]
fn no_webview_outside_plugin_panel() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for top in ["apps", "crates", "tools"] {
        walk(&root.join(top), &mut files);
    }
    files.push(root.join("Cargo.toml"));
    assert!(files.len() > 100, "검사 대상이 너무 적음: {}", files.len());
    let mut found = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.iter().any(|a| rel.starts_with(a)) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if let Some(marker) = marker_in(&text) {
            found.push(format!("{rel}: {marker}"));
        }
    }
    assert!(found.is_empty(), "plugin panel 밖 WebView 사용: {found:?}");
}

#[test]
fn marker_detection() {
    assert_eq!(marker_in("let v = WKWebView()"), Some("WKWebView"));
    assert_eq!(marker_in("import WebKit\n"), Some("import WebKit"));
    assert_eq!(marker_in("wry = \"0.40\""), Some("wry"));
    assert_eq!(marker_in("display: -webkit-box; awry"), None);
}
