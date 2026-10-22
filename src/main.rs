use std::env;
use std::io;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use devnull::api::{self, Call, Client};
use devnull::device::{self, DeviceKey};
use devnull::draft::{self, Drafts};
use devnull::engine::{Effect, Engine, Event, WORKERS};
use devnull::sched::Ticket;
use devnull::{desktop, editor, signin, welcome};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self as input, DisableBracketedPaste, EnableBracketedPaste, Event as Input,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

const INPUT_POLL: Duration = Duration::from_millis(100);

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
    let client = Arc::new(Client::new(
        env::var("DEVNULL_URL").unwrap_or_else(|_| api::BASE.to_owned()),
        key,
    ));
    let mut terminal = ratatui::init();
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    let result = run(&mut terminal, client);
    let _ = execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result.map_err(|error| error.to_string())
}

struct Runtime {
    engine: Engine,
    client: Arc<Client>,
    events: Sender<Event>,
    jobs: Sender<(Ticket, Call)>,
    input: Sender<Pause>,
    device_name: String,
    drafts: Option<PathBuf>,
}

fn run(terminal: &mut DefaultTerminal, client: Arc<Client>) -> io::Result<()> {
    let (events, inbox) = mpsc::channel();
    let jobs = spawn_workers(Arc::clone(&client), events.clone());
    let input = spawn_input(events.clone());
    let device_name = welcome::device_name();
    let drafts = draft::dir();
    let saved = drafts.as_deref().map(draft::load).unwrap_or_default();
    let mut runtime = Runtime {
        engine: Engine::new(device_name.clone(), Instant::now()).with_drafts(Drafts::new(saved)),
        client,
        events,
        jobs,
        input,
        device_name,
        drafts,
    };
    loop {
        let now = Instant::now();
        let effects = runtime.engine.pump(now);
        if let ControlFlow::Break(result) = runtime.perform(terminal, effects) {
            return result;
        }
        terminal.draw(|frame| runtime.engine.draw(frame, now, &Local::now()))?;
        let event = match runtime.engine.next_wake(Instant::now()) {
            Some(wake) => {
                match inbox.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(RecvTimeoutError::Timeout) => Event::Tick,
                    Err(RecvTimeoutError::Disconnected) => {
                        unreachable!("the runtime holds a sender")
                    }
                }
            }
            None => inbox.recv().expect("the runtime holds a sender"),
        };
        let mut pending = vec![event];
        pending.extend(inbox.try_iter());
        for event in pending {
            let effects = runtime.engine.update(event, Instant::now());
            if let ControlFlow::Break(result) = runtime.perform(terminal, effects) {
                return result;
            }
        }
    }
}

impl Runtime {
    fn perform(
        &mut self,
        terminal: &mut DefaultTerminal,
        effects: Vec<Effect>,
    ) -> ControlFlow<io::Result<()>> {
        for effect in effects {
            match effect {
                Effect::Run(ticket, call) => {
                    let _ = self.jobs.send((ticket, call));
                }
                Effect::StartSignIn => {
                    let started =
                        welcome::start(&self.client, &self.device_name, self.events.clone());
                    let effects = self.engine.update(Event::SignIn(started), Instant::now());
                    self.perform(terminal, effects)?;
                }
                Effect::Quit => return ControlFlow::Break(Ok(())),
                Effect::Abort(message) => {
                    return ControlFlow::Break(Err(io::Error::other(message)));
                }
                Effect::Copy(text) => desktop::copy(&text),
                Effect::Open(url) => desktop::open(&url),
                Effect::Edit(text) => {
                    let event = match self.edit(terminal, &text) {
                        Some(edited) => Event::Edited(edited),
                        None => Event::EditorFailed,
                    };
                    let effects = self.engine.update(event, Instant::now());
                    self.perform(terminal, effects)?;
                }
                Effect::Draft(write) => {
                    let stored = match &self.drafts {
                        Some(dir) => draft::store(dir, &write),
                        None => Err(io::Error::other("no home directory")),
                    };
                    if let Err(error) = stored {
                        let effects = self
                            .engine
                            .update(Event::DraftFailed(error), Instant::now());
                        self.perform(terminal, effects)?;
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }

    fn edit(&mut self, terminal: &mut DefaultTerminal, text: &str) -> Option<String> {
        let (parked, is_parked) = mpsc::channel();
        let (resume, until_resumed) = mpsc::channel();
        let pause = Pause {
            parked,
            until_resumed,
        };
        if self.input.send(pause).is_err() || is_parked.recv().is_err() {
            return None;
        }
        let mut out = io::stdout();
        let _ = execute!(out, DisableBracketedPaste, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        let edited = editor::edit(&editor::chosen(), text);
        let _ = terminal::enable_raw_mode();
        let _ = execute!(out, EnterAlternateScreen, EnableBracketedPaste);
        let _ = terminal.clear();
        drop(resume);
        edited
    }
}

struct Pause {
    parked: Sender<()>,
    until_resumed: Receiver<()>,
}

fn spawn_input(events: Sender<Event>) -> Sender<Pause> {
    let (pauses, control) = mpsc::channel::<Pause>();
    thread::spawn(move || {
        loop {
            if let Ok(pause) = control.try_recv() {
                let _ = pause.parked.send(());
                let _ = pause.until_resumed.recv();
                continue;
            }
            match input::poll(INPUT_POLL) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    let _ = events.send(Event::InputLost(error));
                    return;
                }
            }
            let event = match input::read() {
                Ok(Input::Key(key)) => Event::Key(key),
                Ok(Input::Paste(text)) => Event::Paste(text),
                Ok(Input::Resize(..)) => Event::Tick,
                Ok(_) => continue,
                Err(error) => {
                    let _ = events.send(Event::InputLost(error));
                    return;
                }
            };
            if events.send(event).is_err() {
                return;
            }
        }
    });
    pauses
}

fn spawn_workers(client: Arc<Client>, events: Sender<Event>) -> Sender<(Ticket, Call)> {
    let (jobs, queue) = mpsc::channel::<(Ticket, Call)>();
    let queue = Arc::new(Mutex::new(queue));
    for _ in 0..WORKERS {
        let client = Arc::clone(&client);
        let events = events.clone();
        let queue = Arc::clone(&queue);
        thread::spawn(move || {
            loop {
                let job = queue.lock().map(|queue| queue.recv());
                let Ok(Ok((ticket, call))) = job else {
                    return;
                };
                let result = client.call(&call);
                if events.send(Event::Reply(ticket, result)).is_err() {
                    return;
                }
            }
        });
    }
    jobs
}
