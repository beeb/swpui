use std::{
    path::PathBuf,
    sync::{Arc, RwLock, atomic::AtomicBool, mpsc},
    thread,
    time::{Duration, Instant},
};

use rat_widget::{list::ListState, text_input::TextInputState};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyEventKind},
};

use crate::{
    preview::{PreviewCommand, PreviewResult, PreviewWorker, WantedSet},
    search::{FileMatches, SearchResult, SearchWorker, WorkerCommand},
    spinner::SpinnerState,
    types::{Options, Pane},
    ui::{self, preview::PreviewState},
};

pub mod apply;
pub mod input;
pub mod preview;
pub mod search;

const POLL_TIMEOUT: Duration = Duration::from_millis(16);

#[expect(clippy::struct_excessive_bools)]
pub struct App {
    pub root: PathBuf,
    pub options: Options,
    pub search_input: TextInputState,
    pub replace_input: TextInputState,
    pub file_list: ListState,
    pub preview: PreviewState,
    pub spinner: SpinnerState,
    pub focused_pane: Pane,
    pub status_message: Option<String>,
    pub searching: bool,
    pub truncated: bool,
    pub confirm_apply_all: bool,
    pub options_open: bool,
    pub pending_search: bool,
    pub exit: bool,
    pub generation: u64,
    pub results: Vec<FileMatches>,
    pub last_keystroke: Option<Instant>,
    pub cmd_tx: mpsc::Sender<WorkerCommand>,
    pub result_rx: mpsc::Receiver<SearchResult>,
    pub cancelled: Arc<AtomicBool>,
    pub preview_wanted: WantedSet,
    pub preview_cmd_tx: mpsc::Sender<PreviewCommand>,
    pub preview_result_rx: mpsc::Receiver<PreviewResult>,
    pub preview_generation: u64,
}

impl App {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));

        let options = Options::default();
        let worker = SearchWorker::new(root.clone(), cmd_rx, result_tx, Arc::clone(&cancelled))?;
        thread::spawn(move || worker.run(options.into()));

        let (preview_cmd_tx, preview_cmd_rx) = mpsc::channel();
        let (preview_result_tx, preview_result_rx) = mpsc::channel();
        let preview_wanted: WantedSet = Arc::new(RwLock::new([None, None, None]));
        let preview_worker = PreviewWorker::new(
            preview_cmd_rx,
            preview_result_tx,
            Arc::clone(&preview_wanted),
        );
        thread::spawn(move || preview_worker.run());

        Ok(Self {
            root,
            search_input: TextInputState::new(),
            replace_input: TextInputState::new(),
            options,
            results: Vec::new(),
            focused_pane: Pane::default(),
            file_list: ListState::default(),
            status_message: None,
            searching: false,
            truncated: false,
            spinner: SpinnerState::default(),
            confirm_apply_all: false,
            options_open: false,
            preview: PreviewState::new(),
            exit: false,
            generation: 0,
            last_keystroke: None,
            pending_search: false,
            cmd_tx,
            result_rx,
            cancelled,
            preview_wanted,
            preview_cmd_tx,
            preview_result_rx,
            preview_generation: 0,
        })
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.poll_events()?;
            self.poll_search_results();
            self.poll_preview_results();
            self.debounce_search();
            if self.searching {
                self.spinner.tick();
            }
        }
        Ok(())
    }

    pub fn selected_file(&self) -> usize {
        self.file_list.selected().unwrap_or_default()
    }

    pub fn clamp_selection(&mut self) {
        if self.results.is_empty() {
            self.file_list.select(Some(0));
            self.preview.reset_position();
            self.focused_pane = Pane::FileList;
        } else {
            let clamped = self.selected_file().min(self.results.len() - 1);
            self.file_list.select(Some(clamped));
            let match_count = self.results[clamped].matches.len();
            self.preview.clamp_match(match_count);
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        ui::render(self, frame);
    }

    fn poll_events(&mut self) -> anyhow::Result<()> {
        if event::poll(POLL_TIMEOUT)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            self.handle_key(key);
        }
        Ok(())
    }
}
