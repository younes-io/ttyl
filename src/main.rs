use std::env;
use std::io;
use std::process::ExitCode;

use chrono::Local;
use devnull::api::{self, Client};
use devnull::app::App;
use devnull::device::{self, DeviceKey};
use devnull::model::LogPage;
use devnull::{signin, welcome};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if let [_, flag, url] = args.as_slice()
        && flag == "--redirect"
    {
        return report(signin::forward(url).map_err(|error| error.to_string()));
    }
    report(start())
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("devnull: {message}");
            ExitCode::FAILURE
        }
    }
}

fn start() -> Result<(), String> {
    let path = device::key_path().map_err(|error| error.to_string())?;
    let key = DeviceKey::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let client = Client::new(
        env::var("DEVNULL_URL").unwrap_or_else(|_| api::BASE.to_owned()),
        key,
    );
    let me = match client.me() {
        Ok(me) => Some(me),
        Err(error) if error.needs_sign_in() => None,
        Err(error) => return Err(error.to_string()),
    };
    let mut terminal = ratatui::init();
    let result = signed_in(&mut terminal, &client, me);
    ratatui::restore();
    result.map_err(|error| error.to_string())
}

fn signed_in(
    terminal: &mut DefaultTerminal,
    client: &Client,
    me: Option<api::Me>,
) -> io::Result<()> {
    let me = match me {
        Some(me) => me,
        None => match welcome::run(terminal, client)? {
            Some(me) => me,
            None => return Ok(()),
        },
    };
    let page = |json: &str| LogPage::parse(json).expect("fixture parses");
    let mut app = App::new(
        me.username,
        [
            page(include_str!("../fixtures/log-following.json")),
            page(include_str!("../fixtures/log-latest.json")),
            page(include_str!("../fixtures/log-trending.json")),
        ],
    );
    while !app.quit {
        terminal.draw(|frame| app.draw(frame, &Local::now()))?;
        if let Event::Key(key) = event::read()? {
            app.key(key);
        }
    }
    Ok(())
}
