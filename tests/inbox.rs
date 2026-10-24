mod common;

use std::time::{Duration, Instant};

use common::{account, answer_pages, boot, contains, press, reload, runs, screen, secs, tap};
use ratatui::crossterm::event::KeyCode;
use ttyl::api::{Call, Error, Reply};
use ttyl::engine::{Effect, Engine, Event};
use ttyl::model::{Filter, GroupPage, Kind, NotificationId, NotificationPage};
use ttyl::sched::Ticket;

#[test]
fn gn_opens_the_inbox_and_esc_returns_to_the_same_place() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "2jj", at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("notifications"), "{rows:#?}");
    assert!(!rows[0].contains("latest"), "{rows:#?}");
    assert_eq!(rows[1], " all  replies  quotes  stars  mentions  follows");
    assert!(
        rows[29].starts_with(" tab filter  esc back  ? help  Q quit"),
        "{rows:#?}"
    );

    press(&mut engine, "gn", at);
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(
        screen(&mut engine, 100, 30, at)[..29],
        before[..29],
        "one esc undoes gn, and the list still in flight spins in the hint"
    );
}

#[test]
fn esc_from_the_inbox_returns_to_the_thread_it_was_opened_from() {
    let at = Instant::now();
    let mut engine = boot(at);
    common::serve(&mut engine, at);
    tap(&mut engine, KeyCode::Enter, at);
    common::serve(&mut engine, at);
    let before = screen(&mut engine, 100, 30, at);

    press(&mut engine, "gn", at);
    assert!(engine.thread().is_none());
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(screen(&mut engine, 100, 30, at), before);
}

#[test]
fn tab_and_shift_tab_cycle_the_filters_and_wrap() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let bold = |engine: &mut ttyl::engine::Engine| {
        let buffer = common::buffer(engine, 100, 30, at);
        (0..100u16)
            .filter(|x| {
                buffer[(*x, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            })
            .map(|x| buffer[(x, 1)].symbol().to_owned())
            .collect::<String>()
    };
    assert_eq!(bold(&mut engine), "all");
    tap(&mut engine, KeyCode::Tab, at);
    assert_eq!(bold(&mut engine), "replies");
    tap(&mut engine, KeyCode::BackTab, at);
    tap(&mut engine, KeyCode::BackTab, at);
    assert_eq!(bold(&mut engine), "follows");
}

#[test]
fn the_narrow_filter_bar_and_hints_fit_44_columns() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let rows = screen(&mut engine, 44, 20, at);
    assert_eq!(rows[1], " all replies quotes stars mentions follows");
    assert!(contains(&rows[19..], "? help"), "{rows:#?}");
}

fn notifications() -> ttyl::model::NotificationPage {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications.json"
    ))
    .unwrap();
    ttyl::model::NotificationPage::parse(&json, ttyl::model::Filter::All).unwrap()
}

fn laid_out(page: &ttyl::model::NotificationPage, more: bool, width: u16) -> Vec<String> {
    let feed = ttyl::inbox::layout(
        page.filter,
        &page.groups,
        page.read_at,
        more,
        width,
        &common::now(),
    );
    feed.lines
        .iter()
        .map(|line| line.to_string().trim_end().to_owned())
        .collect()
}

#[test]
fn each_type_reads_as_the_web_words_it_with_the_time_at_the_right_edge() {
    let rows = laid_out(&notifications(), false, 100);
    let sentences: Vec<&str> = rows
        .iter()
        .filter(|row| row.starts_with('●') || row.starts_with('○'))
        .map(|row| row[4..].trim_end_matches(char::is_alphanumeric).trim())
        .collect();
    assert_eq!(
        sentences,
        [
            "Grace @grace replied to your commit",
            "Grace and 4 others starred your commit",
            "Margaret and 1 other followed you",
            "Ken @ken quoted your commit",
            "Linus @linus mentioned you",
            "Linus and 1 other starred your commit",
            "Your commit was approved.",
            "Your commit was rejected.",
            "You leveled up.",
            "That's the oldest notification. There's nothing before it.",
        ]
    );
    assert!(rows[0].ends_with(" 1h"), "{rows:#?}");
    assert_eq!(rows[0].chars().count(), 100);
}

#[test]
fn a_row_unread_before_this_load_gets_a_filled_node() {
    let rows = laid_out(&notifications(), true, 100);
    let nodes: String = rows
        .iter()
        .filter_map(|row| row.chars().next())
        .filter(|node| *node != '│')
        .collect();
    assert_eq!(nodes, "●●○○○○○○○", "null and equal read_at were unread");
}

#[test]
fn the_line_under_a_row_is_its_commit_or_the_rejection_reason() {
    let rows = laid_out(&notifications(), true, 100);
    let under = |sentence: &str| {
        let at = rows.iter().position(|row| row.contains(sentence)).unwrap();
        rows[at + 1].clone()
    };
    assert_eq!(
        under("replied"),
        "│ today's footgun: php› $name = 'devnull'; escape your output."
    );
    assert_eq!(under("rejected"), "│ reason: Links to a paid course.");
    assert_eq!(under("followed you"), "│", "a follow has no commit line");
    assert!(rows.last().unwrap().contains("leveled up"), "{rows:#?}");
}

#[test]
fn a_group_without_names_for_its_count_says_how_many_updates_it_holds() {
    let mut page = notifications();
    page.groups[0].event_count = 3;
    let rows = laid_out(&page, true, 100);
    assert!(
        rows[0].starts_with("● Grace replied to your commit · 3 updates"),
        "{rows:#?}"
    );
}

#[test]
fn rows_fit_44_columns_and_keep_the_time() {
    for row in laid_out(&notifications(), true, 44) {
        assert!(row.chars().count() <= 44, "{row}");
    }
    let rows = laid_out(&notifications(), true, 44);
    assert_eq!(rows[0], "● Grace @grace replied to your commit     1h");
    assert!(
        rows[3].ends_with("… 1h") || rows[3].ends_with(" 1h"),
        "{rows:#?}"
    );
}

#[test]
fn an_empty_list_reads_as_the_web_empty_copy_for_its_filter() {
    let mut page = notifications();
    page.groups.clear();
    page.filter = ttyl::model::Filter::Stars;
    assert_eq!(
        laid_out(&page, false, 100),
        [
            "$ cat /dev/notifications",
            "No stars yet. When someone stars one of your commits, it shows up here."
        ]
    );
}

fn page_of(filter: Filter, next_cursor: Option<&str>) -> NotificationPage {
    NotificationPage {
        filter,
        next_cursor: next_cursor.map(str::to_owned),
        ..notifications()
    }
}

/// The one list call in `effects`, with its ticket.
fn list_call(effects: &[Effect]) -> (Ticket, Filter, Option<String>) {
    let lists: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Run(ticket, Call::Notifications { filter, cursor }) => {
                Some((*ticket, *filter, cursor.clone()))
            }
            _ => None,
        })
        .collect();
    let [list] = lists.as_slice() else {
        panic!("one list call: {effects:?}");
    };
    list.clone()
}

fn land(engine: &mut Engine, ticket: Ticket, page: NotificationPage, at: Instant) {
    engine.update(Event::Reply(ticket, Ok(Reply::Notifications(page))), at);
}

fn opened(engine: &mut Engine, at: Instant) {
    let (ticket, _, _) = list_call(&press(engine, "gn", at));
    land(engine, ticket, page_of(Filter::All, None), at);
}

fn count_shown(engine: &mut Engine, at: Instant) -> bool {
    screen(engine, 100, 30, at)[0].contains("● 3  @ada")
}

#[test]
fn gn_asks_for_all_notifications_on_the_same_update() {
    let at = Instant::now();
    let mut engine = boot(at);
    let effects = press(&mut engine, "gn", at);
    let (_, filter, cursor) = list_call(&effects);
    assert_eq!((filter, cursor), (Filter::All, None));
}

#[test]
fn nothing_loads_the_list_while_the_screen_is_closed() {
    let at = Instant::now();
    let mut engine = boot(at);
    for tick in 0..200 {
        for effect in engine.pump(at + secs(tick)) {
            match effect {
                Effect::Run(ticket, Call::Me) => {
                    let reply = Ok(Reply::Account(account(3)));
                    engine.update(Event::Reply(ticket, reply), at + secs(tick));
                }
                Effect::Run(_, Call::Since { .. }) => {}
                other => panic!("{other:?}"),
            }
        }
    }
}

#[test]
fn the_poll_never_clears_the_count_and_a_loaded_list_does() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _, _) = list_call(&press(&mut engine, "gn", at));
    for effect in engine.pump(at + secs(30)) {
        if let Effect::Run(me, Call::Me) = effect {
            engine.update(Event::Reply(me, Ok(Reply::Account(account(3)))), at);
        }
    }
    assert!(count_shown(&mut engine, at + secs(30)));

    land(
        &mut engine,
        ticket,
        page_of(Filter::All, None),
        at + secs(30),
    );
    assert!(!count_shown(&mut engine, at + secs(30)));
    let rows = screen(&mut engine, 100, 30, at + secs(30));
    assert!(rows[2].starts_with(" ● Grace @grace replied"), "{rows:#?}");
}

#[test]
fn a_failed_first_load_keeps_the_count_and_retries_on_its_timer() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _, _) = list_call(&press(&mut engine, "gn", at));
    engine.update(Event::Reply(ticket, Err(Error::Unexpected(502))), at);

    assert!(count_shown(&mut engine, at));
    let rows = screen(&mut engine, 100, 30, at + secs(12));
    assert!(contains(&rows, "retrying in 18s"), "{rows:#?}");
    assert!(runs(&engine.pump(at + secs(29))).is_empty());
    let (_, filter, _) = list_call(&engine.pump(at + secs(30)));
    assert_eq!(filter, Filter::All);
}

#[test]
fn a_failed_reload_keeps_the_rows_and_says_so_in_the_hint() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let (ticket, _, _) = list_call(&reload(&mut engine, at));
    engine.update(Event::Reply(ticket, Err(Error::Unexpected(502))), at);

    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[2].starts_with(" ● Grace @grace replied"), "{rows:#?}");
    assert!(rows[29].starts_with(" fatal: "), "{rows:#?}");
    assert!(rows[29].contains("retrying in 30s"), "{rows:#?}");
}

#[test]
fn a_timer_left_from_a_failure_does_not_load_once_the_screen_is_closed() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _, _) = list_call(&press(&mut engine, "gn", at));
    engine.update(Event::Reply(ticket, Err(Error::Unexpected(502))), at);
    tap(&mut engine, KeyCode::Esc, at);
    let effects = engine.pump(at + secs(30));
    assert!(
        !runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Notifications { .. })),
        "{effects:?}"
    );
}

#[test]
fn each_filter_asks_with_its_type_and_shows_at_once_when_it_comes_back() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    press(&mut engine, "jj", at);
    let before = screen(&mut engine, 100, 30, at);

    let (ticket, filter, _) = list_call(&tap(&mut engine, KeyCode::Tab, at));
    assert_eq!(filter, Filter::Replies);
    let mut replies = page_of(Filter::Replies, None);
    replies.groups.truncate(1);
    land(&mut engine, ticket, replies, at);
    assert!(!contains(&screen(&mut engine, 100, 30, at), "starred"));

    let (_, filter, _) = list_call(&tap(&mut engine, KeyCode::BackTab, at));
    assert_eq!(filter, Filter::All);
    let mut back = screen(&mut engine, 100, 30, at);
    back[29] = before[29].clone();
    assert_eq!(
        back, before,
        "the list and its selection, before the reload lands"
    );
}

#[test]
fn the_loading_line_waits_300ms() {
    let at = Instant::now();
    let mut engine = boot(at);
    press(&mut engine, "gn", at);
    let early = screen(&mut engine, 100, 30, at + Duration::from_millis(299));
    assert!(!contains(&early, "loading notifications"));
    let late = screen(&mut engine, 100, 30, at + Duration::from_millis(300));
    assert_eq!(late[2].trim(), "loading notifications", "{late:#?}");
}

#[test]
fn older_groups_load_five_rows_from_the_end_one_page_at_a_time() {
    let at = Instant::now();
    let mut engine = boot(at);
    let (ticket, _, _) = list_call(&press(&mut engine, "gn", at));
    land(&mut engine, ticket, page_of(Filter::All, Some("p2")), at);

    assert!(runs(&press(&mut engine, "jj", at)).is_empty());
    let (ticket, _, cursor) = list_call(&press(&mut engine, "j", at));
    assert_eq!(cursor.as_deref(), Some("p2"));
    assert!(
        runs(&press(&mut engine, "jG", at)).is_empty(),
        "one in flight"
    );

    let mut older = page_of(Filter::All, Some("p3"));
    for group in &mut older.groups {
        group.event.id = NotificationId(format!("old-{}", group.event.id.0));
    }
    land(&mut engine, ticket, older, at);
    assert!(engine.pump(at).is_empty(), "G was the old end");
    let (ticket, _, cursor) = list_call(&press(&mut engine, "G", at));
    assert_eq!(cursor.as_deref(), Some("p3"));

    land(&mut engine, ticket, page_of(Filter::All, Some("p4")), at);
    assert!(
        runs(&engine.pump(at)).is_empty(),
        "a page of rows already listed ends the list"
    );
}

fn group_page() -> GroupPage {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/notifications-group.json"
    ))
    .unwrap();
    GroupPage::parse(&json).unwrap()
}

/// The one group call in `effects`, with its ticket.
fn group_call(effects: &[Effect]) -> (Ticket, NotificationId, Option<String>) {
    let groups: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Run(ticket, Call::Group { id, cursor, .. }) => {
                Some((*ticket, id.clone(), cursor.clone()))
            }
            _ => None,
        })
        .collect();
    let [group] = groups.as_slice() else {
        panic!("one group call: {effects:?}");
    };
    group.clone()
}

#[test]
fn enter_on_a_single_event_opens_its_thread_and_esc_returns() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let before = screen(&mut engine, 100, 30, at);

    let effects = press(&mut engine, "o", at);
    assert!(
        runs(&effects)
            .iter()
            .any(|call| matches!(call, Call::Commit(_))),
        "{effects:?}"
    );
    assert!(engine.thread().is_some());
    tap(&mut engine, KeyCode::Esc, at);
    assert_eq!(screen(&mut engine, 100, 30, at)[..29], before[..29]);
}

#[test]
fn enter_on_a_group_lists_its_events_and_enter_there_opens_one() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let (ticket, id, cursor) = group_call(&press(&mut engine, "jo", at));
    assert_eq!(id.0, "9e000000-0000-4000-8000-000000000002");
    assert_eq!(cursor, None);
    engine.update(Event::Reply(ticket, Ok(Reply::Group(group_page()))), at);

    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains("notifications"), "{rows:#?}");
    assert!(rows[1].contains("starred"), "{rows:#?}");
    assert!(contains(&rows[2..], "@grace"), "{rows:#?}");
    assert!(contains(&rows[2..], "@ada"), "{rows:#?}");
    assert!(rows[29].contains("enter open"), "{rows:#?}");

    press(&mut engine, "jo", at);
    assert!(engine.thread().is_some());
    tap(&mut engine, KeyCode::Esc, at);
    tap(&mut engine, KeyCode::Esc, at);
    assert!(screen(&mut engine, 100, 30, at)[1].starts_with(" all"));
}

#[test]
fn a_level_up_offers_no_enter() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    assert!(runs(&press(&mut engine, "Go", at)).is_empty());
    let rows = screen(&mut engine, 100, 30, at);
    assert!(!rows[29].contains("enter open"), "{rows:#?}");
}

#[test]
fn the_people_in_a_follow_group_open_their_profile() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let (ticket, _, _) = group_call(&press(&mut engine, "jjo", at));
    let mut page = group_page();
    for event in &mut page.events {
        event.kind = Kind::Followed;
        event.commit = None;
    }
    let follower = page.events[0].actor.username.clone();
    engine.update(Event::Reply(ticket, Ok(Reply::Group(page))), at);
    let effects = press(&mut engine, "o", at);
    assert!(
        runs(&effects).contains(&&Call::User(follower.clone())),
        "{:#?}",
        runs(&effects)
    );
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[0].contains(&format!("@{follower}")), "{rows:#?}");
}

#[test]
fn a_loaded_group_never_clears_the_count() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let (ticket, _, _) = group_call(&press(&mut engine, "jo", at));
    for effect in engine.pump(at + secs(30)) {
        if let Effect::Run(me, Call::Me) = effect {
            engine.update(Event::Reply(me, Ok(Reply::Account(account(3)))), at);
        }
    }
    let reply = Ok(Reply::Group(group_page()));
    engine.update(Event::Reply(ticket, reply), at + secs(30));
    assert!(count_shown(&mut engine, at + secs(30)));
}

#[test]
fn r_on_a_reply_opens_the_composer_on_it_and_y_copies_its_link() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let url = "https://dev-null.sh/@grace/commits/2c3d4e5f-0000-4000-8000-000000000201";
    assert_eq!(press(&mut engine, "y", at), [Effect::Copy(url.to_owned())]);

    press(&mut engine, "r", at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(contains(&rows, " Reply to @grace ─"), "{rows:#?}");
    assert!(
        contains(&rows, "replied to your commit"),
        "the row stays above: {rows:#?}"
    );
}

#[test]
fn s_does_nothing_on_a_notification_since_its_star_is_unknown() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let before = screen(&mut engine, 100, 30, at);
    assert!(press(&mut engine, "s", at).is_empty());
    assert_eq!(screen(&mut engine, 100, 30, at), before);
}

#[test]
fn f_follows_back_from_a_follow_group() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    let (ticket, _, _) = group_call(&press(&mut engine, "jjo", at));
    let mut page = group_page();
    for event in &mut page.events {
        event.kind = Kind::Followed;
        event.commit = None;
    }
    engine.update(Event::Reply(ticket, Ok(Reply::Group(page))), at);
    let rows = screen(&mut engine, 100, 30, at);
    assert!(rows[29].contains("f follow back"), "{rows:#?}");
    assert_eq!(
        runs(&press(&mut engine, "f", at)),
        [&Call::User("grace".to_owned())]
    );
}

fn poll(engine: &mut Engine, unread: u32, at: Instant) {
    for effect in engine.pump(at) {
        if let Effect::Run(me, Call::Me) = effect {
            engine.update(Event::Reply(me, Ok(Reply::Account(account(unread)))), at);
        }
    }
}

fn count_lit(engine: &mut Engine, at: Instant) -> bool {
    let buffer = common::buffer(engine, 100, 30, at);
    let row = screen(engine, 100, 30, at)[0].clone();
    let x = row.find("● ").expect("the count shows") + "● ".len();
    let x = u16::try_from(row[..x].chars().count()).unwrap();
    buffer[(x, 0)]
        .modifier
        .contains(ratatui::style::Modifier::BOLD)
}

#[test]
fn a_rising_count_lights_up_for_five_seconds_and_a_steady_one_does_not() {
    let at = Instant::now();
    let mut engine = boot(at);
    poll(&mut engine, 3, at + secs(30));
    assert!(!count_lit(&mut engine, at + secs(30)), "no rise, no glow");

    poll(&mut engine, 5, at + secs(60));
    assert!(screen(&mut engine, 100, 30, at + secs(60))[0].contains("● 5  @ada"));
    assert!(count_lit(&mut engine, at + secs(64)));
    engine.pump(at + secs(65));
    assert!(!count_lit(&mut engine, at + secs(65)));
}

#[test]
fn a_refresh_says_so_when_the_list_lands_and_opening_stays_quiet() {
    let at = Instant::now();
    let mut engine = boot(at);
    opened(&mut engine, at);
    assert!(
        !contains(&screen(&mut engine, 100, 30, at), "refreshed"),
        "opening is not a refresh"
    );
    let effects = reload(&mut engine, at + secs(1));
    answer_pages(&mut engine, &effects, at + secs(1));
    let (ticket, _, _) = list_call(&effects);
    land(
        &mut engine,
        ticket,
        page_of(Filter::All, None),
        at + secs(2),
    );
    let rows = screen(&mut engine, 100, 30, at + secs(2));
    assert_eq!(rows[29].trim_end(), " notifications refreshed");
}
