mod common;

use std::time::Instant;

use common::{account, contains, log_of, not_found, press, profile, screen, tap};
use proptest::prelude::*;
use ratatui::crossterm::event::KeyCode;
use serde_json::{Value, json};
use ttyl::api::{Call, Error, Reply};
use ttyl::clock::Spark;
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{CommitId, CommitPage, Focus, Log, LogPage, RepliesPage};

const MAX_DEPTH: usize = 6;
const KEYS: [&str; 18] = [
    "j",
    "k",
    "gg",
    "G",
    " ",
    "o",
    "enter",
    "esc",
    "backspace",
    "R",
    "s",
    "f",
    "P",
    "y",
    "O",
    "D",
    "!",
    "2",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Reply,
    Quote,
    Deleted,
}

#[derive(Debug, Clone)]
struct Node {
    parent: Option<usize>,
    kind: Kind,
    lines: usize,
}

#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
    per_page: usize,
}

impl Tree {
    fn id(index: usize) -> String {
        format!("7b000000-0000-4000-8000-{index:012x}")
    }

    fn index(id: &str) -> usize {
        usize::from_str_radix(&id[24..], 16).unwrap()
    }

    fn depth(&self, mut index: usize) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.nodes[index].parent {
            index = parent;
            depth += 1;
        }
        depth
    }

    fn children(&self, owner: usize) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|index| self.nodes[*index].parent == Some(owner))
            .collect()
    }

    fn card(&self, index: usize) -> Value {
        let node = &self.nodes[index];
        let id = Tree::id(index);
        let parent = node.parent.map(Tree::id);
        if node.kind == Kind::Deleted {
            return json!({
                "id": id,
                "parent_id": parent,
                "deleted_at": "2026-10-21T09:00:00Z",
                "replies_count": self.children(index).len(),
            });
        }
        let body = (0..node.lines)
            .map(|line| format!("line {line} of node {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut card = template();
        card["id"] = json!(id);
        card["short_hash"] = json!(format!("{index:07x}"));
        card["body"] = json!(body);
        card["body_segments"] = json!([{ "type": "text", "text": body }]);
        card["replies_count"] = json!(self.children(index).len());
        let (parent_id, quoted) = match node.kind {
            Kind::Quote => (None, parent),
            _ => (parent, None),
        };
        card["parent_id"] = json!(parent_id);
        card["quoted_commit_id"] = json!(quoted);
        card
    }

    fn commit(&self, index: usize) -> Option<CommitPage> {
        if self.nodes[index].kind == Kind::Deleted {
            return None;
        }
        let mut ancestors = Vec::new();
        let mut at = index;
        while let (Kind::Reply, Some(parent)) = (self.nodes[at].kind, self.nodes[at].parent) {
            ancestors.push(self.card(parent));
            at = parent;
        }
        ancestors.reverse();
        let page = json!({ "data": self.card(index), "ancestors": ancestors });
        Some(CommitPage::parse(&page.to_string()).unwrap())
    }

    fn replies(&self, owner: usize, cursor: Option<&str>) -> RepliesPage {
        let children = self.children(owner);
        let start = cursor.map_or(0, |cursor| cursor.parse().unwrap());
        let end = (start + self.per_page).min(children.len());
        let data: Vec<Value> = children[start..end]
            .iter()
            .map(|child| self.card(*child))
            .collect();
        let next = (end < children.len()).then(|| end.to_string());
        let page = json!({ "data": data, "meta": { "next_cursor": next } });
        RepliesPage::parse(&page.to_string()).unwrap()
    }

    fn answer(&self, call: &Call) -> Result<Reply, Error> {
        match call {
            Call::Me => Ok(Reply::Account(account(3))),
            Call::Page(log) | Call::Since { log, .. } | Call::Older { log, .. } => {
                Ok(Reply::Page(LogPage {
                    log: *log,
                    ..log_of(&[self.card(0)])
                }))
            }
            Call::Commit(id) => self
                .commit(Tree::index(&id.0))
                .map(|page| Reply::Thread(Box::new(page)))
                .ok_or_else(not_found),
            Call::Replies { id, cursor } => Ok(Reply::Replies(
                self.replies(Tree::index(&id.0), cursor.as_deref()),
            )),
            Call::Star { id, on } => {
                let page = self.commit(Tree::index(&id.0)).ok_or_else(not_found)?;
                let mut card = page.commit;
                card.starred = *on;
                Ok(Reply::Commit(Box::new(Focus {
                    card,
                    replies_count: page.replies_count,
                })))
            }
            Call::User(_) => Ok(profile("grace", false)),
            Call::Follow { .. } | Call::Report { .. } => Ok(Reply::Done),
            other => panic!("not a call the log or a thread makes: {other:?}"),
        }
    }
}

fn template() -> Value {
    let world: Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/fixtures/world.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    world["replies"]["9f3c2a10-4b7e-4d21-8a6f-1c2d3e4f5a61"][""]["data"][0].clone()
}

fn tree() -> impl Strategy<Value = Tree> {
    (
        prop::collection::vec((any::<u16>(), 0u8..8, 1usize..12), 0..24),
        1usize..5,
    )
        .prop_map(|(picks, per_page)| {
            let mut tree = Tree {
                nodes: vec![Node {
                    parent: None,
                    kind: Kind::Reply,
                    lines: 2,
                }],
                per_page,
            };
            for (index, (parent, kind, lines)) in picks.into_iter().enumerate() {
                let mut parent = usize::from(parent) % (index + 1);
                while tree.depth(parent) >= MAX_DEPTH {
                    parent = tree.nodes[parent].parent.unwrap();
                }
                let kind = match kind {
                    0 => Kind::Deleted,
                    1 => Kind::Quote,
                    _ => Kind::Reply,
                };
                tree.nodes.push(Node {
                    parent: Some(parent),
                    kind,
                    lines,
                });
            }
            tree
        })
}

fn settle(
    engine: &mut Engine,
    tree: &Tree,
    mut effects: Vec<Effect>,
    size: (u16, u16),
    at: Instant,
) {
    loop {
        screen(engine, size.0, size.1, at);
        effects.extend(engine.pump(at));
        if effects.is_empty() {
            return;
        }
        for effect in std::mem::take(&mut effects) {
            if let Effect::Run(ticket, call) = effect {
                effects.extend(engine.update(Event::Reply(ticket, tree.answer(&call)), at));
            }
        }
    }
}

fn hit(engine: &mut Engine, key: &str, at: Instant) -> Vec<Effect> {
    match key {
        "enter" => tap(engine, KeyCode::Enter, at),
        "esc" => tap(engine, KeyCode::Esc, at),
        "backspace" => tap(engine, KeyCode::Backspace, at),
        keys => press(engine, keys, at),
    }
}

fn header_shown(rows: &[String], tree: &Tree, selected: &CommitId) -> bool {
    let index = Tree::index(&selected.0);
    if tree.nodes[index].kind == Kind::Deleted {
        let gone = format!("fatal: bad object {}", selected.short_hash());
        return rows[2..].iter().any(|row| row.ends_with(&gone));
    }
    let first = format!("line 0 of node {index}");
    let Some(text) = rows.iter().position(|row| row.ends_with(&first)) else {
        return false;
    };
    let header = if rows[text - 1].contains('↳') {
        text - 2
    } else {
        text - 1
    };
    header >= 2 && rows[header].contains('●')
}

fn reporting(rows: &[String]) -> bool {
    rows.last()
        .is_some_and(|hints| hints.contains("esc cancel"))
}

/// The first log's screen as it reads with the open log's tab marked instead, since every log
/// serves the same tree.
fn open(log: &[String], engine: &Engine) -> Vec<String> {
    let mut log = log.to_vec();
    if engine
        .log()
        .is_some_and(|screen| screen.active() == Log::Latest)
    {
        log[1] = log[1].replace("/following ── latest", "following ── /latest");
    }
    log
}

fn above_the_hints(rows: &[String]) -> Vec<String> {
    rows[..rows.len() - 1].to_vec()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn the_selected_header_stays_on_screen_and_back_restores_the_log(
        tree in tree(),
        width in prop::sample::select(vec![44u16, 100]),
        height in 10u16..40,
        keys in prop::collection::vec(prop::sample::select(KEYS.to_vec()), 0..30),
    ) {
        let at = Instant::now();
        let size = (width, height);
        let mut engine = Engine::new("ttyl".to_owned(), at);
        settle(&mut engine, &tree, Vec::new(), size, at);
        let log = above_the_hints(&screen(&mut engine, width, height, at));

        let effects = tap(&mut engine, KeyCode::Enter, at);
        settle(&mut engine, &tree, effects, size, at);
        for key in keys {
            let effects = hit(&mut engine, key, at);
            settle(&mut engine, &tree, effects, size, at);
            let rows = screen(&mut engine, width, height, at);
            match engine.thread() {
                _ if reporting(&rows) => {
                    prop_assert!(contains(&rows, " report "), "{key:?}: {rows:#?}");
                }
                Some(thread) => {
                    let selected = thread.selected();
                    prop_assert!(
                        header_shown(&rows, &tree, selected),
                        "{key:?} lost {selected:?}: {rows:#?}"
                    );
                }
                None => {
                    let rows = screen(&mut engine, width, height, at + Spark::Star.lasts());
                    prop_assert_eq!(&above_the_hints(&rows), &open(&log, &engine), "{:?} came back elsewhere", key);
                    let effects = tap(&mut engine, KeyCode::Enter, at);
                    settle(&mut engine, &tree, effects, size, at);
                }
            }
        }
        while engine.thread().is_some() {
            let effects = tap(&mut engine, KeyCode::Esc, at);
            settle(&mut engine, &tree, effects, size, at);
        }
        let rows = screen(&mut engine, width, height, at + Spark::Star.lasts());
        prop_assert_eq!(&above_the_hints(&rows), &open(&log, &engine));
    }
}
