use super::*;
use crate::window_session::test_fixtures::{identity, rename, saved_project, sid, window, TempDir};

#[test]
fn checkpoint_rejects_partial_unknown_or_duplicate_chunks_without_writes() {
    let dir = saved_project("recovery-payload");
    let app = TempDir::new("recovery-store");
    let store = RecoverySnapshotStore::new(app.path());
    let id = identity(dir.path());
    let owner = window(41);
    let mut w = ProjectWindowSession::load(id.clone(), owner).unwrap();
    let edit = rename(w.document().unwrap(), 305, 11, "unsaved Frame");
    w.apply(&edit, 0).unwrap();
    Capture::from_window(&w)
        .unwrap()
        .write(&store, &id, owner)
        .unwrap();
    let p = store.read_snapshot(&id, owner).unwrap().unwrap();
    let snapshot = RecoveryPayload::decode(&p).unwrap();
    let expected = w.document().unwrap().model().to_records().unwrap();
    assert_eq!(
        load_model(&snapshot).unwrap().to_records().unwrap(),
        expected
    );
    let mut truncated = snapshot.clone();
    truncated.unsaved_chunks.pop();
    assert!(load_model(&truncated).is_err());
    let mut duplicate = snapshot.clone();
    duplicate
        .unsaved_chunks
        .push(snapshot.unsaved_chunks[0].clone());
    assert!(load_model(&duplicate).is_err());
    let mut unknown = snapshot.clone();
    unknown.unsaved_chunks[1].kind = stellar_storage::chunk::envelope::ChunkKindCode::optional(57);
    assert!(load_model(&unknown).is_err());
    assert_eq!(store.read_snapshot(&id, owner).unwrap().unwrap(), p);
    let unchanged = ProjectWindowSession::load(id, window(42)).unwrap();
    assert_ne!(
        unchanged
            .document()
            .unwrap()
            .model()
            .record(sid(11))
            .unwrap()
            .display_name,
        "unsaved Frame"
    );
}
