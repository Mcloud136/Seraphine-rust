#![windows_subsystem = "windows"]

use std::io::{BufReader, prelude::*};
use std::process::{Child, Command as ProcessCommand};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use interprocess::local_socket::{GenericNamespaced, Stream, prelude::*};
use seraphine_winui3_spike::stress::{SimulatedLcuEvent, simulated_event_json};
use serde::{Deserialize, Serialize};
use windows_reactor::*;

const DEFAULT_ITEMS: usize = 1_000;
const DEFAULT_EVENT_RATE: u64 = 1_000;
const DEFAULT_UI_HZ: u64 = 50;
const DEFAULT_DURATION_SECS: u64 = 60;

#[derive(Debug, Serialize)]
struct SidecarCommand {
    duration_ms: u64,
    event_rate: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct SidecarResponse {
    mode: String,
    pid: u32,
    events: u64,
    parsed: u64,
    errors: u64,
    elapsed_ms: u128,
}

#[derive(Default)]
struct EventCounters {
    generated: AtomicU64,
    parsed: AtomicU64,
    parse_errors: AtomicU64,
}

#[derive(Clone, PartialEq)]
struct MatchItem {
    id: usize,
    title: String,
    detail: String,
}

#[derive(Clone, PartialEq)]
struct RowInput {
    item: Rc<MatchItem>,
    on_open: Callback<usize>,
}

#[derive(Clone)]
enum RowMessage {
    Open,
}

struct MatchRow;

impl Component for MatchRow {
    type Message = RowMessage;
    type Input = RowInput;

    fn create(_input: &Self::Input, _context: &ComponentContext<Self>) -> Self {
        Self
    }

    fn update(&mut self, message: Self::Message, context: &ComponentContext<Self>) {
        if matches!(message, RowMessage::Open) {
            context.input().on_open.call(context.input().item.id);
        }
    }

    fn view(&self, input: &Self::Input, context: &mut ViewContext<Self>) -> View {
        StackPanel::new()
            .spacing(2.0)
            .children((
                TextBlock::new()
                    .text(format!("#{}  {}", input.item.id, input.item.title))
                    .font_size(16.0),
                TextBlock::new().text(input.item.detail.clone()),
                Button::new()
                    .on_click(context.message(RowMessage::Open))
                    .content("Inspect"),
            ))
            .into()
    }
}

#[derive(Clone)]
enum Message {
    Start,
    StartSidecar,
    Stop,
    Tick,
    WorkerFinished,
    SidecarFinished(Result<SidecarResponse, String>),
    OpenRow(usize),
}

struct StressApp {
    items: Vec<Rc<MatchItem>>,
    source_revision: u64,
    counters: Arc<EventCounters>,
    worker: Option<ComponentTask>,
    sidecar_worker: Option<ComponentTask>,
    sidecar_process: Option<Child>,
    sidecar_running: bool,
    ui_timer: Option<ComponentTimer>,
    running: bool,
    started_at: Option<Instant>,
    duration: Duration,
    event_rate: u64,
    ui_hz: u64,
    generated: u64,
    parsed: u64,
    parse_errors: u64,
    consumed: u64,
    coalesced: u64,
    ui_ticks: u64,
    measured_rate: f64,
    last_rate_sample: Instant,
    last_rate_generated: u64,
    selected: Option<usize>,
    status: String,
}

impl StressApp {
    fn env_number(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default)
    }

    fn request_sidecar(
        pipe_name: String,
        duration_ms: u64,
        event_rate: u64,
    ) -> Result<SidecarResponse, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut connection = loop {
            match pipe_name.as_str().to_ns_name::<GenericNamespaced>() {
                Ok(name) => match Stream::connect(name) {
                    Ok(stream) => break BufReader::new(stream),
                    error => {
                        if Instant::now() >= deadline {
                            return Err(format!("sidecar connect failed: {error:?}"));
                        }
                    }
                },
                Err(error) => return Err(format!("invalid pipe name: {error}")),
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let command = SidecarCommand {
            duration_ms,
            event_rate,
        };
        let mut payload = serde_json::to_string(&command)
            .map_err(|error| format!("serialize command: {error}"))?;
        payload.push('\n');
        connection
            .get_mut()
            .write_all(payload.as_bytes())
            .map_err(|error| format!("write command: {error}"))?;
        connection
            .get_mut()
            .flush()
            .map_err(|error| format!("flush command: {error}"))?;

        let mut response = String::new();
        connection
            .read_line(&mut response)
            .map_err(|error| format!("read response: {error}"))?;
        serde_json::from_str(&response).map_err(|error| format!("parse response: {error}"))
    }

    fn start(&mut self, context: &ComponentContext<Self>) {
        if self.running {
            return;
        }

        self.running = true;
        self.started_at = Some(Instant::now());
        self.generated = 0;
        self.parsed = 0;
        self.parse_errors = 0;
        self.consumed = 0;
        self.coalesced = 0;
        self.ui_ticks = 0;
        self.measured_rate = 0.0;
        self.last_rate_sample = Instant::now();
        self.last_rate_generated = 0;
        self.counters.generated.store(0, Ordering::Relaxed);
        self.counters.parsed.store(0, Ordering::Relaxed);
        self.counters.parse_errors.store(0, Ordering::Relaxed);

        let counters = Arc::clone(&self.counters);
        let duration = self.duration;
        let event_rate = self.event_rate;
        self.worker = Some(context.spawn_background(move |token| {
            let started = Instant::now();
            let mut sequence = 0_u64;

            while started.elapsed() < duration && !token.is_cancelled() {
                let json = simulated_event_json(sequence);
                match serde_json::from_str::<SimulatedLcuEvent>(&json) {
                    Ok(event)
                        if event.sequence == sequence
                            && event.event_type
                                == "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase"
                            && !event.payload.is_empty() =>
                    {
                        counters.parsed.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        counters.parse_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
                counters.generated.fetch_add(1, Ordering::Relaxed);
                sequence += 1;

                let next = started + Duration::from_secs_f64(sequence as f64 / event_rate as f64);
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }

            Message::WorkerFinished
        }));

        self.schedule_tick(context);
        self.status = format!(
            "Running {} events/s for {} s; UI coalesces at {} Hz",
            self.event_rate,
            self.duration.as_secs(),
            self.ui_hz
        );
    }

    fn start_sidecar(&mut self, context: &ComponentContext<Self>) {
        if self.running || self.sidecar_running {
            return;
        }

        let pipe_name = format!("seraphine-stress-{}", std::process::id());
        let sidecar_path = std::env::current_exe()
            .map(|path| path.with_file_name("seraphine-winui3-sidecar.exe"))
            .map_err(|error| error.to_string());
        let sidecar_path = match sidecar_path {
            Ok(path) => path,
            Err(error) => {
                self.status = format!("Cannot locate sidecar: {error}");
                return;
            }
        };

        let child = ProcessCommand::new(sidecar_path)
            .arg(&pipe_name)
            .spawn()
            .map_err(|error| error.to_string());
        let child = match child {
            Ok(child) => child,
            Err(error) => {
                self.status = format!("Cannot start sidecar: {error}");
                return;
            }
        };

        self.sidecar_running = true;
        self.status = format!("Sidecar started: pid={}", child.id());
        self.sidecar_process = Some(child);
        let duration_ms = self.duration.as_millis() as u64;
        let event_rate = self.event_rate;
        self.sidecar_worker = Some(context.spawn_background(move |_| {
            Message::SidecarFinished(Self::request_sidecar(pipe_name, duration_ms, event_rate))
        }));
    }

    fn schedule_tick(&mut self, context: &ComponentContext<Self>) {
        let delay = Duration::from_secs_f64(1.0 / self.ui_hz as f64);
        self.ui_timer = Some(context.set_timeout(delay, Message::Tick));
    }

    fn stop(&mut self) {
        self.running = false;
        self.sidecar_running = false;
        if let Some(worker) = self.worker.take() {
            worker.cancel();
        }
        if let Some(worker) = self.sidecar_worker.take() {
            worker.cancel();
        }
        if let Some(mut process) = self.sidecar_process.take() {
            let _ = process.kill();
        }
        if let Some(timer) = self.ui_timer.take() {
            timer.cancel();
        }
        self.status = "Stopped".to_string();
    }
}

impl Component for StressApp {
    type Message = Message;
    type Input = ();

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let item_count = Self::env_number("SERAPHINE_SPIKE_ITEMS", DEFAULT_ITEMS as u64) as usize;
        let items = (0..item_count)
            .map(|id| {
                Rc::new(MatchItem {
                    id,
                    title: format!("Match {id:04}"),
                    detail: format!("Ranked Solo · Victory · KDA 8/2/9 · Queue {}", 420 + id % 5),
                })
            })
            .collect();
        let mut app = Self {
            items,
            source_revision: 0,
            counters: Arc::new(EventCounters::default()),
            worker: None,
            sidecar_worker: None,
            sidecar_process: None,
            sidecar_running: false,
            ui_timer: None,
            running: false,
            started_at: None,
            duration: Duration::from_secs(Self::env_number(
                "SERAPHINE_SPIKE_DURATION_SECS",
                DEFAULT_DURATION_SECS,
            )),
            event_rate: Self::env_number("SERAPHINE_SPIKE_EVENT_RATE", DEFAULT_EVENT_RATE),
            ui_hz: Self::env_number("SERAPHINE_SPIKE_UI_HZ", DEFAULT_UI_HZ),
            generated: 0,
            parsed: 0,
            parse_errors: 0,
            consumed: 0,
            coalesced: 0,
            ui_ticks: 0,
            measured_rate: 0.0,
            last_rate_sample: Instant::now(),
            last_rate_generated: 0,
            selected: None,
            status: "Ready".to_string(),
        };

        if std::env::var("SERAPHINE_SPIKE_AUTOSTART").as_deref() == Ok("1") {
            app.start(context);
        }
        if std::env::var("SERAPHINE_SPIKE_AUTOSTART_SIDECAR").as_deref() == Ok("1") {
            app.start_sidecar(context);
        }
        app
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Start => self.start(context),
            Message::StartSidecar => self.start_sidecar(context),
            Message::Stop => self.stop(),
            Message::Tick => {
                let generated = self.counters.generated.load(Ordering::Relaxed);
                let parsed = self.counters.parsed.load(Ordering::Relaxed);
                let parse_errors = self.counters.parse_errors.load(Ordering::Relaxed);
                let pending = generated.saturating_sub(self.consumed);
                self.coalesced += pending;
                self.consumed = generated;
                self.generated = generated;
                self.parsed = parsed;
                self.parse_errors = parse_errors;
                self.ui_ticks += 1;

                let sample_elapsed = self.last_rate_sample.elapsed();
                if sample_elapsed >= Duration::from_millis(500) {
                    let delta = generated.saturating_sub(self.last_rate_generated);
                    self.measured_rate = delta as f64 / sample_elapsed.as_secs_f64();
                    self.last_rate_sample = Instant::now();
                    self.last_rate_generated = generated;
                }

                let elapsed = self
                    .started_at
                    .map(|started| started.elapsed().as_secs())
                    .unwrap_or(0);
                self.status = format!(
                    "elapsed={elapsed}s events={} parsed={} errors={} rate={:.0}/s ui_ticks={} coalesced={} pending={}",
                    self.generated,
                    self.parsed,
                    self.parse_errors,
                    self.measured_rate,
                    self.ui_ticks,
                    self.coalesced,
                    generated.saturating_sub(self.consumed)
                );

                if self.running {
                    self.schedule_tick(context);
                }
            }
            Message::WorkerFinished => {
                self.running = false;
                self.worker = None;
                if let Some(timer) = self.ui_timer.take() {
                    timer.cancel();
                }
                self.generated = self.counters.generated.load(Ordering::Relaxed);
                self.parsed = self.counters.parsed.load(Ordering::Relaxed);
                self.parse_errors = self.counters.parse_errors.load(Ordering::Relaxed);
                self.status = format!(
                    "Finished: events={} parsed={} errors={} ui_ticks={} coalesced={}",
                    self.generated, self.parsed, self.parse_errors, self.ui_ticks, self.coalesced
                );
            }
            Message::SidecarFinished(result) => {
                self.sidecar_running = false;
                self.sidecar_worker = None;
                if let Some(mut process) = self.sidecar_process.take() {
                    let _ = process.try_wait();
                }
                match result {
                    Ok(response) => {
                        self.status = format!(
                            "Sidecar finished: pid={} mode={} events={} parsed={} errors={} elapsed={}ms",
                            response.pid,
                            response.mode,
                            response.events,
                            response.parsed,
                            response.errors,
                            response.elapsed_ms
                        );
                    }
                    Err(error) => self.status = format!("Sidecar failed: {error}"),
                }
            }
            Message::OpenRow(id) => {
                self.selected = Some(id);
                self.status = format!("Selected virtual row #{id}");
            }
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        context.window_title("Seraphine Stress Spike");
        let items = Rc::new(self.items.clone());
        let key_items = Rc::clone(&items);
        let on_open = context.callback(Message::OpenRow);
        let rows = VirtualSource::new(
            self.source_revision,
            items.len(),
            move |index| key_items[index].id,
            move |index| {
                View::component::<MatchRow>(RowInput {
                    item: Rc::clone(&items[index]),
                    on_open: on_open.clone(),
                })
            },
        );

        let selected = self
            .selected
            .map(|id| format!("Selected row: #{id}"))
            .unwrap_or_else(|| "Selected row: none".to_string());

        Grid::new()
            .rows([GridLength::Auto, GridLength::STAR, GridLength::Auto])
            .children((
                StackPanel::new().spacing(6.0).children((
                    TextBlock::new()
                        .text("Seraphine Rust/WinUI 3 stress spike")
                        .font_size(22.0)
                        .font_weight(FontWeight::BOLD),
                    TextBlock::new().text(format!(
                        "Virtual rows: {} · target events: {}/s · UI: {} Hz · duration: {} s",
                        self.items.len(),
                        self.event_rate,
                        self.ui_hz,
                        self.duration.as_secs()
                    )),
                    StackPanel::new().spacing(6.0).children((
                        Button::new()
                            .is_enabled(!self.running && !self.sidecar_running)
                            .on_click(context.message(Message::Start))
                            .content("Start stress"),
                        Button::new()
                            .is_enabled(!self.running && !self.sidecar_running)
                            .on_click(context.message(Message::StartSidecar))
                            .content("Start sidecar"),
                        Button::new()
                            .is_enabled(self.running || self.sidecar_running)
                            .on_click(context.message(Message::Stop))
                            .content("Stop"),
                    )),
                    TextBlock::new().text(self.status.clone()),
                    TextBlock::new().text(selected.clone()),
                )),
                ScrollViewer::new()
                    .grid_row(1)
                    .margin(Thickness::new(0.0, 8.0, 0.0, 0.0))
                    .content(ItemsRepeater::new().virtual_source(rows)),
                TextBlock::new().grid_row(2).text(selected),
            ))
            .into()
    }
}

fn main() {
    App::run_component::<StressApp>(()).unwrap();
}
