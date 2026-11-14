use super::*;
use crate::{
    agent::Conversation,
    message::{Block, Message},
    session::{SessionKind, SessionMeta},
};
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt},
};

struct Fixture {
    dir: PathBuf,
    session: Session,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("mecha-native-writer-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let session = Session::create(
            &dir,
            SessionMeta {
                id: "authored-native-fixture".into(),
                created_at: chrono::Utc::now(),
                provider: "fixture".into(),
                model: "fixture".into(),
                workspace: dir.clone(),
                title: None,
                kind: Some(SessionKind::Test),
            },
        )
        .unwrap();
        Self { dir, session }
    }
    fn bind(&self) -> BoundNativeSession {
        let m = fs::metadata(&self.session.path).unwrap();
        BoundNativeSession::bind(
            &self.session,
            NativeIdentity {
                device: m.dev(),
                inode: m.ino(),
            },
            &fs::read(&self.session.path).unwrap(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
fn message() -> Record {
    Record::Message(Message::user("authored fixture"))
}
fn refused(writer: &mut BoundNativeSession) {
    assert!(writer.append(&message()).is_err());
    assert!(writer.sync_verified().is_err());
    assert!(writer.append(&message()).is_err());
}

#[test]
fn replacement_receives_no_append_even_when_its_bytes_match() {
    let f = Fixture::new();
    let mut w = f.bind();
    let bytes = fs::read(&f.session.path).unwrap();
    fs::rename(&f.session.path, f.dir.join("original")).unwrap();
    fs::write(&f.session.path, &bytes).unwrap();
    refused(&mut w);
    assert_eq!(fs::read(&f.session.path).unwrap(), bytes);
    assert_eq!(fs::read(f.dir.join("original")).unwrap(), bytes);
}
#[test]
fn deletion_is_never_recreated() {
    let f = Fixture::new();
    let mut w = f.bind();
    fs::remove_file(&f.session.path).unwrap();
    refused(&mut w);
    assert!(!f.session.path.exists());
}
#[test]
fn leaf_symlink_and_hardlink_refuse() {
    for hard in [false, true] {
        let f = Fixture::new();
        let mut w = f.bind();
        let original = f.dir.join("original");
        if hard {
            fs::hard_link(&f.session.path, &original).unwrap();
        } else {
            fs::rename(&f.session.path, &original).unwrap();
            symlink(&original, &f.session.path).unwrap();
        }
        let bytes = fs::read(&original).unwrap();
        refused(&mut w);
        assert_eq!(fs::read(original).unwrap(), bytes);
    }
}
#[test]
fn renamed_parent_and_ancestor_symlink_refuse() {
    let f = Fixture::new();
    let parent = f.dir.join("parent");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("native.jsonl");
    fs::rename(&f.session.path, &path).unwrap();
    let session = Session {
        meta: f.session.meta.clone(),
        path,
    };
    let m = fs::metadata(&session.path).unwrap();
    let mut w = BoundNativeSession::bind(
        &session,
        NativeIdentity {
            device: m.dev(),
            inode: m.ino(),
        },
        &fs::read(&session.path).unwrap(),
    )
    .unwrap();
    fs::rename(&parent, f.dir.join("moved")).unwrap();
    symlink(f.dir.join("moved"), &parent).unwrap();
    refused(&mut w);
}
#[test]
fn foreign_growth_and_truncation_refuse() {
    for truncate in [false, true] {
        let f = Fixture::new();
        let mut w = f.bind();
        if truncate {
            fs::OpenOptions::new()
                .write(true)
                .open(&f.session.path)
                .unwrap()
                .set_len(1)
                .unwrap();
        } else {
            f.session.append(&message()).unwrap();
        }
        let bytes = fs::read(&f.session.path).unwrap();
        refused(&mut w);
        assert_eq!(fs::read(&f.session.path).unwrap(), bytes);
    }
}
#[test]
fn equal_length_mutation_refuses_a_proof_and_poisons_writer() {
    let f = Fixture::new();
    let mut w = f.bind();
    w.append(&message()).unwrap();
    let mut bytes = fs::read(&f.session.path).unwrap();
    let pos = bytes
        .windows(b"authored fixture".len())
        .position(|b| b == b"authored fixture")
        .unwrap();
    bytes[pos] = b'A';
    fs::write(&f.session.path, &bytes).unwrap();
    assert!(w.sync_verified().is_err());
    refused(&mut w);
}
#[test]
fn bind_requires_exact_identity_prefix_and_strict_records() {
    let f = Fixture::new();
    let m = fs::metadata(&f.session.path).unwrap();
    let identity = NativeIdentity {
        device: m.dev(),
        inode: m.ino(),
    };
    let bytes = fs::read(&f.session.path).unwrap();
    assert!(BoundNativeSession::bind(
        &f.session,
        NativeIdentity {
            inode: identity.inode + 1,
            ..identity
        },
        &bytes
    )
    .is_err());
    assert!(BoundNativeSession::bind(&f.session, identity, &bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad.extend_from_slice(
        b"{\"record\":\"taint\",\"private\":false,\"private\":true,\"untrusted\":false}\n",
    );
    fs::write(&f.session.path, &bad).unwrap();
    assert!(BoundNativeSession::bind(&f.session, identity, &bad).is_err());
}
#[test]
fn append_and_run_match_legacy_bytes_and_sync_exact_prefix() {
    let f = Fixture::new();
    let mut w = f.bind();
    let legacy = Session {
        meta: f.session.meta.clone(),
        path: f.dir.join("legacy.jsonl"),
    };
    fs::copy(&f.session.path, &legacy.path).unwrap();
    let before = vec![Message::user("authored fixture")];
    w.append(&message()).unwrap();
    legacy.append(&message()).unwrap();
    assert!(fs::read(&f.session.path).unwrap().ends_with(b"{\"record\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"authored fixture\"}]}\n"));
    let mut convo = Conversation::from(before.clone());
    convo.push(Message::assistant(vec![Block::text("fixture answer")]));
    convo.harness_notes = vec!["fixture note".into()];
    w.record_run(&before, &convo).unwrap();
    legacy.record_run(&before, &convo).unwrap();
    let proof = w.sync_verified().unwrap();
    let bytes = fs::read(&legacy.path).unwrap();
    assert_eq!(proof.bytes(), bytes);
    assert_eq!(proof.endpoint(), bytes.len() as u64);
    assert_eq!(proof.digest(), super::super::authority::digest(&bytes));
    assert_eq!(
        proof.identity(),
        NativeIdentity {
            device: fs::metadata(&f.session.path).unwrap().dev(),
            inode: fs::metadata(&f.session.path).unwrap().ino()
        }
    );
    assert!(proof.bytes().ends_with(b"{\"record\":\"notes\",\"notes\":[\"fixture note\"]}\n{\"record\":\"grants\",\"grants\":[]}\n{\"record\":\"goal_anchor\",\"goal\":null}\n"));
    let records = super::super::recording::records(proof.bytes()).unwrap();
    assert!(matches!(
        &records[records.len() - 3..],
        [
            Record::Notes { .. },
            Record::Grants { .. },
            Record::GoalAnchor { .. }
        ]
    ));
}
#[test]
fn exact_empty_retry_extends_current_user_tail_and_matches_legacy() {
    let f = Fixture::new();
    f.session.append(&message()).unwrap();
    let mut w = f.bind();
    let legacy = Session {
        meta: f.session.meta.clone(),
        path: f.dir.join("legacy.jsonl"),
    };
    fs::copy(&f.session.path, &legacy.path).unwrap();
    let before = vec![Message::user("authored fixture")];
    let mut convo = Conversation::from(before.clone());
    convo.messages[0]
        .content
        .push(Block::text(crate::agent::EMPTY_TURN_NUDGE));
    convo.push(Message::assistant(vec![Block::text("fixture answer")]));
    w.record_run(&before, &convo).unwrap();
    legacy.record_run(&before, &convo).unwrap();
    let proof = w.sync_verified().unwrap();
    assert_eq!(proof.bytes(), fs::read(&legacy.path).unwrap());
    assert!(super::super::recording::records(proof.bytes())
        .unwrap()
        .iter()
        .any(|r| matches!(r, Record::Extend { index: 0, .. })));
}
#[test]
fn rewrite_unsupported_extend_and_wrong_before_write_nothing() {
    for mode in 0..4 {
        let f = Fixture::new();
        f.session.append(&message()).unwrap();
        let mut w = f.bind();
        let initial = fs::read(&f.session.path).unwrap();
        let mut before = vec![Message::user("authored fixture")];
        let mut convo = Conversation::from(before.clone());
        match mode {
            0 => convo.messages[0] = Message::user("changed head"),
            1 => convo.messages[0]
                .content
                .push(Block::text("unsupported addition")),
            2 => {
                convo.rewritten = vec![before.clone()];
            }
            _ => {
                before[0] = Message::user("wrong before");
                convo = Conversation::from(before.clone());
                convo.push(Message::assistant(vec![Block::text("answer")]));
            }
        }
        assert!(w.record_run(&before, &convo).is_err());
        assert_eq!(fs::read(&f.session.path).unwrap(), initial);
    }
}
#[test]
fn partial_write_poisons_even_when_original_bytes_are_restored() {
    let f = Fixture::new();
    let mut w = f.bind();
    let original = fs::read(&f.session.path).unwrap();
    test_io::fail_write_after(7);
    assert!(w.append(&message()).is_err());
    assert_eq!(
        fs::metadata(&f.session.path).unwrap().len(),
        original.len() as u64 + 7
    );
    fs::write(&f.session.path, &original).unwrap();
    refused(&mut w);
}
#[test]
fn file_or_parent_sync_failure_never_yields_a_proof() {
    for parent in [false, true] {
        let f = Fixture::new();
        let mut w = f.bind();
        w.append(&message()).unwrap();
        test_io::fail_sync(parent);
        assert!(w.sync_verified().is_err());
        refused(&mut w);
    }
}
#[test]
fn source_bound_is_checked_before_appending_any_bytes() {
    let f = Fixture::new();
    let mut w = f.bind();
    let bytes = fs::read(&f.session.path).unwrap();
    let record = Record::Message(Message::user(
        "x".repeat(super::super::authority::SOURCE_MAX),
    ));
    assert!(w.append(&record).is_err());
    assert_eq!(fs::read(&f.session.path).unwrap(), bytes);
}

#[test]
fn preparation_predicts_exact_final_bytes_and_updates_for_host_tail() {
    let f = Fixture::new();
    f.session.append(&message()).unwrap();
    let mut w = f.bind();
    let before = vec![Message::user("authored fixture")];
    let mut convo = Conversation::from(before.clone());
    convo.push(Message::assistant(vec![Block::text("fixture answer")]));
    let prefix = fs::read(&f.session.path).unwrap();
    let mut prepared = w.prepare_run(&before, &convo).unwrap();
    assert_eq!(fs::read(&f.session.path).unwrap(), prefix);
    let previous_digest = prepared.digest();
    prepared
        .push_record(&Record::Taint(crate::agent::Taint::default()))
        .unwrap();
    assert_ne!(prepared.digest(), previous_digest);
    let mut expected = prefix;
    expected.extend_from_slice(prepared.bytes());
    let endpoint = prepared.endpoint();
    let digest = prepared.digest();
    assert_eq!(digest, super::super::authority::digest(&expected));
    w.append_prepared(prepared).unwrap();
    let proof = w.sync_verified().unwrap();
    assert_eq!(proof.bytes(), expected);
    assert_eq!(proof.endpoint(), endpoint);
    assert_eq!(proof.digest(), digest);
}
#[test]
fn stale_and_different_live_writer_preparations_refuse_without_writing() {
    for foreign in [false, true] {
        let f = Fixture::new();
        let mut w = f.bind();
        let prepared = w.prepare_run(&[], &Conversation::default()).unwrap();
        if foreign {
            w = f.bind();
        } else {
            w.append(&message()).unwrap();
        }
        let bytes = fs::read(&f.session.path).unwrap();
        assert!(w.append_prepared(prepared).is_err());
        assert_eq!(fs::read(&f.session.path).unwrap(), bytes);
        refused(&mut w);
    }
}
#[test]
fn binding_refuses_leaf_ancestor_symlinks_and_existing_hardlinks() {
    for mode in 0..3 {
        let f = Fixture::new();
        let metadata = fs::metadata(&f.session.path).unwrap();
        let identity = NativeIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        let prefix = fs::read(&f.session.path).unwrap();
        let mut session = f.session.clone();
        match mode {
            0 => {
                let alias = f.dir.join("alias");
                symlink(&session.path, &alias).unwrap();
                session.path = alias;
            }
            1 => {
                let alias = f.dir.join("alias");
                fs::hard_link(&session.path, &alias).unwrap();
            }
            _ => {
                let alias = f.dir.join("alias");
                symlink(&f.dir, &alias).unwrap();
                session.path = alias.join(session.path.file_name().unwrap());
            }
        }
        assert!(BoundNativeSession::bind(&session, identity, &prefix).is_err());
    }
}

#[test]
fn programs_refuse_before_messages_and_old_program_history_is_not_reemitted() {
    let f = Fixture::new();
    let mut w = f.bind();
    let mut convo = Conversation::user("fixture run");
    convo
        .program_calls
        .push(crate::tool::code::record::ProgramCall::ran(
            "fixture-host-call",
            "fixture-program",
            "fs_read",
            serde_json::json!({}),
            false,
            false,
            std::sync::Arc::from("fixture bytes"),
        ));
    let bytes = fs::read(&f.session.path).unwrap();
    assert!(w.record_run(&[], &convo).is_err());
    assert_eq!(fs::read(&f.session.path).unwrap(), bytes);
    let mut w = f.bind();
    convo.program_calls_from = 1;
    w.record_run(&[], &convo).unwrap();
    let proof = w.sync_verified().unwrap();
    assert!(!super::super::recording::records(proof.bytes())
        .unwrap()
        .iter()
        .any(|r| matches!(r, Record::Program { .. })));
}

#[test]
fn mutation_during_sync_is_caught_by_the_post_sync_read() {
    let f = Fixture::new();
    let mut w = f.bind();
    w.append(&message()).unwrap();
    let path = f.session.path.clone();
    let original = fs::read(&path).unwrap();
    let mut changed = original.clone();
    let pos = changed
        .windows(b"authored fixture".len())
        .position(|b| b == b"authored fixture")
        .unwrap();
    changed[pos] = b'A';
    test_io::after_file_sync(move || fs::write(path, changed).unwrap());
    assert!(w.sync_verified().is_err());
    fs::write(&f.session.path, original).unwrap();
    refused(&mut w);
}
