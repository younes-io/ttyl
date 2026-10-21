use std::env;
use std::io;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use chrono::Local;
use devnull::api::{self, Call, Client};
use devnull::device::{self, DeviceKey};
use devnull::engine::{Effect, Engine, Event, WORKERS};
use devnull::sched::Ticket;
use devnull::{signin, welcome};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self as input, Event as Input};

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
    let result = run(&mut terminal, client);
    ratatui::restore();
    result.map_err(|error| error.to_string())
}

/// The threads around the engine: input, the workers, and the channel they all answer on.
struct Runtime {
    engine: Engine,
    client: Arc<Client>,
    events: Sender<Event>,
    jobs: Sender<(Ticket, Call)>,
    device_name: String,
}

fn run(terminal: &mut DefaultTerminal, client: Arc<Client>) -> io::Result<()> {
    let (events, inbox) = mpsc::channel();
    let jobs = spawn_workers(Arc::clone(&client), events.clone());
    spawn_input(events.clone());
    let device_name = welcome::device_name();
    let mut runtime = Runtime {
        engine: Engine::new(device_name.clone(), Instant::now()),
        client,
        events,
        jobs,
        device_name,
    };
    loop {
        let now = Instant::now();
        let effects = runtime.engine.pump(now);
        if runtime.perform(effects) {
            return Ok(());
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
            if runtime.perform(effects) {
                return Ok(());
            }
        }
    }
}

impl Runtime {
    /// Carries out effects, and says whether the engine asked to quit.
    fn perform(&mut self, effects: Vec<Effect>) -> bool {
        for effect in effects {
            match effect {
                Effect::Run(ticket, call) => {
                    let _ = self.jobs.send((ticket, call));
                }
                Effect::StartSignIn => {
                    let started =
                        welcome::start(&self.client, &self.device_name, self.events.clone());
                    let effects = self.engine.update(Event::SignIn(started), Instant::now());
                    if self.perform(effects) {
                        return true;
                    }
                }
                Effect::Quit => return true,
            }
        }
        false
    }
}

fn spawn_input(events: Sender<Event>) {
    thread::spawn(move || {
        loop {
            let event = match input::read() {
                Ok(Input::Key(key)) => Event::Key(key),
                Ok(Input::Resize(..)) => Event::Tick,
                Ok(_) => continue,
                Err(_) => return,
            };
            if events.send(event).is_err() {
                return;
            }
        }
    });
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
