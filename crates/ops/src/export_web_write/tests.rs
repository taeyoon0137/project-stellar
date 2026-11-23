//! XXII.5.2 검증: user file preservation fixture.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use stellar_codegen::output::GeneratedFiles;

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-web-write-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp");
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn files(pairs: &[(&str, &str)]) -> GeneratedFiles {
    let mut out = GeneratedFiles::new();
    for (path, content) in pairs {
        out.insert(path, content).expect("insert");
    }
    out
}

fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    list_files(dir)
        .expect("list")
        .into_iter()
        .map(|p| {
            let bytes = fs::read(dir.join(&p)).expect("read");
            (p, bytes)
        })
        .collect()
}

#[test]
fn fresh_and_regenerated_directories_are_replaced_with_manifest() {
    let temp = TempDir::new("fresh");
    let out = temp.0.join("web");
    let first = files(&[
        ("index.html", "<p>1</p>"),
        ("styles.css", "a {}"),
        ("js/state.js", "x"),
    ]);
    let report = replace_directory(&out, &first).expect("first");
    assert_eq!(report.written, ["index.html", "js/state.js", "styles.css"]);
    let manifest = fs::read_to_string(out.join(MANIFEST)).expect("manifest");
    assert!(manifest.ends_with("index.html\njs/state.js\nstyles.css\n"));
    // 다시 생성: 없어진 파일은 지우고 내용은 바꿉니다.
    let second = files(&[("index.html", "<p>2</p>")]);
    let report = replace_directory(&out, &second).expect("second");
    assert_eq!(report.removed, ["js/state.js", "styles.css"]);
    assert_eq!(list_files(&out).expect("list"), [MANIFEST, "index.html"]);
    assert!(fs::read_to_string(out.join("index.html"))
        .unwrap()
        .contains("<p>2</p>"));
    // 임시·백업 디렉터리가 남지 않습니다.
    let leftovers: Vec<_> = fs::read_dir(&temp.0)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("stellar-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn user_files_are_never_overwritten() {
    // 검증 기준(XXII.5.2): user file preservation fixture.
    let temp = TempDir::new("user");
    let out = temp.0.join("web");
    replace_directory(&out, &files(&[("index.html", "<p>1</p>")])).expect("first");
    // 생성 디렉터리 안에 사용자가 파일을 더하면 다음 생성은 거부되고 아무것도 바뀌지 않습니다.
    fs::write(out.join("notes.md"), "사용자 메모").expect("user");
    fs::write(out.join("index.html"), "사용자가 고친 HTML").expect("edit");
    let before = snapshot(&out);
    let error = replace_directory(&out, &files(&[("index.html", "<p>2</p>")])).unwrap_err();
    assert!(matches!(&error, WriteError::UserFiles(list) if list == &["notes.md".to_string()]));
    assert_eq!(snapshot(&out), before);
    // manifest 없는 사용자 디렉터리는 비어 있지 않으면 쓰지 않습니다.
    let mine = temp.0.join("mine");
    fs::create_dir_all(mine.join("src")).unwrap();
    fs::write(mine.join("src/app.js"), "console.log(1)").unwrap();
    let before = snapshot(&mine);
    let error = replace_directory(&mine, &files(&[("index.html", "<p>x</p>")])).unwrap_err();
    assert!(matches!(&error, WriteError::UserFiles(list) if list == &["src/app.js".to_string()]));
    assert_eq!(snapshot(&mine), before);
    // 파일 경로는 디렉터리가 아니므로 거부합니다.
    let error = replace_directory(&mine.join("src/app.js"), &files(&[("a.css", "x")])).unwrap_err();
    assert!(matches!(error, WriteError::NotADirectory(_)));
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let temp = TempDir::new("link");
    let out = temp.0.join("web");
    replace_directory(&out, &files(&[("index.html", "<p>1</p>")])).expect("first");
    let target = temp.0.join("elsewhere.txt");
    fs::write(&target, "밖의 파일").unwrap();
    std::os::unix::fs::symlink(&target, out.join("link.txt")).unwrap();
    let error = replace_directory(&out, &files(&[("index.html", "<p>2</p>")])).unwrap_err();
    assert!(matches!(error, WriteError::Symlink(_)));
    assert_eq!(fs::read_to_string(&target).unwrap(), "밖의 파일");
    let linked_dir = temp.0.join("linked");
    std::os::unix::fs::symlink(&out, &linked_dir).unwrap();
    assert!(matches!(
        replace_directory(&linked_dir, &files(&[("a.css", "x")])).unwrap_err(),
        WriteError::Symlink(_)
    ));
}

#[test]
fn only_marked_regions_change_in_user_files() {
    // 검증 기준(XXII.5.2): generated marker 아래만 수정하고 나머지 byte는 그대로.
    let temp = TempDir::new("marker");
    let file = temp.0.join("app.css");
    let user = "/* 사용자 규칙 */\nbody { margin: 0; }\r\n/* stellar:generated:begin tokens */\n:root { --old: 1px; }\n/* stellar:generated:end tokens */\n.mine { color: red; }  \n/* stellar:generated:begin motion */\nold\n/* stellar:generated:end motion */\n끝";
    fs::write(&file, user).unwrap();
    let mut regions = BTreeMap::new();
    regions.insert("tokens".to_string(), ":root { --new: 2px; }".to_string());
    update_marked_regions(&file, &regions).expect("update");
    let after = fs::read_to_string(&file).unwrap();
    assert_eq!(
        after,
        "/* 사용자 규칙 */\nbody { margin: 0; }\r\n/* stellar:generated:begin tokens */\n:root { --new: 2px; }\n/* stellar:generated:end tokens */\n.mine { color: red; }  \n/* stellar:generated:begin motion */\nold\n/* stellar:generated:end motion */\n끝"
    );
    // 오류 case: 없는 구간, 짝 없는 marker, 겹친 marker, 같은 이름 두 번. 파일은 그대로입니다.
    let cases = [
        ("missing", "a\n", "x", "missing"),
        (
            "unclosed",
            "/* stellar:generated:begin a */\nx\n",
            "a",
            "mismatch",
        ),
        (
            "nested",
            "/* stellar:generated:begin a */\n/* stellar:generated:begin b */\n/* stellar:generated:end b */\n/* stellar:generated:end a */\n",
            "a",
            "mismatch",
        ),
        (
            "twice",
            "/* stellar:generated:begin a */\n/* stellar:generated:end a */\n/* stellar:generated:begin a */\n/* stellar:generated:end a */\n",
            "a",
            "mismatch",
        ),
    ];
    for (tag, content, name, expected) in cases {
        let path = temp.0.join(format!("{tag}.css"));
        fs::write(&path, content).unwrap();
        let mut regions = BTreeMap::new();
        regions.insert(name.to_string(), "new".to_string());
        let error = update_marked_regions(&path, &regions).unwrap_err();
        match expected {
            "missing" => assert!(matches!(error, WriteError::MarkerMissing(_)), "{tag}"),
            _ => assert!(matches!(error, WriteError::MarkerMismatch(_)), "{tag}"),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), content, "{tag}");
    }
}
