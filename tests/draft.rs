mod common;

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use ttyl::draft::{self, Draft, Drafts, Target, Write};
use ttyl::model::CommitId;

fn reply() -> Target {
    Target::Reply(CommitId("9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61".to_owned()))
}

#[test]
fn each_change_is_one_write_and_an_empty_draft_is_dropped() {
    let mut drafts = Drafts::default();
    assert_eq!(
        drafts.write(&Target::New, &Draft::text("hello")),
        Some(Write::Save(Target::New, Draft::text("hello")))
    );
    assert_eq!(drafts.write(&Target::New, &Draft::text("hello")), None);
    assert_eq!(drafts.get(&Target::New), Some(&Draft::text("hello")));
    assert_eq!(
        drafts.write(&Target::New, &Draft::text("  \n")),
        Some(Write::Drop(Target::New))
    );
    assert_eq!(drafts.get(&Target::New), None);
    assert_eq!(drafts.write(&Target::New, &Draft::text("")), None);
}

#[test]
fn targets_keep_their_own_drafts() {
    let mut drafts = Drafts::default();
    drafts.write(&Target::New, &Draft::text("new"));
    drafts.write(&reply(), &Draft::text("reply"));
    let quote = Target::Quote(CommitId("9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61".to_owned()));
    drafts.write(&quote, &Draft::text("quote"));
    assert_eq!(drafts.get(&Target::New), Some(&Draft::text("new")));
    assert_eq!(drafts.get(&reply()), Some(&Draft::text("reply")));
    assert_eq!(drafts.get(&quote), Some(&Draft::text("quote")));
    assert_eq!(drafts.discard(&reply()), Some(Write::Drop(reply())));
    assert_eq!(drafts.discard(&reply()), None);
}

#[test]
fn drafts_survive_on_disk_and_come_back() {
    let dir = common::scratch("drafts").join("ttyl/drafts");
    draft::store(
        &dir,
        &Write::Save(Target::New, Draft::text("first\nsecond")),
    )
    .unwrap();
    draft::store(&dir, &Write::Save(reply(), Draft::text("a reply"))).unwrap();
    draft::store(&dir, &Write::Save(reply(), Draft::text("a better reply"))).unwrap();
    let odd = Target::Quote(CommitId("../escape".to_owned()));
    draft::store(&dir, &Write::Save(odd, Draft::text("stays in memory"))).unwrap();

    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["new", "reply-9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61"],
        "no temp file is left behind"
    );
    let mode = fs::metadata(dir.join("new")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    fs::write(dir.join("quote-0a000000"), "").unwrap();
    fs::write(dir.join("notes.txt"), "not a draft").unwrap();
    let loaded = draft::load(&dir);
    assert_eq!(
        loaded,
        HashMap::from([
            (Target::New, Draft::text("first\nsecond")),
            (reply(), Draft::text("a better reply")),
        ])
    );

    draft::store(&dir, &Write::Drop(Target::New)).unwrap();
    draft::store(&dir, &Write::Drop(Target::New)).unwrap();
    assert!(!dir.join("new").exists());
}

#[test]
fn the_engine_starts_with_the_saved_drafts() {
    let at = std::time::Instant::now();
    let engine =
        ttyl::engine::Engine::new("ttyl".to_owned(), at).with_drafts(Drafts::new(HashMap::from([
            (Target::New, Draft::text("unsent")),
        ])));
    assert_eq!(engine.draft(&Target::New), Some("unsent"));
    assert_eq!(engine.draft(&reply()), None);
}
