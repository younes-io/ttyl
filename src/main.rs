use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write as _};
use std::ops::ControlFlow;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self as input, DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste,
    EnableFocusChange, Event as Input,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::Size;
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ttyl::api::{self, Call, Client};
use ttyl::attach::{self, Attachment};
use ttyl::desktop::{self, Clipped};
use ttyl::device::{self, DeviceKey};
use ttyl::draft::{self, Draft, Drafts, Target};
use ttyl::engine::{Effect, Engine, Event, WORKERS};
use ttyl::sched::Ticket;
use ttyl::{editor, signin, viewer, welcome};

const INPUT_POLL: Duration = Duration::from_millis(100);

enum Command<'a> {
    Run,
    Redirect(&'a str),
    Version,
    Help,
    Unknown(&'a str),
}

fn command<'a>(args: &[&'a str]) -> Command<'a> {
    match args {
        [] => Command::Run,
        ["--redirect", url] => Command::Redirect(url),
        ["--version" | "-V"] => Command::Version,
        ["--help" | "-h"] => Command::Help,
        ["--version" | "-V" | "--help" | "-h", arg, ..] | [arg, ..] => Command::Unknown(arg),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match command(&args) {
        Command::Run => report(start()),
        Command::Redirect(url) => report(signin::forward(url).map_err(|error| error.to_string())),
        Command::Version => {
            println!("ttyl {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Help => {
            println!(
                "{}\n\nUsage: ttyl [--version | --help]\n\nEnvironment:\n  DEVNULL_URL  the server to use (default: {})",
                env!("CARGO_PKG_DESCRIPTION"),
                api::BASE
            );
            ExitCode::SUCCESS
        }
        Command::Unknown(arg) => {
            eprintln!("ttyl: unknown argument '{arg}'\nRun 'ttyl --help' for usage.");
            ExitCode::from(2)
        }
    }
}

fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("ttyl: {message}");
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
    // Asked before input is read on another thread, which would take the terminal's answer.
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let _ = execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange);
    let result = run(&mut terminal, client, picker);
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableFocusChange);
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
    font: FontSize,
}

fn run(terminal: &mut DefaultTerminal, client: Arc<Client>, picker: Picker) -> io::Result<()> {
    let (events, inbox) = mpsc::channel();
    let jobs = spawn_workers(Arc::clone(&client), events.clone());
    let input = spawn_input(events.clone());
    let device_name = welcome::device_name();
    let drafts = draft::dir();
    let saved = drafts.as_deref().map(draft::load).unwrap_or_default();
    if let Some(dir) = drafts.as_deref() {
        prune(dir, &saved);
    }
    let mut runtime = Runtime {
        font: picker.font_size(),
        engine: Engine::new(device_name.clone(), Instant::now())
            .with_drafts(Drafts::new(saved))
            .with_picker(picker),
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
                Effect::Fetch(url) => {
                    let cells = terminal.size().unwrap_or_default();
                    let bound = Size::new(
                        cells.width.saturating_mul(self.font.width),
                        cells.height.saturating_mul(self.font.height),
                    );
                    let (client, events) = (Arc::clone(&self.client), self.events.clone());
                    thread::spawn(move || {
                        let image = (client.picture(&url).map_err(|error| error.to_string()))
                            .and_then(|bytes| viewer::decode(&bytes, bound));
                        let _ = events.send(Event::Fetched(url, image));
                    });
                }
                Effect::Edit(text) => {
                    let event = match self.edit(terminal, &text) {
                        Some(edited) => Event::Edited(edited),
                        None => Event::EditorFailed,
                    };
                    let effects = self.engine.update(event, Instant::now());
                    self.perform(terminal, effects)?;
                }
                Effect::Read(paths, pasted) => {
                    let looks = (paths.into_iter())
                        .map(|path| {
                            let look = attach::look(&path);
                            (path, look)
                        })
                        .collect();
                    let effects = self
                        .engine
                        .update(Event::Read(looks, pasted), Instant::now());
                    self.perform(terminal, effects)?;
                }
                Effect::Clip => {
                    let event = self.clip();
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

    /// What ctrl-v in the composer brings in. An image is kept as a file, the way a picked one is.
    fn clip(&self) -> Event {
        let read = |paths: Vec<PathBuf>| {
            let looks = (paths.into_iter())
                .map(|path| {
                    let look = attach::look(&path);
                    (path, look)
                })
                .collect();
            Event::Read(looks, None)
        };
        match desktop::paste() {
            Clipped::Image(bytes) => match self.drafts.as_deref().map(|dir| keep(dir, &bytes)) {
                Some(Ok(path)) => read(vec![path]),
                Some(Err(error)) => {
                    Event::Unclipped(format!("can't keep the pasted image: {error}."))
                }
                None => {
                    Event::Unclipped("can't keep the pasted image: no home directory.".to_owned())
                }
            },
            Clipped::Files(paths) => read(paths),
            Clipped::Text(text) => Event::Paste(text),
            Clipped::Empty => Event::Unclipped("no image or text on the clipboard.".to_owned()),
            Clipped::Unreachable => Event::Unclipped(
                "can't reach the clipboard. install wl-clipboard or xclip.".to_owned(),
            ),
        }
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
        let _ = execute!(
            out,
            DisableBracketedPaste,
            DisableFocusChange,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        let edited = editor::edit(&editor::chosen(), text);
        let _ = terminal::enable_raw_mode();
        let _ = execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        );
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
                Ok(Input::FocusGained) => Event::Focused,
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

/// Where pasted images are kept, beside the drafts that name them.
fn pasted(drafts: &Path) -> PathBuf {
    drafts.with_file_name("pasted")
}

/// Writes a pasted image to a file of its own, named for when it was pasted.
fn keep(drafts: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let dir = pasted(drafts);
    DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    let extension = attach::probe(bytes).map_or("png", |probe| probe.kind.extension());
    let stamp = Local::now().format("pasted-%H%M%S");
    for tries in 1.. {
        let name = match tries {
            1 => format!("{stamp}.{extension}"),
            _ => format!("{stamp}-{tries}.{extension}"),
        };
        let path = dir.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => return file.write_all(bytes).map(|()| path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("some name is free")
}

/// Removes pasted images no draft names any more, as after their commit went out.
fn prune(drafts: &Path, saved: &HashMap<Target, Draft>) {
    let named: HashSet<&Path> = (saved.values())
        .filter_map(|draft| match &draft.attachment {
            Attachment::Images(images) => Some(images),
            _ => None,
        })
        .flatten()
        .map(|image| image.path.as_path())
        .collect();
    let Ok(entries) = fs::read_dir(pasted(drafts)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !named.contains(path.as_path()) {
            let _ = fs::remove_file(path);
        }
    }
}
