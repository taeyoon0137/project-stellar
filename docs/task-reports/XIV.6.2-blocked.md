# 차단 보고서: XIV.6.2 실제 index 적용의 경합 보존

## WBS

- WBS: XIV.6.2
- 작업명: file mode와 blob OID를 공식 update-index 명령으로 적용
- 상태: `[!]` — 적용 후보의 안전성 검토 단계이며 실제 stage API를 완료하지 않았습니다.
- 선행: XIV.6.1 (`[x]`, `ba98f47c`)

## 실패 선행조건

- 사용자는 중간 실패·경합에서 타인의 index 변경을 덮어쓰지 않을 것을 요구했습니다. 기술 아키텍처 §24.2는 `.git` 내부 직접 수정을 금지합니다. 기존 runner는 완성된 stdin을 한 번에 보내므로 Git이 잠근 뒤 기준 index를 비교하고 그 결과에 따라 입력을 승인하는 경계가 없습니다.
- `read_index_snapshot` 후 `update-index --index-info`를 호출하는 단순 구현은 그 사이 변경된 같은 경로를 덮을 수 있습니다. caller 사전 비교와 Git 잠금 취득은 서로 다른 시점입니다.
- 실제 fixture에서도 `read-tree -i -m base candidate`는 외부 파일→후보 하위 파일 경합에서 외부 staged 파일을 잃고, 반대 방향은 성공 반환에도 후보를 적용하지 않았으며, unrelated REUC를 지웠습니다. 일반 경로의 carry-forward만으로 전체 보존을 주장할 수 없습니다. Git 소스는 index lock 후 읽기를 수행하지만 resolve-undo를 지웁니다. index flags·ITA·split/sparse index와 rollback 경합도 별도 증명이 필요합니다.
- 이 단계에서 `.git/index.lock`을 직접 쓰거나 rename하는 방식, 별도 실행기, 강제 옵션으로 경계를 우회하지 않았습니다.

## 재현 명령

- 공식 소스 검토: [update-index의 lock → index read](https://github.com/git/git/blob/v2.54.0/builtin/update-index.c#L1047-L1055), [read-tree의 lock과 resolve-undo clear](https://github.com/git/git/blob/v2.54.0/builtin/read-tree.c#L181-L196).
- `git apply --cached`도 검토했습니다. [binary patch old OID 검사](https://github.com/git/git/blob/v2.54.0/apply.c#L3097-L3125)는 있으나, 모든 변경을 binary patch로 만들고 index flags를 보존하는 단순한 공식 경로를 이번 범위에서 검증하지 못했습니다. 불가능하다는 결론은 아닙니다.


임시 probe를 기존 crate test 모듈에 등록해 실행했습니다. 실행 후 임시 모듈 등록과 `.rs` 파일은 제거했으며 아래 재현 코드만 보고서에 보존합니다. 모든 Git 변경은 기존 `GitFixture`의 임시 프로젝트 안에서만 했습니다.

| 명령/상황 | 결과 |
|---|---|
| `cargo test -p stellar-git-porcelain index_publish_probe:: -- --nocapture` (최초) | 101 — 2 passed, 1 failed. D/F 반대 방향의 성공이 후보 적용을 뜻한다는 probe 기대가 실제 결과와 달랐음 |
| 같은 명령 (관찰 결과 분기 수정 후) | 0 — 3개 재현 테스트 통과. 이는 아래 위험 확인이며 XIV.6.2 완료 검증이 아님 |
| 사전 snapshot → 외부 same-path 갱신 → candidate index-info | 외부 staged OID를 candidate OID로 덮음 |
| base는 anchor만, 후보 d/a 추가, 외부 d 추가 → read-tree -i -m | exit 0, 외부 d 소실, 후보 d/a로 교체 |
| base는 anchor만, 후보 d 추가, 외부 d/a 추가 → read-tree -i -m | exit 0, 외부 d/a 유지, 후보 d 미적용 |
| unrelated resolved.txt의 REUC 3개 + selected 수정 → read-tree -i -m | exit 0, selected 반영, unrelated REUC 3개가 0개로 감소 |
| `cargo xtask validate-task-report docs/task-reports/XIV.6.2-blocked.md` | 0 |
| `cargo fmt --all -- --check` / `git diff --check` / `git diff --exit-code -- crates` | 모두 0 — probe 제거 후 production 변경 없음 |
| `cargo xtask validate-task-status` / `cargo xtask sync-plan` / `cargo xtask verify-task XIV.5.6` / `cargo xtask verify-task XIV.6.1` | 모두 0 — 완료된 선행만 검증 |

실측 Git은 `2.54.0 (Apple Git-157)`, macOS 27.2입니다. 아래 재현은 그 동작을 기록하며 모든 Git 버전에 일반화하지 않습니다. 경합 순서 재현과 대안 위험 확인은 수행했지만, 안전한 publish/rollback 구현의 검증은 아닙니다.

재현하려면 아래 코드를 임시 `crates/git-porcelain/src/index_publish_probe.rs`에 두고 `lib.rs`에 `#[cfg(all(test, unix))] mod index_publish_probe;`를 임시 등록한 뒤 위 Cargo 명령을 실행합니다. 완료 후 두 임시 변경을 제거합니다.

```rust
//! XIV.6.2 임시 실측: 공식 Git 명령의 경합·REUC·directory/file 동작을 관찰합니다.

use crate::blob_write::write_blob;
use crate::fixture::GitFixture;
use crate::object::ObjectId;
use crate::runner::{GitOutput, argv};
use crate::snapshot::{IndexSnapshot, read_index_snapshot};

fn fixture(tag: &str) -> GitFixture {
    let fx = GitFixture::with_system_git(tag)
        .unwrap()
        .expect("system Git required for index publish probe");
    println!(
        "{tag}: {}",
        String::from_utf8_lossy(&fx.git(["--version"]).unwrap().stdout).trim()
    );
    fx
}

fn index_input(fx: &GitFixture, input: &str) {
    let output = fx
        .gated()
        .run_with_input(
            &argv(["update-index", "--index-info"]),
            fx.work(),
            input.as_bytes(),
        )
        .unwrap();
    assert!(output.success(), "index-info: {:?}", output);
}

fn set_entry(fx: &GitFixture, path: &str, oid: &ObjectId) {
    index_input(fx, &format!("100644 {oid}\t{path}\n"));
}

fn tree_oid(fx: &GitFixture) -> ObjectId {
    let output = fx.git(["write-tree"]).unwrap();
    ObjectId::from_hex_bytes(output.stdout.strip_suffix(b"\n").unwrap()).unwrap()
}

fn snapshot(fx: &GitFixture) -> IndexSnapshot {
    read_index_snapshot(&fx.gated(), fx.work()).unwrap()
}

fn report(label: &str, snapshot: &IndexSnapshot) {
    let entries: Vec<_> = snapshot
        .entries
        .iter()
        .map(|entry| {
            format!(
                "{} {} {}",
                entry.mode,
                entry.oid,
                entry.path.text().display()
            )
        })
        .collect();
    println!("{label}: {entries:?}; unmerged={:?}", snapshot.unmerged);
}

fn read_tree_merge(fx: &GitFixture, base: &ObjectId, target: &ObjectId) -> GitOutput {
    let output = fx
        .gated()
        .run(
            &argv(["read-tree", "-i", "-m", base.as_str(), target.as_str()]),
            fx.work(),
        )
        .unwrap();
    println!(
        "read-tree -i -m {base} {target}: exit={:?}, stdout={:?}, stderr={:?}",
        output.exit_code,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn naive_index_info_overwrites_same_path_changed_after_base_read() {
    let fx = fixture("index-publish-naive-race");
    fx.write("selected", b"base\n").unwrap();
    fx.git(["add", "--", "selected"]).unwrap();
    let base = snapshot(&fx);
    let target = write_blob(&fx.gated(), fx.work(), b"candidate\n").unwrap();
    let external = write_blob(&fx.gated(), fx.work(), b"external writer\n").unwrap();

    // base 확인이 끝난 뒤 별도 writer가 같은 경로를 갱신한 순서를 재현합니다.
    set_entry(&fx, "selected", &external);
    let externally_changed = snapshot(&fx);
    assert_eq!(externally_changed.entries[0].oid, external);
    assert_ne!(base, externally_changed);
    set_entry(&fx, "selected", &target);
    let after = snapshot(&fx);
    assert_eq!(after.entries.len(), 1);
    assert_eq!(after.entries[0].oid, target);
    assert_ne!(after.entries[0].oid, external);
    assert_eq!(
        std::fs::read(fx.work().join("selected")).unwrap(),
        b"base\n"
    );
    report("naive/base", &base);
    report("naive/external", &externally_changed);
    report("naive/after", &after);
    println!("naive/outcome: external same-path update overwritten");
}

#[test]
fn read_tree_merge_observes_unrelated_resolve_undo_extension() {
    let fx = fixture("index-publish-reuc");
    fx.write("selected", b"base selected\n").unwrap();
    fx.write("resolved.txt", b"resolved bytes\n").unwrap();
    fx.git(["add", "-A"]).unwrap();
    let base = tree_oid(&fx);
    let resolved = snapshot(&fx)
        .entries
        .into_iter()
        .find(|entry| entry.path.as_bytes() == b"resolved.txt")
        .unwrap()
        .oid;
    let target_blob = write_blob(&fx.gated(), fx.work(), b"candidate selected\n").unwrap();
    set_entry(&fx, "selected", &target_blob);
    let expected_target = snapshot(&fx);
    let target = tree_oid(&fx);
    fx.git(["read-tree", base.as_str()]).unwrap();

    let conflict_base = write_blob(&fx.gated(), fx.work(), b"conflict base\n").unwrap();
    let ours = write_blob(&fx.gated(), fx.work(), b"ours\n").unwrap();
    let theirs = write_blob(&fx.gated(), fx.work(), b"theirs\n").unwrap();
    index_input(
        &fx,
        &format!(
            "0 {}\tresolved.txt\n100644 {conflict_base} 1\tresolved.txt\n100644 {ours} 2\tresolved.txt\n100644 {theirs} 3\tresolved.txt\n",
            "0".repeat(conflict_base.as_str().len())
        ),
    );
    assert_eq!(snapshot(&fx).unmerged.len(), 3);
    set_entry(&fx, "resolved.txt", &resolved);
    let before = snapshot(&fx);
    assert!(!before.has_conflicts());
    let reuc_before = fx.git(["ls-files", "--resolve-undo", "-z"]).unwrap().stdout;
    assert!(!reuc_before.is_empty(), "stage 0 resolve must create REUC");

    let output = read_tree_merge(&fx, &base, &target);
    let after = snapshot(&fx);
    let reuc_after = fx.git(["ls-files", "--resolve-undo", "-z"]).unwrap().stdout;
    report("reuc/before", &before);
    report("reuc/after", &after);
    println!(
        "reuc/records: before={:?}, after={:?}",
        String::from_utf8_lossy(&reuc_before),
        String::from_utf8_lossy(&reuc_after)
    );
    if output.success() {
        assert_eq!(after, expected_target);
        assert!(reuc_after.is_empty() || reuc_after == reuc_before);
        println!(
            "reuc/outcome: merge accepted, unrelated REUC {}",
            if reuc_after == reuc_before {
                "preserved"
            } else {
                "cleared"
            }
        );
    } else {
        assert_eq!(after, before);
        assert_eq!(reuc_after, reuc_before);
        println!("reuc/outcome: merge rejected, index and REUC unchanged");
    }
}

#[test]
fn read_tree_merge_observes_external_directory_file_collisions_in_both_directions() {
    for (tag, target_path, external_path) in [
        ("index-publish-target-child", "d/a", "d"),
        ("index-publish-target-parent", "d", "d/a"),
    ] {
        let fx = fixture(tag);
        fx.write("anchor", b"anchor\n").unwrap();
        fx.git(["add", "--", "anchor"]).unwrap();
        let base = tree_oid(&fx);
        let target_blob = write_blob(&fx.gated(), fx.work(), b"candidate path\n").unwrap();
        set_entry(&fx, target_path, &target_blob);
        let expected_target = snapshot(&fx);
        let target = tree_oid(&fx);
        fx.git(["read-tree", base.as_str()]).unwrap();

        let external_blob = write_blob(&fx.gated(), fx.work(), b"external path\n").unwrap();
        set_entry(&fx, external_path, &external_blob);
        let before = snapshot(&fx);
        assert_eq!(before.entries.len(), 2);
        assert!(
            before
                .entries
                .iter()
                .any(|entry| entry.path.as_bytes() == external_path.as_bytes()
                    && entry.oid == external_blob)
        );
        let output = read_tree_merge(&fx, &base, &target);
        let after = snapshot(&fx);
        report(&format!("{tag}/external"), &before);
        report(&format!("{tag}/after"), &after);
        if output.success() && after == expected_target {
            assert_eq!(after, expected_target);
            assert!(
                !after
                    .entries
                    .iter()
                    .any(|entry| entry.path.as_bytes() == external_path.as_bytes())
            );
            println!(
                "{tag}/outcome: merge accepted, external {external_path:?} lost to candidate {target_path:?}"
            );
        } else if output.success() {
            assert_eq!(after, before);
            println!("{tag}/outcome: success returned, external preserved but candidate not applied");
        } else {
            assert_eq!(after, before);
            println!("{tag}/outcome: merge rejected, external {external_path:?} preserved");
        }
        assert_eq!(
            std::fs::read(fx.work().join("anchor")).unwrap(),
            b"anchor\n"
        );
    }
}
```

## 필요한 결정과 환경

- 기존 typed runner 안에서 Git 잠금과 기준 snapshot 검사를 하나의 안전한 적용 프로토콜로 묶는 설계·검증이 필요합니다. 지원하지 않는 index 상태는 데이터 손실 없이 진단해야 합니다.
- 이 보고서는 `.git` 직접 쓰기 허가를 요청하거나 기존 경계를 변경하지 않습니다. 새 실행기·강제 동작·main checkout·다른 worktree 변경·push·계정 설정 변경을 하지 않았습니다.
- `DEV-MAC` 임시 Git fixture만 검증 범위입니다. 실제 앱 QA·REF·Windows·플랫폼 gate는 미검증입니다.

## 변경되지 않은 파일

- 실제 index 적용 production API, runtime stage 명령, FFI·UI 연결은 추가하지 않았습니다.
- XIV.6.2~XIV.6.6 원장·완료 표기는 변경하지 않았습니다. XIV.6.3~XIV.6.6은 순서를 건너뛰어 착수하지 않았습니다.
- 성공한 선행 코드와 원장은 XIV.5.6 `68be98af`, XIV.6.1 `ba98f47c` 및 후속 문서 커밋에 있습니다.

## 재개 조건

- 일반 저장 프로젝트에서 속성 A만 실제 stage하고 B를 unstaged로 유지하며, 별도 staged 경로와 원본 bytes·메모리 revision을 보존하는 end-to-end 검증.
- 잠금 직전/도중 외부 same-path·unrelated·D/F 변경 및 index metadata를 포함한 원자적 경합 검사. 실패·취소·timeout·semantic mismatch 때 타인의 변경을 덮지 않는 복원 또는 명시적 복원 충돌 결과.
- symlink/executable/삭제, 실제 명령 횟수·병렬 결과, 적용 후 Index provider의 semantic 재검증. 단계별 완료 뒤에만 원장 갱신.
