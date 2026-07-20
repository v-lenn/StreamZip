use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Terminal,
};
use std::collections::VecDeque;
use std::io::{stdout, Stdout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ProgressState {
    pub archive_name: String,
    pub mode: String,
    pub total_bytes: u64,
    pub extracted_bytes: AtomicU64,
    pub reclaimed_bytes: AtomicU64,
    pub file_count: AtomicU64,
    pub status: Mutex<String>,
    pub logs: Mutex<VecDeque<String>>,
    pub is_finished: AtomicBool,
    pub no_log: bool,
    pub start_time: Instant,
}

impl ProgressState {
    pub fn new(archive_name: String, mode: String, total_bytes: u64, no_log: bool) -> Self {
        Self {
            archive_name,
            mode,
            total_bytes,
            extracted_bytes: AtomicU64::new(0),
            reclaimed_bytes: AtomicU64::new(0),
            file_count: AtomicU64::new(0),
            status: Mutex::new("Initializing...".to_string()),
            logs: Mutex::new(VecDeque::new()),
            is_finished: AtomicBool::new(false),
            no_log,
            start_time: Instant::now(),
        }
    }

    pub fn set_status(&self, s: &str) {
        if let Ok(mut lock) = self.status.lock() {
            *lock = s.to_string();
        }
    }

    pub fn add_log(&self, msg: &str) {
        if self.no_log {
            return;
        }
        if let Ok(mut lock) = self.logs.lock() {
            lock.push_back(msg.to_string());
            if lock.len() > 50 {
                lock.pop_front();
            }
        }
    }

    pub fn inc_bytes(&self, delta: u64) {
        self.extracted_bytes.fetch_add(delta, Ordering::Relaxed);
    }

    pub fn add_reclaimed(&self, bytes: u64) {
        self.reclaimed_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn inc_file_count(&self) {
        self.file_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct AppUi {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl AppUi {
    pub fn new() -> std::io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn handle_events(&self) -> bool {
        if event::poll(Duration::from_millis(10)).unwrap_or(false) {
            if let Ok(Event::Key(key)) = event::read() {
                if key.code == KeyCode::Char('q')
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return true;
                }
            }
        }
        false
    }

    pub fn render(&mut self, state: &ProgressState) -> std::io::Result<()> {
        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(6),
                    Constraint::Length(1),
                ])
                .split(f.size());

            let pkg_ver = env!("CARGO_PKG_VERSION");
            let header = Paragraph::new(format!(
                " ⚡ StreamZip v{}  │  Archive: {}  │  Mode: {}",
                pkg_ver, state.archive_name, state.mode
            ))
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Archive Info "),
            );
            f.render_widget(header, chunks[0]);

            let ext_b = state.extracted_bytes.load(Ordering::Relaxed);
            let pct = if state.total_bytes > 0 {
                ((ext_b as f64 / state.total_bytes as f64) * 100.0).min(100.0) as u16
            } else {
                0
            };

            let elapsed_sec = state.start_time.elapsed().as_secs_f64();
            let speed_mbs = if elapsed_sec > 0.1 {
                (ext_b as f64 / 1_048_576.0) / elapsed_sec
            } else {
                0.0
            };

            let remaining_bytes = state.total_bytes.saturating_sub(ext_b);
            let eta_str = if speed_mbs > 0.0 && remaining_bytes > 0 {
                let eta_sec = (remaining_bytes as f64 / 1_048_576.0) / speed_mbs;
                format!("{:02}:{:02}", (eta_sec / 60.0) as u64, (eta_sec % 60.0) as u64)
            } else {
                "--:--".to_string()
            };

            let gauge = Gauge::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Extraction Progress (ETA: {}) ", eta_str)),
                )
                .gauge_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .percent(pct)
                .label(format!(
                    " {}% ({:.2} MB / {:.2} MB @ {:.1} MB/s)",
                    pct,
                    ext_b as f64 / 1_048_576.0,
                    state.total_bytes as f64 / 1_048_576.0,
                    speed_mbs
                ));
            f.render_widget(gauge, chunks[1]);

            let reclaimed_mb = state.reclaimed_bytes.load(Ordering::Relaxed) as f64 / 1_048_576.0;
            let count = state.file_count.load(Ordering::Relaxed);
            let status_text = state.status.lock().map(|s| s.clone()).unwrap_or_default();

            let stats_txt = format!(
                " Status: {:<16} │ Files: {:<6} │ Speed: {:.1} MB/s │ Reclaimed: {:.2} MB",
                status_text, count, speed_mbs, reclaimed_mb
            );
            let stats = Paragraph::new(stats_txt)
                .style(Style::default().fg(Color::Green))
                .block(Block::default().borders(Borders::ALL).title(" Live Stats "));
            f.render_widget(stats, chunks[2]);

            let logs_lock = state.logs.lock().map(|l| l.clone()).unwrap_or_default();
            let log_title = if state.no_log {
                " Live Extraction Logs (Disabled via --no-log) "
            } else {
                " Live Extraction Logs "
            };

            let items: Vec<ListItem> = if state.no_log {
                vec![ListItem::new(
                    "  • Logging disabled (--no-log active for maximum extraction speed)",
                )]
            } else {
                logs_lock
                    .iter()
                    .rev()
                    .take(12)
                    .map(|l| ListItem::new(format!("  • {}", l)))
                    .collect()
            };

            let list =
                List::new(items).block(Block::default().borders(Borders::ALL).title(log_title));
            f.render_widget(list, chunks[3]);

            let footer = Paragraph::new(" [Ctrl+C / Q] Safe Stop & Save Journal")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(footer, chunks[4]);
        })?;
        Ok(())
    }

    pub fn restore(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

impl Drop for AppUi {
    fn drop(&mut self) {
        self.restore();
    }
}
