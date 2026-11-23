//! 텍스트 원본 기본 열기·저장 실측(V.10.7, ADR-0088 Performance). 기본 시험에서는 빼고(`#[ignore]`) release로
//! 실행합니다.
//!
//! ```text
//! cargo test -p stellar-runtime --release --lib window_session::source_bench -- --ignored --nocapture
//! ```

use super::test_fixtures::*;
use super::*;
use stellar_commands::document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_commands::document::frame::FramePayload;
use stellar_commands::document::records::{NodePayload, NodeRecord};
use stellar_core_types::order_key::OrderKey;
use stellar_project::external_change::source::TextSourceSession;

/// View 하나: root 아래 `groups`개 Frame, 각 아래 `per`개 Frame.
fn large_document(groups: u64, per: u64) -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), "큰 문서".into());
    let mut page = PageDefinition::new(sid(2), "페이지".into());
    page.add_view(ViewDefinition::new(sid(3), "화면".into(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut model = DocumentModel::new(definition);
    let frame = |id: u64, parent: Option<u64>, index: u64| NodeRecord {
        id: sid(id),
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![
            0x10 + (index / 200) as u8,
            0x10 + (index % 200) as u8,
        ])
        .unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{id}"),
        payload: NodePayload::Frame(FramePayload::default()),
    };
    model.insert_node(frame(10, None, 0)).unwrap();
    for g in 0..groups {
        let group = 1_000 + g;
        model.insert_node(frame(group, Some(10), g)).unwrap();
        for c in 0..per {
            model
                .insert_node(frame(100_000 + g * 1_000 + c, Some(group), c))
                .unwrap();
        }
    }
    model
}

fn ms(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = std::time::Instant::now();
    let value = f();
    (value, ms(started.elapsed()))
}

#[test]
#[ignore = "실측(release로 실행)"]
fn measure_text_source_open_and_save() {
    for (groups, per) in [(10, 100), (100, 100), (200, 250)] {
        let model = large_document(groups, per);
        let nodes = model.node_count();
        let project = saved_project_with("bench", &[&model]);
        let locks = TempDir::new("bench-locks");
        let dir = locks.path().to_path_buf();
        let lock = LockDir::new(move || Ok(dir.clone()));
        let open = || {
            ProjectWindowSession::load_with_lock_dir(
                identity(project.path()),
                window(1),
                lock.clone(),
            )
            .unwrap()
        };
        let (session, migrate) = time(open);
        assert_eq!(session.open_plan(), OpenPlan::Migrate);
        drop(session);
        let files = stellar_project::document_io::session::read_source(project.path())
            .unwrap()
            .unwrap()
            .files
            .len();
        let mut cache = Vec::new();
        for _ in 0..5 {
            let (session, elapsed) = time(open);
            assert_eq!(session.open_plan(), OpenPlan::Cache);
            cache.push(elapsed);
        }
        // 원본만 바뀐 뒤(Git pull) 열기: 원본 전체 해석 + cache 다시 만들기.
        let mut text = TextSourceSession::open(project.path(), window(9)).unwrap();
        let tx = rename(text.document(), 77, 1_000, "pull");
        text.apply(&tx).unwrap();
        text.save().unwrap();
        let (session, from_text) = time(open);
        assert_eq!(session.open_plan(), OpenPlan::FromText);
        // 이름 하나 바꾼 저장: 원본 인코딩 + 바뀐 파일 journal + cache generation + stamp.
        let mut session = session;
        let mut saves = Vec::new();
        let mut serialize = Vec::new();
        let mut write = Vec::new();
        for i in 0..5_u64 {
            let tx = rename(
                session.document().unwrap(),
                500 + i,
                1_000,
                &format!("s{i}"),
            );
            session.apply(&tx, 0).unwrap();
            let started = std::time::Instant::now();
            let capture = session.capture_save().unwrap().unwrap();
            let lock = lock.clone();
            let (written, timings) = capture.write_timed(move || lock.get());
            let outcome = session.apply_save(written.unwrap());
            saves.push(ms(started.elapsed()));
            serialize.push(ms(timings.serialize));
            write.push(ms(timings.write));
            assert_eq!(outcome.source_files, 1);
        }
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        println!(
            "nodes={nodes} files={files} migrate_open={migrate:.1}ms cache_open_p50={:.1}ms \
             from_text_open={from_text:.1}ms save_p50={:.1}ms (serialize+encode {:.1}ms, write {:.1}ms)",
            median(&mut cache),
            median(&mut saves),
            median(&mut serialize),
            median(&mut write),
        );
    }
}

#[test]
#[ignore = "실측(release로 실행)"]
fn measure_encode_parts() {
    for (groups, per) in [(10, 100), (100, 100), (200, 250)] {
        let model = large_document(groups, per);
        let records = model.to_records().unwrap();
        let (_, chunk) =
            time(|| stellar_project::document_io::serialize::serialize_records(&records).unwrap());
        let (copy, from_records) = time(|| DocumentModel::from_records(records.clone()).unwrap().0);
        let library = stellar_project::document_io::session::SourceLibrary::default();
        let project = library.join(copy);
        let (encoded, encode) =
            time(|| stellar_project::document_io::source_v4::project::encode(&project).unwrap());
        let (_, decode) = time(|| {
            stellar_project::document_io::source_v4::project::decode(&encoded.files).unwrap()
        });
        println!("nodes={} chunk={chunk:.1}ms from_records={from_records:.1}ms encode={encode:.1}ms decode={decode:.1}ms files={}", model.node_count(), encoded.files.len());
    }
}
