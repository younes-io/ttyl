use std::io;

use chrono::Local;
use devnull::app::App;
use devnull::model::LogPage;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event};

fn main() -> io::Result<()> {
    let page = |json: &str| LogPage::parse(json).expect("fixture parses");
    let mut app = App::new([
        page(include_str!("../fixtures/log-following.json")),
        page(include_str!("../fixtures/log-latest.json")),
        page(include_str!("../fixtures/log-trending.json")),
    ]);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    while !app.quit {
        terminal.draw(|frame| app.draw(frame, &Local::now()))?;
        if let Event::Key(key) = event::read()? {
            app.key(key);
        }
    }
    Ok(())
}
