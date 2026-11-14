use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, IsTerminal as _, Read as _, Write as _};
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
    EnableFocusChange, Event as Input, KeyEventKind,
};
use ratatui::crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{cursor, execute};
use ratatui::layout::Size;
use ratatui_image::FontSize;
use ratatui_image::picker::Picker;
use ttyl::api::{self, Call, Client, Reply};
use ttyl::attach::{self, Attachment};
use ttyl::cache::{Cache, Start};
use ttyl::desktop::{self, Clipped};
use ttyl::device::{self, DeviceKey};
use ttyl::draft::{self, Draft, Drafts, Target};
use ttyl::engine::{self, Effect, Engine, Event, WORKERS};
use ttyl::gitlog::{self, Format};
use ttyl::model::{Log, Status};
use ttyl::sched::Ticket;
use ttyl::{body, compose, editor, signin, theme, viewer, welcome};

const INPUT_POLL: Duration = Duration::from_millis(100);
/// The file in ttyl's state that remembers the icons the viewer picked on the welcome screen.
const ICONS: &str = "icons";

enum Command<'a> {
    Run,
    Redirect(&'a str),
    Status,
    /// A commit from the shell, with its text from `-m` or else from stdin.
    Commit(Option<&'a str>),
    /// A log printed the way `git log` prints commits.
    Log(Log, Format),
    Version,
    Help,
    Unknown(&'a str),
}

fn command<'a>(args: &[&'a str]) -> Command<'a> {
    match args {
        [] => Command::Run,
        ["--redirect", url] => Command::Redirect(url),
        ["status"] => Command::Status,
        ["commit"] => Command::Commit(None),
        ["commit", "-m", text] => Command::Commit(Some(text)),
        ["status" | "commit", arg, ..] => Command::Unknown(arg),
        ["log", rest @ ..] => log_command(rest),
        ["--version" | "-V"] => Command::Version,
        ["--help" | "-h"] => Command::Help,
        ["--version" | "-V" | "--help" | "-h", arg, ..] | [arg, ..] => Command::Unknown(arg),
    }
}

fn log_command<'a>(args: &[&'a str]) -> Command<'a> {
    let (mut log, mut format) = (Log::Following, Format::Medium);
    for &arg in args {
        match arg {
            "--oneline" => format = Format::Oneline,
            _ => match ttyl::log::LOGS.into_iter().find(|log| log.slug() == arg) {
                Some(named) => log = named,
                None => return Command::Unknown(arg),
            },
        }
    }
    Command::Log(log, format)
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match command(&args) {
        Command::Run => report(start()),
        Command::Redirect(url) => report(signin::forward(url).map_err(|error| error.to_string())),
        Command::Status => report(status()),
        Command::Commit(text) => report(commit(text)),
        Command::Log(log, format) => report(print_log(log, format)),
        Command::Version => {
            println!("ttyl {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Help => {
            println!(
                "{}\n\nUsage: ttyl [--version | --help]\n       ttyl status\n       ttyl commit [-m <text>]\n       ttyl log [following | latest | trending] [--oneline]\n\nCommands:\n  status  print how many notifications are unread\n  commit  send a commit, with its text from -m or from stdin\n  log     print a log the way git log prints commits, through $PAGER in a terminal\n\nEnvironment:\n  DEVNULL_URL  the server to use (default: {})",
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
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("not a terminal; run ttyl in one".to_owned());
    }
    let (base, key) = device()?;
    let cache = Cache::locate(&base, &key);
    let start = cache.as_ref().and_then(Cache::load);
    let mut client = Client::new(base, key);
    if let Some(cache) = cache {
        client = client.with_cache(cache);
    }
    let client = Arc::new(client);
    let mut terminal = ratatui::init();
    // Asked before input is read on another thread, which would take the terminal's answer.
    let picker = if tmux_drops_queries() {
        Picker::halfblocks()
    } else {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())
    };
    let _ = execute!(io::stdout(), EnableBracketedPaste, EnableFocusChange);
    let result = run(&mut terminal, client, picker, start);
    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableFocusChange);
    ratatui::restore();
    result.map_err(|error| error.to_string())
}

/// The server and the key this terminal signs in with.
fn device() -> Result<(String, DeviceKey), String> {
    let path = device::key_path().map_err(|error| error.to_string())?;
    let key = DeviceKey::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let base = env::var("DEVNULL_URL").unwrap_or_else(|_| api::BASE.to_owned());
    Ok((base, key))
}

fn client() -> Result<Client, String> {
    let (base, key) = device()?;
    Ok(Client::new(base, key))
}

fn refused(error: &api::Error) -> String {
    if error.needs_sign_in() {
        "not signed in; run ttyl to sign in".to_owned()
    } else if error.is("commit_rules_required") {
        compose::RULES.join(" ")
    } else {
        error.to_string()
    }
}

fn status() -> Result<(), String> {
    let me = client()?.me().map_err(|error| refused(&error))?;
    println!("{}", me.unread);
    Ok(())
}

fn print_log(log: Log, format: Format) -> Result<(), String> {
    let page = client()?.page(log, None).map_err(|error| refused(&error))?;
    let tty = io::stdout().is_terminal();
    let text = gitlog::print(&page.rows, format, &Local, tty);
    let shown = match tty {
        true => through_pager(&text),
        false => io::stdout().write_all(text.as_bytes()),
    };
    match shown {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => Err(error.to_string()),
        _ => Ok(()),
    }
}

/// Shows `text` through `$PAGER`, or `less` as git does, which quits at once when it fits the screen.
fn through_pager(text: &str) -> io::Result<()> {
    let pager = env::var("PAGER")
        .ok()
        .filter(|pager| !pager.trim().is_empty())
        .unwrap_or_else(|| "less".to_owned());
    let spawned = std::process::Command::new("sh")
        .arg("-c")
        .arg(&pager)
        .env("LESS", env::var_os("LESS").unwrap_or_else(|| "FRX".into()))
        .stdin(std::process::Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return io::stdout().write_all(text.as_bytes());
    };
    let written = (child.stdin.take())
        .expect("stdin is piped")
        .write_all(text.as_bytes());
    child.wait()?;
    written
}

fn commit(text: Option<&str>) -> Result<(), String> {
    let text = match text {
        Some(text) => text.to_owned(),
        None if io::stdin().is_terminal() => {
            return Err("nothing to commit; pass -m <text> or pipe the text in".to_owned());
        }
        None => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .map_err(|error| format!("can't read stdin: {error}"))?;
            text
        }
    };
    if let Some(refusal) = body::refusal(&text) {
        return Err(refusal);
    }
    let post = Call::Post {
        target: Target::New,
        body: text,
        attachment: Attachment::None,
    };
    let reply = client()?.call(&post).map_err(|error| {
        if error.may_have_landed() {
            engine::NO_ANSWER.to_owned()
        } else {
            refused(&error)
        }
    })?;
    let Reply::Commit(focus) = reply else {
        unreachable!("a post answers with its commit")
    };
    let commit = &focus.card.commit;
    match commit.status {
        Status::Pending => {
            println!("{}", engine::PENDING);
        }
        Status::Approved | Status::Rejected => {
            let first = commit.body.lines().next().unwrap_or("");
            println!("[log {}] {first}", commit.short_hash);
        }
    }
    println!("{}", commit.html_url);
    Ok(())
}

/// Whether ttyl runs in a tmux pane that drops passthrough, as tmux does unless told otherwise. The
/// image query would get no answer there, so it would wait out its timeout with a blank screen and
/// leave a thread reading stdin that takes the first key.
fn tmux_drops_queries() -> bool {
    let Some(pane) = env::var_os("TMUX").and(env::var_os("TMUX_PANE")) else {
        return false;
    };
    std::process::Command::new("tmux")
        .arg("display")
        .arg("-p")
        .arg("-t")
        .arg(pane)
        .arg("#{allow-passthrough}")
        .output()
        .is_ok_and(|out| out.status.success() && out.stdout.trim_ascii() == b"off")
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

fn run(
    terminal: &mut DefaultTerminal,
    client: Arc<Client>,
    picker: Picker,
    start: Option<Start>,
) -> io::Result<()> {
    let (events, inbox) = mpsc::channel();
    let jobs = spawn_workers(Arc::clone(&client), events.clone());
    let input = spawn_input(events.clone());
    let device_name = welcome::device_name();
    if env::var_os("TTYL_ICONS").is_none()
        && let Some(saved) =
            draft::state().and_then(|state| fs::read_to_string(state.join(ICONS)).ok())
    {
        theme::set_plain_icons(saved.trim() == "plain");
    }
    let drafts = draft::dir();
    let saved = drafts.as_deref().map(draft::load).unwrap_or_default();
    if let Some(dir) = drafts.as_deref() {
        prune(dir, &saved);
    }
    let font = picker.font_size();
    let now = Instant::now();
    let mut engine = Engine::new(device_name.clone(), now)
        .with_drafts(Drafts::new(saved))
        .with_picker(picker);
    if let Some(start) = start {
        engine = engine.with_start(start, now);
    }
    let mut runtime = Runtime {
        font,
        engine,
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
                    let edited =
                        self.hand_over(terminal, || editor::edit(&editor::chosen(), &text));
                    let event = match edited.flatten() {
                        Some(edited) => Event::Edited(edited),
                        None => Event::EditorFailed,
                    };
                    let effects = self.engine.update(event, Instant::now());
                    self.perform(terminal, effects)?;
                }
                Effect::Pipe { command, input } => {
                    let outcome = self
                        .hand_over(terminal, || pipe(&command, &input))
                        .unwrap_or_else(|| Err("can't hand it the terminal".to_owned()));
                    let effects = self
                        .engine
                        .update(Event::Piped(command, outcome), Instant::now());
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
                Effect::Icons(plain) => {
                    if let Some(state) = draft::state() {
                        let _ = fs::create_dir_all(&state).and_then(|()| {
                            fs::write(state.join(ICONS), if plain { "plain" } else { "nerd" })
                        });
                    }
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
            Clipped::Unreachable if cfg!(target_os = "macos") => {
                Event::Unclipped("can't reach the clipboard.".to_owned())
            }
            Clipped::Unreachable => Event::Unclipped(
                "can't reach the clipboard. install wl-clipboard or xclip.".to_owned(),
            ),
        }
    }

    /// Gives the terminal to `run`, as a shell would to a program, and takes it back after.
    fn hand_over<T>(
        &mut self,
        terminal: &mut DefaultTerminal,
        run: impl FnOnce() -> T,
    ) -> Option<T> {
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
        let ran = run();
        let _ = terminal::enable_raw_mode();
        let _ = execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        );
        let _ = terminal.clear();
        drop(resume);
        Some(ran)
    }
}

/// Runs `command` through `sh` with `input` on its stdin. When it printed something, ttyl waits
/// for a key so it can be read; a command that printed nothing, such as `wl-copy`, returns at once.
fn pipe(command: &str, input: &str) -> Result<(), String> {
    let before = cursor::position().ok();
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("can't run sh: {error}"))?;
    let written = (child.stdin.take())
        .expect("stdin is piped")
        .write_all(input.as_bytes());
    let status = child.wait().map_err(|error| error.to_string())?;
    let outcome = match (status.code(), written) {
        (Some(0), Ok(())) => Ok(()),
        (Some(0), Err(error)) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        (Some(0), Err(error)) => Err(format!("can't write to it: {error}")),
        (Some(code), _) => Err(format!("exited with {code}")),
        (None, _) => Err("killed by a signal".to_owned()),
    };
    if cursor::position().ok() != before {
        let said = match &outcome {
            Ok(()) => "done".to_owned(),
            Err(why) => why.clone(),
        };
        print!("\r\n[{command}: {said}] press any key to go back to ttyl");
        let _ = io::stdout().flush();
        let _ = terminal::enable_raw_mode();
        loop {
            match input::read() {
                Ok(Input::Key(key)) if key.kind == KeyEventKind::Press => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        print!("\r\n");
    }
    outcome
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
