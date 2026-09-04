use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use boomux::client::{self, Client};
use boomux::protocol::{
    AgentState, AttachFrame, ShellSnapshot, ShellSpec, ShellStatus, TerminalProfile,
};
use gpui::Keystroke;
use libghostty_vt::kitty::graphics::{ImageFormat, PlacementIterator};
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{Palette, PaletteIndex, RgbColor, Underline};
use libghostty_vt::terminal::{Mode, ScrollViewport};
use libghostty_vt::{RenderState, Terminal as GhosttyTerminal, TerminalOptions};

use crate::generated_names;

const SCROLLBACK_ROWS: usize = 2_000;
const RECONNECT_ATTEMPTS: usize = 80;
const RECONNECT_DELAY: Duration = Duration::from_millis(25);
const RESIZE_SETTLE: Duration = Duration::from_millis(100);
const KITTY_IMAGE_STORAGE_BYTES: u64 = 64 * 1024 * 1024;
const EMULATOR_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellChoice {
    pub id: String,
    pub name: String,
    pub workspace_id: String,
    pub cwd: PathBuf,
    pub status: ShellStatus,
    pub run_id: Option<String>,
}

impl ShellChoice {
    pub fn status_label(&self) -> &'static str {
        match self.status {
            ShellStatus::Pending => "pending",
            ShellStatus::Running => "running",
            ShellStatus::Exited { .. } => "exited",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceChoice {
    pub id: String,
    pub name: String,
    pub shells: Vec<ShellChoice>,
    pub agent_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChoice {
    pub id: String,
    pub shell_name: String,
    pub workspace: String,
    pub shell_id: String,
    pub integration: String,
    pub state: AgentState,
    pub updated_at_ms: u64,
    pub needs_attention: bool,
}

impl AgentChoice {
    pub fn state_label(&self) -> &'static str {
        match self.state {
            AgentState::Unknown => "unknown",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Idle => "idle",
            AgentState::Inactive => "inactive",
            AgentState::Done => "finished",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoomuxOverview {
    pub workspaces: Vec<WorkspaceChoice>,
    pub agents: Vec<AgentChoice>,
    pub focused_shell_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCell {
    pub text: String,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub wide: bool,
    pub continuation: bool,
    pub cursor: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalScreen {
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<TerminalCell>,
    pub scroll_total: u64,
    pub scroll_offset: u64,
    pub scroll_len: u64,
    pub images: Vec<TerminalImage>,
    pub image_placements: Vec<TerminalImagePlacement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalImage {
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    /// GPUI's image atlas consumes BGRA8 pixels.
    pub bgra: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalImagePlacement {
    pub image_generation: u64,
    pub viewport_col: i32,
    pub viewport_row: i32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub z: i32,
}

struct SharedTerminal {
    screen: Mutex<TerminalScreen>,
    emulator: Mutex<Option<mpsc::SyncSender<EmulatorCommand>>>,
    writer: Mutex<Option<std::os::unix::net::UnixStream>>,
    profile: Mutex<TerminalProfile>,
    status: Mutex<String>,
    revision: AtomicU64,
    application_cursor: AtomicBool,
    bracketed_paste: AtomicBool,
    pending_scroll_row: AtomicU64,
    pending_scroll_wakeup: AtomicBool,
    closed: AtomicBool,
}

impl SharedTerminal {
    fn new(profile: TerminalProfile) -> Self {
        Self {
            screen: Mutex::new(blank_screen(profile.rows, profile.cols)),
            emulator: Mutex::new(None),
            writer: Mutex::new(None),
            profile: Mutex::new(profile),
            status: Mutex::new("connecting".into()),
            revision: AtomicU64::new(1),
            application_cursor: AtomicBool::new(false),
            bracketed_paste: AtomicBool::new(false),
            pending_scroll_row: AtomicU64::new(0),
            pending_scroll_wakeup: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn install_emulator(&self, sender: mpsc::SyncSender<EmulatorCommand>) {
        *self.emulator.lock().unwrap() = Some(sender);
    }

    fn emulator_command(&self, command: EmulatorCommand) -> Result<(), String> {
        self.emulator
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(|| "Ghostty terminal core is not running".to_string())?
            .send(command)
            .map_err(|_| "Ghostty terminal core stopped".to_string())
    }

    fn scroll_to_row(&self, row: usize) -> Result<(), String> {
        self.pending_scroll_row.store(row as u64, Ordering::Release);
        if self.pending_scroll_wakeup.swap(true, Ordering::AcqRel) {
            return Ok(());
        }

        let result = {
            let emulator = self.emulator.lock().unwrap();
            let Some(sender) = emulator.as_ref() else {
                self.pending_scroll_wakeup.store(false, Ordering::Release);
                return Err("Ghostty terminal core is not running".to_string());
            };
            sender.try_send(EmulatorCommand::ScrollLatest)
        };
        match result {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.pending_scroll_wakeup.store(false, Ordering::Release);
                Err("Ghostty terminal core stopped".to_string())
            }
        }
    }

    fn process(&self, bytes: Vec<u8>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        if let Err(error) = self.emulator_command(EmulatorCommand::Output(bytes)) {
            self.close(error);
        }
    }

    fn resize_emulator(&self, rows: u16, cols: u16, pixel_width: u16, pixel_height: u16) {
        let cell_width = u32::from(pixel_width / cols.max(1)).max(1);
        let cell_height = u32::from(pixel_height / rows.max(1)).max(1);
        if let Err(error) = self.emulator_command(EmulatorCommand::Resize {
            rows,
            cols,
            cell_width,
            cell_height,
        }) {
            self.close(error);
        }
    }

    fn set_status(&self, status: impl Into<String>) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        self.replace_status(status);
    }

    fn replace_status(&self, status: impl Into<String>) {
        *self.status.lock().unwrap() = status.into();
        self.bump_revision();
    }

    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }

    fn install_writer(&self, stream: &std::os::unix::net::UnixStream) -> Result<(), String> {
        let writer = stream
            .try_clone()
            .map_err(|error| format!("could not clone Boomux attachment: {error}"))?;
        *self.writer.lock().unwrap() = Some(writer);
        Ok(())
    }

    fn send(&self, frame: AttachFrame) -> Result<(), String> {
        let mut writer = self.writer.lock().unwrap();
        let stream = writer
            .as_mut()
            .ok_or_else(|| "Boomux terminal is not attached".to_string())?;
        frame
            .write_to(stream)
            .map_err(|error| format!("Boomux terminal write failed: {error}"))
    }

    fn close(&self, status: impl Into<String>) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        *self.writer.lock().unwrap() = None;
        if let Some(emulator) = self.emulator.lock().unwrap().take() {
            let _ = emulator.send(EmulatorCommand::Stop);
        }
        self.replace_status(status);
    }
}

enum EmulatorCommand {
    Output(Vec<u8>),
    Resize {
        rows: u16,
        cols: u16,
        cell_width: u32,
        cell_height: u32,
    },
    Scroll(ScrollViewport),
    ScrollLatest,
    Stop,
}

pub struct TerminalSession {
    pub shell_id: String,
    pub shell_name: String,
    shared: Arc<SharedTerminal>,
    last_size: Mutex<(u16, u16)>,
}

impl TerminalSession {
    pub fn attach(
        shell: ShellChoice,
        rows: u16,
        cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<Self, String> {
        let client = client::connect_if_running()
            .map_err(|error| format!("could not connect to Boomux: {error}"))?
            .ok_or_else(|| "Boomux is not running".to_string())?;
        let profile = terminal_profile(rows, cols, pixel_width, pixel_height);
        let attachment = attach_shell(&client, &shell, profile.clone(), true)?;
        let shared = Arc::new(SharedTerminal::new(profile));
        let stream = attachment.stream;
        shared.install_writer(&stream)?;
        start_emulator(&shared, rows, cols, pixel_width, pixel_height)?;
        shared.process(attachment.reconstruction);

        let expected_run_id = client
            .get_shell(&shell.id)
            .ok()
            .and_then(|snapshot| snapshot.run.map(|run| run.id))
            .or(shell.run_id.clone());
        spawn_reader(
            client,
            shell.id.clone(),
            expected_run_id,
            stream,
            Arc::clone(&shared),
        );
        // Attachment warnings describe daemon-side environment history; they
        // are diagnostic metadata, not actionable terminal state. Keep pane
        // headings quiet after a successful attachment while still surfacing
        // later connection and emulator failures through `status_message`.
        shared.set_status("attached");

        Ok(Self {
            shell_id: shell.id,
            shell_name: shell.name,
            shared,
            // Force one normal resize from the first GPUI render pass. The
            // attachment profile already established this size; repeating it
            // here avoids blocking startup on the old two-step resize nudge.
            last_size: Mutex::new((0, 0)),
        })
    }

    pub fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
    }

    pub fn status_message(&self) -> Option<String> {
        let status = self.shared.status.lock().unwrap();
        match status.as_str() {
            "attached" | "connecting" => None,
            status => Some(
                status
                    .strip_prefix("attached · ")
                    .unwrap_or(status)
                    .to_string(),
            ),
        }
    }

    pub fn screen(&self) -> TerminalScreen {
        self.shared.screen.lock().unwrap().clone()
    }

    pub fn send_key(&self, keystroke: &Keystroke) -> bool {
        let application_cursor = self.shared.application_cursor.load(Ordering::Acquire);
        let Some(bytes) = encode_key(keystroke, application_cursor) else {
            return false;
        };
        // Typing follows conventional terminal behavior and returns the
        // viewport to the live prompt before the PTY produces more output.
        if let Err(error) = self
            .shared
            .emulator_command(EmulatorCommand::Scroll(ScrollViewport::Bottom))
        {
            self.shared.set_status(error);
        }
        if let Err(error) = self.shared.send(AttachFrame::Input(bytes)) {
            self.shared.set_status(error);
        }
        true
    }

    pub fn paste(&self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let bytes = encode_paste(text, self.shared.bracketed_paste.load(Ordering::Acquire));
        if let Err(error) = self.shared.send(AttachFrame::Input(bytes)) {
            self.shared.set_status(error);
        }
        true
    }

    pub fn scroll(&self, lines: isize) -> bool {
        if lines == 0 {
            return false;
        }
        if let Err(error) = self
            .shared
            .emulator_command(EmulatorCommand::Scroll(ScrollViewport::Delta(lines)))
        {
            self.shared.set_status(error);
        }
        true
    }

    pub fn scroll_to(&self, row: usize) {
        if let Err(error) = self.shared.scroll_to_row(row) {
            self.shared.set_status(error);
        }
    }

    pub fn scroll_to_top(&self) {
        if let Err(error) = self
            .shared
            .emulator_command(EmulatorCommand::Scroll(ScrollViewport::Top))
        {
            self.shared.set_status(error);
        }
    }

    pub fn scroll_to_bottom(&self) {
        if let Err(error) = self
            .shared
            .emulator_command(EmulatorCommand::Scroll(ScrollViewport::Bottom))
        {
            self.shared.set_status(error);
        }
    }

    pub fn resize(&self, rows: u16, cols: u16, pixel_width: u16, pixel_height: u16) -> bool {
        let mut last_size = self.last_size.lock().unwrap();
        if *last_size == (rows, cols) {
            return false;
        }
        *last_size = (rows, cols);
        {
            let mut profile = self.shared.profile.lock().unwrap();
            profile.rows = rows;
            profile.cols = cols;
            profile.pixel_width = pixel_width;
            profile.pixel_height = pixel_height;
        }
        self.shared
            .resize_emulator(rows, cols, pixel_width, pixel_height);
        if let Err(error) = self.shared.send(AttachFrame::Resize {
            rows,
            cols,
            pixel_width,
            pixel_height,
        }) {
            self.shared.set_status(error);
        } else {
            self.shared.bump_revision();
        }
        true
    }

    pub fn focus(&self) {
        if let Err(error) = self.shared.send(AttachFrame::FocusGained) {
            self.shared.set_status(error);
        }
    }
}

fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if !bracketed {
        return normalized.into_bytes();
    }
    let mut bytes = Vec::with_capacity(normalized.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(normalized.replace("\x1b[201~", "").as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.shared.send(AttachFrame::Detached);
        self.shared.closed.store(true, Ordering::Release);
        if let Some(emulator) = self.shared.emulator.lock().unwrap().take() {
            let _ = emulator.send(EmulatorCommand::Stop);
        }
    }
}

pub fn discover_overview() -> Result<BoomuxOverview, String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    let snapshot = client
        .snapshot()
        .map_err(|error| format!("could not read Boomux workspaces: {error}"))?;
    let focused_shell_id = snapshot
        .focused_terminal
        .as_ref()
        .map(|focused| focused.shell_id.clone());
    let mut workspaces = Vec::new();
    let mut agents = Vec::new();
    for workspace in &snapshot.workspaces {
        let shells = workspace
            .shells
            .iter()
            .cloned()
            .map(shell_choice)
            .collect::<Vec<_>>();
        let visible_agents = workspace.agents.iter().filter(|agent| {
            let attached_to_current_run = workspace.shells.iter().any(|shell| {
                shell.id == agent.shell_id
                    && shell.run.as_ref().is_some_and(|run| run.id == agent.run_id)
            });
            agent_is_visible(
                agent.observation.state,
                agent.attention.is_some(),
                attached_to_current_run,
            )
        });
        let agent_count = visible_agents.clone().count();
        agents.extend(visible_agents.map(|agent| {
            AgentChoice {
                id: agent.id.clone(),
                shell_name: workspace
                    .shells
                    .iter()
                    .find(|shell| shell.id == agent.shell_id)
                    .map(|shell| shell.name.clone())
                    .unwrap_or_else(|| agent.name.clone()),
                workspace: workspace.name.clone(),
                shell_id: agent.shell_id.clone(),
                integration: agent.integration.clone(),
                state: agent.observation.state,
                updated_at_ms: agent.observation.observed_at_ms,
                needs_attention: agent.attention.is_some(),
            }
        }));
        workspaces.push(WorkspaceChoice {
            id: workspace.id.clone(),
            name: workspace.name.clone(),
            shells,
            agent_count,
        });
    }
    agents.sort_by_key(|agent| std::cmp::Reverse(agent.updated_at_ms));
    Ok(BoomuxOverview {
        workspaces,
        agents,
        focused_shell_id,
    })
}

fn agent_is_visible(state: AgentState, has_attention: bool, attached_to_current_run: bool) -> bool {
    has_attention
        || attached_to_current_run && !matches!(state, AgentState::Inactive | AgentState::Done)
}

fn shell_choice(shell: ShellSnapshot) -> ShellChoice {
    ShellChoice {
        id: shell.id,
        name: shell.name,
        workspace_id: shell.workspace_id,
        cwd: shell.cwd,
        status: shell.status,
        run_id: shell.run.map(|run| run.id),
    }
}

/// Create a pending shell next to an existing shell. Boomux remains the owner
/// of the PTY; the caller can immediately attach the returned choice.
pub fn create_shell(anchor: &ShellChoice) -> Result<ShellChoice, String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    let workspace = client
        .get_workspace(&anchor.workspace_id)
        .map_err(|error| format!("could not read Boomux workspace: {error}"))?;
    let name =
        generated_names::random_excluding(workspace.shells.iter().map(|shell| shell.name.as_str()))
            .ok_or_else(|| "Boomux shell names are exhausted".to_string())?;
    let shell = client
        .create_shell(
            &workspace.id,
            ShellSpec::login(
                name,
                workspace.default_cwd.unwrap_or_else(|| anchor.cwd.clone()),
            ),
        )
        .map_err(|error| format!("could not create Boomux shell: {error}"))?;
    Ok(shell_choice(shell))
}

pub fn create_shell_in_workspace(workspace_id: &str) -> Result<ShellChoice, String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    let workspace = client
        .get_workspace(workspace_id)
        .map_err(|error| format!("could not read Boomux workspace: {error}"))?;
    let name =
        generated_names::random_excluding(workspace.shells.iter().map(|shell| shell.name.as_str()))
            .ok_or_else(|| "Boomux shell names are exhausted".to_string())?;
    let cwd = workspace
        .default_cwd
        .or_else(|| workspace.shells.first().map(|shell| shell.cwd.clone()))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "could not determine a working directory for the new shell".to_string())?;
    client
        .create_shell(&workspace.id, ShellSpec::login(name, cwd))
        .map(shell_choice)
        .map_err(|error| format!("could not create Boomux shell: {error}"))
}

/// Create a local Workspace with its first pending login Shell. Boomux owns
/// both resources; the returned Shell can be attached immediately.
pub fn create_workspace_with_shell() -> Result<ShellChoice, String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    let snapshot = client
        .snapshot()
        .map_err(|error| format!("could not read Boomux workspaces: {error}"))?;
    let workspace_name = generated_names::random_excluding(
        snapshot
            .workspaces
            .iter()
            .map(|workspace| workspace.name.as_str()),
    )
    .ok_or_else(|| "Boomux workspace names are exhausted".to_string())?;
    let shell_name = generated_names::random_excluding(std::iter::empty())
        .ok_or_else(|| "Boomux shell names are exhausted".to_string())?;
    let cwd = std::env::current_dir()
        .map_err(|error| format!("could not determine the new workspace directory: {error}"))?;
    let workspace = client
        .create_workspace_with_default_cwd(
            workspace_name,
            Some(cwd.clone()),
            vec![ShellSpec::login(shell_name, cwd)],
        )
        .map_err(|error| format!("could not create Boomux workspace: {error}"))?;
    workspace
        .shells
        .into_iter()
        .next()
        .map(shell_choice)
        .ok_or_else(|| "Boomux created the workspace without its initial shell".to_string())
}

pub fn rename_workspace(workspace_id: &str, name: &str) -> Result<(), String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    client
        .rename_workspace(workspace_id, name)
        .map_err(|error| format!("could not rename Boomux workspace: {error}"))
}

pub fn rename_shell(shell_id: &str, name: &str) -> Result<(), String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    client
        .rename_shell(shell_id, name)
        .map_err(|error| format!("could not rename Boomux shell: {error}"))
}

pub fn remove_workspace(workspace_id: &str) -> Result<(), String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    client
        .close_workspace(workspace_id)
        .map_err(|error| format!("could not remove Boomux workspace: {error}"))
}

pub fn close_shell(shell_id: &str) -> Result<(), String> {
    let Some(client) = client::connect_if_running()
        .map_err(|error| format!("could not connect to Boomux: {error}"))?
    else {
        return Err("Boomux is not running".into());
    };
    client
        .close_shell(shell_id)
        .map_err(|error| format!("could not close Boomux shell: {error}"))
}

fn terminal_profile(rows: u16, cols: u16, pixel_width: u16, pixel_height: u16) -> TerminalProfile {
    TerminalProfile {
        term: Some("xterm-ghostty".into()),
        colorterm: Some("truecolor".into()),
        term_program: Some("boomux-desktop".into()),
        term_program_version: Some(env!("CARGO_PKG_VERSION").into()),
        rows,
        cols,
        pixel_width,
        pixel_height,
    }
}

fn attach_shell(
    client: &Client,
    shell: &ShellChoice,
    profile: TerminalProfile,
    takeover: bool,
) -> Result<client::Attachment, String> {
    let result = match (&shell.status, shell.run_id.as_deref()) {
        (ShellStatus::Running, Some(run_id)) => {
            client.attach_exact_run_with_client_environment(&shell.id, run_id, takeover, profile)
        }
        (ShellStatus::Pending, _) => {
            client.attach_with_client_environment(&shell.id, takeover, profile)
        }
        (ShellStatus::Exited { .. }, _) => {
            client.attach_restarting_with_client_environment(&shell.id, takeover, profile)
        }
        (ShellStatus::Running, None) => client.attach(&shell.id, takeover, profile),
    };
    result.map_err(|error| format!("could not attach {}: {error}", shell.name))
}

struct EmulatorCore {
    terminal: GhosttyTerminal<'static, 'static>,
    render_state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    graphics: PlacementIterator<'static>,
    cell_width: u32,
    cell_height: u32,
}

impl EmulatorCore {
    fn new(
        shared: &Arc<SharedTerminal>,
        rows: u16,
        cols: u16,
        pixel_width: u16,
        pixel_height: u16,
    ) -> Result<Self, String> {
        let mut terminal = GhosttyTerminal::new(TerminalOptions {
            // Start at a sentinel size so the first resize also records pixel
            // dimensions; libghostty treats an unchanged cell size as a no-op.
            cols: 1,
            rows: 1,
            max_scrollback: SCROLLBACK_ROWS,
        })
        .map_err(|error| format!("could not create Ghostty terminal: {error}"))?;
        configure_terminal(&mut terminal)?;
        terminal
            .set_kitty_image_storage_limit(KITTY_IMAGE_STORAGE_BYTES)
            .map_err(|error| format!("could not enable Ghostty Kitty graphics: {error}"))?;

        let weak = Arc::downgrade(shared);
        terminal
            .on_pty_write(move |_, bytes| {
                if let Some(shared) = weak.upgrade()
                    && let Err(error) = shared.send(AttachFrame::Input(bytes.to_vec()))
                {
                    shared.set_status(error);
                }
            })
            .map_err(|error| format!("could not configure Ghostty PTY replies: {error}"))?;
        let cell_width = cell_dimension(pixel_width, cols);
        let cell_height = cell_dimension(pixel_height, rows);
        terminal
            .resize(cols, rows, cell_width, cell_height)
            .map_err(|error| format!("could not size Ghostty terminal: {error}"))?;

        Ok(Self {
            terminal,
            render_state: RenderState::new()
                .map_err(|error| format!("could not create Ghostty render state: {error}"))?,
            rows: RowIterator::new()
                .map_err(|error| format!("could not create Ghostty row iterator: {error}"))?,
            cells: CellIterator::new()
                .map_err(|error| format!("could not create Ghostty cell iterator: {error}"))?,
            graphics: PlacementIterator::new()
                .map_err(|error| format!("could not create Ghostty graphics iterator: {error}"))?,
            cell_width,
            cell_height,
        })
    }

    fn apply(&mut self, command: EmulatorCommand) -> Result<bool, String> {
        match command {
            EmulatorCommand::Output(bytes) => self.terminal.vt_write(&bytes),
            EmulatorCommand::Resize {
                rows,
                cols,
                cell_width,
                cell_height,
            } => {
                self.terminal
                    .resize(cols, rows, cell_width, cell_height)
                    .map_err(|error| format!("Ghostty resize failed: {error}"))?;
                self.cell_width = cell_width;
                self.cell_height = cell_height;
            }
            EmulatorCommand::Scroll(viewport) => self.terminal.scroll_viewport(viewport),
            EmulatorCommand::ScrollLatest => {
                unreachable!("latest scroll requests are resolved by the emulator worker")
            }
            EmulatorCommand::Stop => return Ok(false),
        }
        Ok(true)
    }

    fn screen(&mut self) -> Result<TerminalScreen, String> {
        let (images, image_placements) = terminal_images(
            &self.terminal,
            &mut self.graphics,
            self.cell_width,
            self.cell_height,
        )?;
        let scrollbar = self
            .terminal
            .scrollbar()
            .map_err(|error| format!("could not read Ghostty scrollbar: {error}"))?;
        let snapshot = self
            .render_state
            .update(&self.terminal)
            .map_err(|error| format!("Ghostty render update failed: {error}"))?;
        let rows = snapshot
            .rows()
            .map_err(|error| format!("could not read Ghostty rows: {error}"))?;
        let cols = snapshot
            .cols()
            .map_err(|error| format!("could not read Ghostty columns: {error}"))?;
        let colors = snapshot
            .colors()
            .map_err(|error| format!("could not read Ghostty colors: {error}"))?;
        let cursor = if snapshot
            .cursor_visible()
            .map_err(|error| format!("could not read Ghostty cursor visibility: {error}"))?
        {
            snapshot
                .cursor_viewport()
                .map_err(|error| format!("could not read Ghostty cursor: {error}"))?
        } else {
            None
        };

        let mut output = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        let mut row_iter = self
            .rows
            .update(&snapshot)
            .map_err(|error| format!("could not iterate Ghostty rows: {error}"))?;
        let mut y = 0_u16;
        while let Some(row) = row_iter.next() {
            let mut cell_iter = self
                .cells
                .update(row)
                .map_err(|error| format!("could not iterate Ghostty cells: {error}"))?;
            let mut x = 0_u16;
            while let Some(cell) = cell_iter.next() {
                let style = cell
                    .style()
                    .map_err(|error| format!("could not read Ghostty cell style: {error}"))?;
                let mut text = String::new();
                cell.graphemes_utf8(&mut text)
                    .map_err(|error| format!("could not read Ghostty cell text: {error}"))?;
                if text.is_empty() || style.invisible {
                    text.push(' ');
                }
                let wide = cell
                    .raw_cell()
                    .and_then(|cell| cell.wide())
                    .map_err(|error| format!("could not read Ghostty cell width: {error}"))?;
                let mut foreground = cell
                    .fg_color()
                    .map_err(|error| format!("could not read Ghostty foreground: {error}"))?
                    .unwrap_or(colors.foreground);
                let mut background = cell
                    .bg_color()
                    .map_err(|error| format!("could not read Ghostty background: {error}"))?
                    .unwrap_or(colors.background);
                if style.inverse {
                    std::mem::swap(&mut foreground, &mut background);
                }
                output.push(TerminalCell {
                    text,
                    foreground: rgb_value(foreground),
                    background: rgb_value(background),
                    bold: style.bold,
                    italic: style.italic,
                    underline: style.underline != Underline::None,
                    wide: wide == CellWide::Wide,
                    continuation: matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead),
                    cursor: cursor.is_some_and(|cursor| cursor.x == x && cursor.y == y),
                });
                x = x.saturating_add(1);
            }
            y = y.saturating_add(1);
        }

        Ok(TerminalScreen {
            rows,
            cols,
            cells: output,
            scroll_total: scrollbar.total,
            scroll_offset: scrollbar.offset,
            scroll_len: scrollbar.len,
            images,
            image_placements,
        })
    }
}

fn apply_emulator_command(
    core: &mut EmulatorCore,
    shared: &SharedTerminal,
    command: EmulatorCommand,
) -> Result<bool, String> {
    match command {
        EmulatorCommand::ScrollLatest => {
            shared.pending_scroll_wakeup.store(false, Ordering::Release);
            let row = shared.pending_scroll_row.load(Ordering::Acquire) as usize;
            core.apply(EmulatorCommand::Scroll(ScrollViewport::Row(row)))
        }
        command => core.apply(command),
    }
}

fn terminal_images(
    terminal: &GhosttyTerminal<'_, '_>,
    iterator: &mut PlacementIterator<'_>,
    cell_width: u32,
    cell_height: u32,
) -> Result<(Vec<TerminalImage>, Vec<TerminalImagePlacement>), String> {
    let graphics = terminal
        .kitty_graphics()
        .map_err(|error| format!("could not read Ghostty graphics: {error}"))?;
    let mut iteration = iterator
        .update(&graphics)
        .map_err(|error| format!("could not iterate Ghostty graphics: {error}"))?;
    let mut images = Vec::new();
    let mut image_placements = Vec::new();
    let mut copied_generations = HashSet::new();

    while let Some(placement) = iteration.next() {
        let image_id = placement
            .image_id()
            .map_err(|error| format!("could not read Ghostty image id: {error}"))?;
        let Some(image) = graphics.image(image_id) else {
            continue;
        };
        let info = placement
            .placement_render_info(&image, terminal)
            .map_err(|error| format!("could not read Ghostty image placement: {error}"))?;
        if !info.viewport_visible {
            continue;
        }
        let generation = image
            .generation()
            .map_err(|error| format!("could not read Ghostty image generation: {error}"))?;
        if copied_generations.insert(generation) {
            let width = image
                .width()
                .map_err(|error| format!("could not read Ghostty image width: {error}"))?;
            let height = image
                .height()
                .map_err(|error| format!("could not read Ghostty image height: {error}"))?;
            let format = image
                .format()
                .map_err(|error| format!("could not read Ghostty image format: {error}"))?;
            let data = image
                .data()
                .map_err(|error| format!("could not read Ghostty image pixels: {error}"))?;
            images.push(TerminalImage {
                generation,
                width,
                height,
                bgra: image_bgra(format, width, height, data)?.into(),
            });
        }
        image_placements.push(TerminalImagePlacement {
            image_generation: generation,
            viewport_col: info.viewport_col,
            viewport_row: info.viewport_row,
            x_offset: placement
                .x_offset()
                .map_err(|error| format!("could not read Ghostty image x offset: {error}"))?,
            y_offset: placement
                .y_offset()
                .map_err(|error| format!("could not read Ghostty image y offset: {error}"))?,
            pixel_width: info.pixel_width,
            pixel_height: info.pixel_height,
            source_x: info.source_x,
            source_y: info.source_y,
            source_width: info.source_width,
            source_height: info.source_height,
            cell_width,
            cell_height,
            z: placement
                .z()
                .map_err(|error| format!("could not read Ghostty image z-index: {error}"))?,
        });
    }

    Ok((images, image_placements))
}

fn image_bgra(
    format: ImageFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| "Ghostty image dimensions are too large".to_string())?;
    let channels = match format {
        ImageFormat::Rgb => 3,
        ImageFormat::Rgba => 4,
        ImageFormat::GrayAlpha => 2,
        ImageFormat::Gray => 1,
        ImageFormat::Png => return Err("Ghostty returned undecoded PNG image data".into()),
        _ => return Err("Ghostty returned an unsupported image format".into()),
    };
    if data.len() != pixel_count.saturating_mul(channels) {
        return Err(format!(
            "Ghostty image has {} bytes; expected {}",
            data.len(),
            pixel_count.saturating_mul(channels)
        ));
    }

    let mut bgra = Vec::with_capacity(pixel_count.saturating_mul(4));
    for pixel in data.chunks_exact(channels) {
        match format {
            ImageFormat::Rgb => bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]),
            ImageFormat::Rgba => bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]),
            ImageFormat::GrayAlpha => {
                bgra.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]])
            }
            ImageFormat::Gray => bgra.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            ImageFormat::Png => unreachable!(),
            _ => unreachable!(),
        }
    }
    Ok(bgra)
}

fn start_emulator(
    shared: &Arc<SharedTerminal>,
    rows: u16,
    cols: u16,
    pixel_width: u16,
    pixel_height: u16,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::sync_channel(EMULATOR_QUEUE_CAPACITY);
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let worker_shared = Arc::clone(shared);
    thread::Builder::new()
        .name("boomux-ghostty-terminal".into())
        .spawn(move || {
            let mut core =
                match EmulatorCore::new(&worker_shared, rows, cols, pixel_width, pixel_height) {
                    Ok(core) => core,
                    Err(error) => {
                        let _ = ready_sender.send(Err(error));
                        return;
                    }
                };
            if let Err(error) = publish_screen(&mut core, &worker_shared) {
                let _ = ready_sender.send(Err(error));
                return;
            }
            if ready_sender.send(Ok(())).is_err() {
                return;
            }

            while let Ok(command) = receiver.recv() {
                match apply_emulator_command(&mut core, &worker_shared, command) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(error) => {
                        worker_shared.close(error);
                        return;
                    }
                }

                let mut stopped = false;
                while let Ok(command) = receiver.try_recv() {
                    match apply_emulator_command(&mut core, &worker_shared, command) {
                        Ok(true) => {}
                        Ok(false) => {
                            stopped = true;
                            break;
                        }
                        Err(error) => {
                            worker_shared.close(error);
                            return;
                        }
                    }
                }
                if stopped {
                    return;
                }
                // A full queue could not accept the wake-up marker, but it did
                // keep the latest requested row. Apply that row once after the
                // lossless command queue has been drained.
                if worker_shared
                    .pending_scroll_wakeup
                    .swap(false, Ordering::AcqRel)
                {
                    let row = worker_shared.pending_scroll_row.load(Ordering::Acquire) as usize;
                    if let Err(error) =
                        core.apply(EmulatorCommand::Scroll(ScrollViewport::Row(row)))
                    {
                        worker_shared.close(error);
                        return;
                    }
                }
                if core.terminal.mode(Mode::SYNC_OUTPUT).unwrap_or(false) {
                    continue;
                }
                if let Err(error) = publish_screen(&mut core, &worker_shared) {
                    worker_shared.close(error);
                    return;
                }
            }
        })
        .map_err(|error| format!("could not start Ghostty terminal worker: {error}"))?;

    ready_receiver
        .recv()
        .map_err(|_| "Ghostty terminal worker stopped during startup".to_string())??;
    shared.install_emulator(sender);
    Ok(())
}

fn publish_screen(core: &mut EmulatorCore, shared: &SharedTerminal) -> Result<(), String> {
    let screen = core.screen()?;
    let application_cursor = core
        .terminal
        .mode(Mode::DECCKM)
        .map_err(|error| format!("could not read Ghostty cursor mode: {error}"))?;
    *shared.screen.lock().unwrap() = screen;
    shared
        .application_cursor
        .store(application_cursor, Ordering::Release);
    let bracketed_paste = core
        .terminal
        .mode(Mode::BRACKETED_PASTE)
        .map_err(|error| format!("could not read Ghostty bracketed paste mode: {error}"))?;
    shared
        .bracketed_paste
        .store(bracketed_paste, Ordering::Release);
    shared.bump_revision();
    Ok(())
}

fn configure_terminal(terminal: &mut GhosttyTerminal<'_, '_>) -> Result<(), String> {
    let mut palette = Palette::default();
    for index in 0..=u8::MAX {
        palette.set(PaletteIndex(index), rgb_color(indexed_color(index)));
    }
    terminal
        .set_default_fg_color(Some(rgb_color(DEFAULT_FOREGROUND)))
        .and_then(|terminal| terminal.set_default_bg_color(Some(rgb_color(DEFAULT_BACKGROUND))))
        .and_then(|terminal| terminal.set_default_cursor_color(Some(rgb_color(DEFAULT_FOREGROUND))))
        .and_then(|terminal| terminal.set_default_color_palette(Some(palette)))
        .map_err(|error| format!("could not configure Ghostty colors: {error}"))?;
    Ok(())
}

fn blank_screen(rows: u16, cols: u16) -> TerminalScreen {
    TerminalScreen {
        rows,
        cols,
        cells: vec![
            TerminalCell {
                text: " ".into(),
                foreground: DEFAULT_FOREGROUND,
                background: DEFAULT_BACKGROUND,
                bold: false,
                italic: false,
                underline: false,
                wide: false,
                continuation: false,
                cursor: false,
            };
            usize::from(rows) * usize::from(cols)
        ],
        scroll_total: u64::from(rows),
        scroll_offset: 0,
        scroll_len: u64::from(rows),
        images: Vec::new(),
        image_placements: Vec::new(),
    }
}

fn cell_dimension(pixels: u16, cells: u16) -> u32 {
    u32::from(pixels / cells.max(1)).max(1)
}

const DEFAULT_FOREGROUND: u32 = 0xcdd6f4;
const DEFAULT_BACKGROUND: u32 = 0x11111b;
const ANSI_COLORS: [u32; 16] = [
    0x1e1e2e, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4, 0x45475a,
    0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xffffff,
];

fn rgb_color(value: u32) -> RgbColor {
    RgbColor {
        r: ((value >> 16) & 0xff) as u8,
        g: ((value >> 8) & 0xff) as u8,
        b: (value & 0xff) as u8,
    }
}

fn rgb_value(color: RgbColor) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn spawn_reader(
    client: Client,
    shell_id: String,
    expected_run_id: Option<String>,
    mut stream: std::os::unix::net::UnixStream,
    shared: Arc<SharedTerminal>,
) {
    thread::Builder::new()
        .name("boomux-desktop-terminal".into())
        .spawn(move || {
            loop {
                match AttachFrame::read_from(&mut stream) {
                    Ok(AttachFrame::Output(bytes)) => shared.process(bytes),
                    Ok(AttachFrame::Resize {
                        rows,
                        cols,
                        pixel_width,
                        pixel_height,
                    }) => {
                        shared.resize_emulator(rows, cols, pixel_width, pixel_height);
                        shared.bump_revision();
                    }
                    Ok(AttachFrame::Reconnect) => {
                        let _ = AttachFrame::ReconnectAck.write_to(&mut stream);
                        shared.set_status("reconnecting");
                        let profile = shared.profile.lock().unwrap().clone();
                        match reconnect(&client, &shell_id, expected_run_id.as_deref(), &profile) {
                            Ok(attachment) => {
                                stream = attachment.stream;
                                if let Err(error) = resynchronize_terminal_size(
                                    &mut stream,
                                    profile.rows,
                                    profile.cols,
                                    profile.pixel_width,
                                    profile.pixel_height,
                                ) {
                                    shared
                                        .close(format!("could not restore terminal size: {error}"));
                                    return;
                                }
                                if let Err(error) = shared.install_writer(&stream) {
                                    shared.close(error);
                                    return;
                                }
                                shared.process(attachment.reconstruction);
                                shared.set_status("attached");
                            }
                            Err(error) => {
                                shared.close(error);
                                return;
                            }
                        }
                    }
                    Ok(AttachFrame::Detached) => {
                        shared.close("detached");
                        return;
                    }
                    Ok(_) => {
                        shared.close("Boomux sent an invalid terminal frame");
                        return;
                    }
                    Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
                        shared.close("connection closed");
                        return;
                    }
                    Err(error) => {
                        shared.close(format!("terminal read failed: {error}"));
                        return;
                    }
                }
            }
        })
        .expect("spawn Boomux terminal reader");
}

fn resynchronize_terminal_size(
    stream: &mut std::os::unix::net::UnixStream,
    rows: u16,
    cols: u16,
    pixel_width: u16,
    pixel_height: u16,
) -> Result<(), String> {
    let (nudged_rows, nudged_cols) = if rows > 1 {
        (rows - 1, cols)
    } else if cols > 1 {
        (rows, cols - 1)
    } else {
        return Ok(());
    };
    AttachFrame::Resize {
        rows: nudged_rows,
        cols: nudged_cols,
        pixel_width,
        pixel_height,
    }
    .write_to(stream)
    .map_err(|error| format!("could not nudge terminal size: {error}"))?;
    thread::sleep(RESIZE_SETTLE);
    AttachFrame::Resize {
        rows,
        cols,
        pixel_width,
        pixel_height,
    }
    .write_to(stream)
    .map_err(|error| format!("could not synchronize terminal size: {error}"))
}

fn reconnect(
    client: &Client,
    shell_id: &str,
    expected_run_id: Option<&str>,
    profile: &TerminalProfile,
) -> Result<client::Attachment, String> {
    let mut last_error = None;
    for _ in 0..RECONNECT_ATTEMPTS {
        let result = if let Some(run_id) = expected_run_id {
            client.attach_exact_run(shell_id, run_id, false, profile.clone())
        } else {
            client.attach(shell_id, false, profile.clone())
        };
        match result {
            Ok(attachment) => return Ok(attachment),
            Err(error) => last_error = Some(error),
        }
        thread::sleep(RECONNECT_DELAY);
    }
    Err(format!(
        "could not reconnect Boomux terminal: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".into())
    ))
}

fn encode_key(keystroke: &Keystroke, application_cursor: bool) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }

    let special = match keystroke.key.as_str() {
        "enter" => Some(b"\r".as_slice()),
        "backspace" => Some(b"\x7f".as_slice()),
        "tab" if modifiers.shift => Some(b"\x1b[Z".as_slice()),
        "tab" => Some(b"\t".as_slice()),
        "escape" => Some(b"\x1b".as_slice()),
        "up" if application_cursor => Some(b"\x1bOA".as_slice()),
        "down" if application_cursor => Some(b"\x1bOB".as_slice()),
        "right" if application_cursor => Some(b"\x1bOC".as_slice()),
        "left" if application_cursor => Some(b"\x1bOD".as_slice()),
        "up" => Some(b"\x1b[A".as_slice()),
        "down" => Some(b"\x1b[B".as_slice()),
        "right" => Some(b"\x1b[C".as_slice()),
        "left" => Some(b"\x1b[D".as_slice()),
        "home" => Some(b"\x1b[H".as_slice()),
        "end" => Some(b"\x1b[F".as_slice()),
        "delete" => Some(b"\x1b[3~".as_slice()),
        "pageup" => Some(b"\x1b[5~".as_slice()),
        "pagedown" => Some(b"\x1b[6~".as_slice()),
        _ => None,
    };

    let mut bytes = if let Some(special) = special {
        special.to_vec()
    } else if modifiers.control {
        let byte = keystroke.key.as_bytes().first().copied()?;
        if !byte.is_ascii() {
            return None;
        }
        vec![byte.to_ascii_uppercase() & 0x1f]
    } else {
        keystroke.key_char.as_ref()?.as_bytes().to_vec()
    };

    if modifiers.alt {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

fn indexed_color(index: u8) -> u32 {
    match index {
        0..=15 => ANSI_COLORS[usize::from(index)],
        16..=231 => {
            let value = index - 16;
            let component = |part: u8| if part == 0 { 0 } else { 55 + part * 40 };
            let red = component(value / 36);
            let green = component((value % 36) / 6);
            let blue = component(value % 6);
            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (u32::from(gray) << 16) | (u32::from(gray) << 8) | u32::from(gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;

    use boomux::protocol::{AgentState, AttachFrame};
    use gpui::{Keystroke, Modifiers};
    use libghostty_vt::terminal::Mode;

    use super::{
        EmulatorCommand, EmulatorCore, SharedTerminal, agent_is_visible, encode_key, encode_paste,
        image_bgra, indexed_color, resynchronize_terminal_size, terminal_profile,
    };
    use std::sync::Arc;

    fn key(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.into(),
            key_char: key_char.map(str::to_owned),
            modifiers,
        }
    }

    #[test]
    fn agent_visibility_requires_a_current_shell_run_or_attention() {
        assert!(agent_is_visible(AgentState::Idle, false, true));
        assert!(!agent_is_visible(AgentState::Idle, false, false));
        assert!(!agent_is_visible(AgentState::Inactive, false, true));
        assert!(!agent_is_visible(AgentState::Done, false, true));
        assert!(agent_is_visible(AgentState::Done, true, false));
    }

    #[test]
    fn absolute_scroll_requests_keep_only_the_latest_row() {
        let shared = SharedTerminal::new(terminal_profile(24, 80, 800, 480));
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        shared.install_emulator(sender);

        shared.scroll_to_row(10).unwrap();
        shared.scroll_to_row(40).unwrap();
        shared.scroll_to_row(75).unwrap();

        assert!(matches!(
            receiver.try_recv(),
            Ok(EmulatorCommand::ScrollLatest)
        ));
        assert!(receiver.try_recv().is_err());
        assert_eq!(shared.pending_scroll_row.load(Ordering::Acquire), 75);
        assert!(shared.pending_scroll_wakeup.load(Ordering::Acquire));
    }

    #[test]
    fn rejected_scroll_request_releases_its_wakeup_slot() {
        let shared = SharedTerminal::new(terminal_profile(24, 80, 800, 480));
        assert!(shared.scroll_to_row(10).is_err());
        assert!(!shared.pending_scroll_wakeup.load(Ordering::Acquire));
    }

    #[test]
    fn encodes_text_control_and_cursor_keys() {
        assert_eq!(
            encode_key(&key("a", Some("a"), Modifiers::default()), false),
            Some(b"a".to_vec())
        );
        assert_eq!(
            encode_key(
                &key(
                    "c",
                    None,
                    Modifiers {
                        control: true,
                        ..Default::default()
                    }
                ),
                false
            ),
            Some(vec![3])
        );
        assert_eq!(
            encode_key(&key("up", None, Modifiers::default()), true),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn paste_normalizes_newlines_and_honors_bracketed_mode() {
        assert_eq!(encode_paste("one\r\ntwo\r", false), b"one\ntwo\n");
        assert_eq!(
            encode_paste("one\r\ntwo", true),
            b"\x1b[200~one\ntwo\x1b[201~"
        );
    }

    #[test]
    fn maps_256_color_cube_and_grayscale() {
        assert_eq!(indexed_color(16), 0x000000);
        assert_eq!(indexed_color(231), 0xffffff);
        assert_eq!(indexed_color(232), 0x080808);
        assert_eq!(indexed_color(255), 0xeeeeee);
    }

    #[test]
    fn ghostty_reflows_wrapped_content_when_resized() {
        let shared = Arc::new(SharedTerminal::new(terminal_profile(2, 10, 100, 40)));
        let mut core = EmulatorCore::new(&shared, 2, 10, 100, 40).unwrap();
        core.apply(EmulatorCommand::Output(b"abcdefghijklmnop".to_vec()))
            .unwrap();
        core.apply(EmulatorCommand::Resize {
            rows: 4,
            cols: 5,
            cell_width: 10,
            cell_height: 20,
        })
        .unwrap();

        let screen = core.screen().unwrap();
        let lines = screen
            .cells
            .chunks(usize::from(screen.cols))
            .map(|row| {
                row.iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(&lines[..3], ["abcde", "fghij", "klmno"]);
        assert!(lines[3].starts_with('p'));
    }

    #[test]
    fn ghostty_scrolls_history_and_returns_to_bottom() {
        let shared = Arc::new(SharedTerminal::new(terminal_profile(3, 10, 100, 60)));
        let mut core = EmulatorCore::new(&shared, 3, 10, 100, 60).unwrap();
        core.apply(EmulatorCommand::Output(
            b"one\r\ntwo\r\nthree\r\nfour\r\nfive".to_vec(),
        ))
        .unwrap();
        let bottom = core.screen().unwrap();
        assert_eq!(
            bottom.scroll_offset,
            bottom.scroll_total.saturating_sub(bottom.scroll_len)
        );

        core.apply(EmulatorCommand::Scroll(
            libghostty_vt::terminal::ScrollViewport::Delta(-2),
        ))
        .unwrap();
        let history = core.screen().unwrap();
        let history_text = history
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        assert!(history_text.contains("one"), "{history_text:?}");
        assert_ne!(history, bottom);
        assert!(history.scroll_offset < bottom.scroll_offset);

        core.apply(EmulatorCommand::Scroll(
            libghostty_vt::terminal::ScrollViewport::Bottom,
        ))
        .unwrap();
        assert_eq!(core.screen().unwrap(), bottom);
    }

    #[test]
    fn ghostty_decodes_and_places_kitty_rgb_images() {
        let shared = Arc::new(SharedTerminal::new(terminal_profile(3, 10, 100, 60)));
        let mut core = EmulatorCore::new(&shared, 3, 10, 100, 60).unwrap();
        core.apply(EmulatorCommand::Output(
            b"\x1b_Gf=24,s=2,v=1,i=7;/wAAAP8A\x1b\\\x1b_Ga=p,i=7,c=2,r=1,C=1\x1b\\".to_vec(),
        ))
        .unwrap();

        let screen = core.screen().unwrap();
        assert_eq!(screen.images.len(), 1);
        assert_eq!(screen.images[0].width, 2);
        assert_eq!(screen.images[0].height, 1);
        assert_eq!(
            screen.images[0].bgra.as_ref(),
            &[0, 0, 255, 255, 0, 255, 0, 255]
        );
        assert_eq!(screen.image_placements.len(), 1);
        assert_eq!(screen.image_placements[0].pixel_width, 20);
        assert_eq!(screen.image_placements[0].pixel_height, 20);
    }

    #[test]
    fn ghostty_advertises_kitty_graphics_to_terminal_apps() {
        let (client, mut daemon) = UnixStream::pair().unwrap();
        daemon
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let shared = Arc::new(SharedTerminal::new(terminal_profile(3, 10, 100, 60)));
        shared.install_writer(&client).unwrap();
        let mut core = EmulatorCore::new(&shared, 3, 10, 100, 60).unwrap();
        core.apply(EmulatorCommand::Output(b"\x1b_Gi=1,a=q\x1b\\".to_vec()))
            .unwrap();

        let response = AttachFrame::read_from(&mut daemon).unwrap();
        let AttachFrame::Input(bytes) = response else {
            panic!("expected a Kitty graphics capability response");
        };
        assert_eq!(bytes, b"\x1b_Gi=1;OK\x1b\\");
    }

    #[test]
    fn ghostty_accepts_a_doom_sized_synchronized_frame() {
        const WIDTH: usize = 640;
        const HEIGHT: usize = 400;
        const ENCODED_BYTES: usize = WIDTH * HEIGHT * 4;

        let shared = Arc::new(SharedTerminal::new(terminal_profile(30, 80, 800, 600)));
        let mut core = EmulatorCore::new(&shared, 30, 80, 800, 600).unwrap();
        let mut output = b"\x1b[?2026h\x1b_Ga=d\x1b\\".to_vec();
        for offset in (0..ENCODED_BYTES).step_by(4096) {
            let first = offset == 0;
            let final_chunk = offset + 4096 == ENCODED_BYTES;
            if first {
                output.extend_from_slice(b"\x1b_Gf=24,s=640,v=400,i=1,m=1;");
            } else if final_chunk {
                output.extend_from_slice(b"\x1b_Gm=0;");
            } else {
                output.extend_from_slice(b"\x1b_Gm=1;");
            }
            output.extend(std::iter::repeat_n(b'A', 4096));
            output.extend_from_slice(b"\x1b\\");
        }
        output.extend_from_slice(b"\x1b_Ga=p,i=1,c=64,r=24,C=1\x1b\\\x1b[?2026l");

        core.apply(EmulatorCommand::Output(output)).unwrap();
        assert!(!core.terminal.mode(Mode::SYNC_OUTPUT).unwrap());
        let screen = core.screen().unwrap();
        assert_eq!(screen.images.len(), 1);
        assert_eq!(screen.images[0].bgra.len(), WIDTH * HEIGHT * 4);
        assert_eq!(screen.image_placements.len(), 1);
        assert_eq!(screen.image_placements[0].pixel_width, 640);
        assert_eq!(screen.image_placements[0].pixel_height, 480);
    }

    #[test]
    fn converts_supported_kitty_formats_to_bgra() {
        assert_eq!(
            image_bgra(
                libghostty_vt::kitty::graphics::ImageFormat::Rgb,
                1,
                1,
                &[1, 2, 3]
            )
            .unwrap(),
            [3, 2, 1, 255]
        );
        assert_eq!(
            image_bgra(
                libghostty_vt::kitty::graphics::ImageFormat::GrayAlpha,
                1,
                1,
                &[9, 10]
            )
            .unwrap(),
            [9, 9, 9, 10]
        );
    }

    #[test]
    fn attachment_resize_is_nudged_then_restored() {
        let (mut client, mut daemon) = UnixStream::pair().unwrap();
        let receiver = thread::spawn(move || {
            assert_eq!(
                AttachFrame::read_from(&mut daemon).unwrap(),
                AttachFrame::Resize {
                    rows: 21,
                    cols: 70,
                    pixel_width: 588,
                    pixel_height: 374,
                }
            );
            assert_eq!(
                AttachFrame::read_from(&mut daemon).unwrap(),
                AttachFrame::Resize {
                    rows: 22,
                    cols: 70,
                    pixel_width: 588,
                    pixel_height: 374,
                }
            );
        });

        resynchronize_terminal_size(&mut client, 22, 70, 588, 374).unwrap();
        receiver.join().unwrap();
    }
}
