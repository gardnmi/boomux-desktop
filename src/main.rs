mod generated_names;
mod layout;
mod terminal;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{
    Animation, AnimationExt, App, Bounds, ClickEvent, ClipboardItem, Context, Corners, Div,
    DragMoveEvent, FocusHandle, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, RenderImage, ScrollAnchor, ScrollHandle, ScrollWheelEvent,
    SharedString, Stateful, TextRun, UnderlineStyle, Window, WindowBounds, WindowOptions, actions,
    canvas, div, ease_out_quint, fill, font, point, prelude::*, px, relative, rgb, rgb_to_hsla,
    rgba, size,
};
use layout::{Axis, Direction, Node, Rect};
use terminal::{
    AgentChoice, BoomuxOverview, ShellChoice, TerminalImagePlacement, TerminalScreen,
    TerminalSession,
};

const HEADER_HEIGHT: f32 = 42.0;
const FOOTER_HEIGHT: f32 = 30.0;
const PANEL_PADDING: f32 = 8.0;
const MIN_FLOAT_WIDTH: f32 = 220.0;
const MIN_FLOAT_HEIGHT: f32 = 160.0;
const TERMINAL_CELL_WIDTH: f32 = 8.4;
const TERMINAL_CELL_HEIGHT: f32 = 17.0;
const TERMINAL_PADDING: f32 = 16.0;
const SIDEBAR_WIDTH: f32 = 300.0;
const LAYOUT_ANIMATION_DURATION: Duration = Duration::from_millis(180);
const DRAG_ACTIVATION_DISTANCE: f32 = 4.0;

actions!(
    compositor,
    [
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        ResizeLeft,
        ResizeRight,
        ResizeUp,
        ResizeDown,
        NewPane,
        ClosePane,
        ToggleFloating,
        ToggleFullscreen,
        ToggleSidebarFocus,
        ToggleHelp,
        RenameResource,
        RemoveShell,
        CopySelection,
        PasteClipboard,
    ]
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NavigationRegion {
    #[default]
    Terminal,
    Sidebar,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SidebarItem {
    Workspace(String),
    Shell {
        workspace_id: String,
        shell_id: String,
    },
    Agent {
        agent_id: String,
        shell_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarResource {
    Workspace {
        id: String,
        name: String,
    },
    Shell {
        id: String,
        workspace_id: String,
        name: String,
    },
}

impl SidebarResource {
    fn name(&self) -> &str {
        match self {
            Self::Workspace { name, .. } | Self::Shell { name, .. } => name,
        }
    }

    fn kind_label(&self) -> &'static str {
        match self {
            Self::Workspace { .. } => "Workspace",
            Self::Shell { .. } => "Shell",
        }
    }
}

#[derive(Clone, Debug)]
struct SidebarMenu {
    target: SidebarResource,
    top: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceDialogKind {
    Rename,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutSection {
    General,
    Navigation,
    Panes,
    Terminal,
    Sidebar,
}

impl ShortcutSection {
    fn label(self) -> &'static str {
        match self {
            Self::General => "GENERAL",
            Self::Navigation => "NAVIGATION",
            Self::Panes => "PANES",
            Self::Terminal => "TERMINAL",
            Self::Sidebar => "SIDEBAR",
        }
    }
}

struct ShortcutSpec {
    section: ShortcutSection,
    keys: &'static str,
    description: &'static str,
}

const KEY_TOGGLE_HELP: &str = "f1";
const KEY_RENAME_RESOURCE: &str = "f2";
const KEY_TOGGLE_SIDEBAR: &str = "f6";
const KEY_NEW_PANE: &str = "secondary-enter";
const KEY_DETACH_PANE: &str = "secondary-w";
const KEY_REMOVE_SHELL: &str = "secondary-shift-w";
const KEY_TOGGLE_FLOATING: &str = "secondary-space";
const KEY_TOGGLE_FULLSCREEN: &str = "secondary-f";

const HELP_SHORTCUTS: &[ShortcutSpec] = &[
    ShortcutSpec {
        section: ShortcutSection::General,
        keys: "F1",
        description: "Toggle this help menu",
    },
    ShortcutSpec {
        section: ShortcutSection::General,
        keys: "Escape",
        description: "Close the current menu or dialog",
    },
    ShortcutSpec {
        section: ShortcutSection::Navigation,
        keys: "Ctrl + Arrow / H J K L",
        description: "Focus an adjacent pane or enter the sidebar",
    },
    ShortcutSpec {
        section: ShortcutSection::Navigation,
        keys: "F6",
        description: "Toggle focus between sidebar and terminal",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + Enter",
        description: "Create a Shell in the focused terminal's Workspace",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + W",
        description: "Detach the pane; preserve its Boomux Shell",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + Shift + W",
        description: "Permanently remove the selected Shell",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + Space",
        description: "Toggle tiled or floating",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + F",
        description: "Toggle fullscreen",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + Shift + Arrow / H J K L",
        description: "Move or swap the focused pane",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + Alt + H J K L",
        description: "Resize the focused pane",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + left drag",
        description: "Lift, move, and re-tile a pane",
    },
    ShortcutSpec {
        section: ShortcutSection::Panes,
        keys: "Ctrl + right drag",
        description: "Resize a pane from the pointer",
    },
    ShortcutSpec {
        section: ShortcutSection::Terminal,
        keys: "Ctrl + Shift + C / V",
        description: "Copy selection or paste clipboard",
    },
    ShortcutSpec {
        section: ShortcutSection::Terminal,
        keys: "Shift + Page Up / Down",
        description: "Scroll terminal history by one viewport",
    },
    ShortcutSpec {
        section: ShortcutSection::Terminal,
        keys: "Shift + Home / End",
        description: "Jump to the start or end of terminal history",
    },
    ShortcutSpec {
        section: ShortcutSection::Terminal,
        keys: "Left drag",
        description: "Select terminal text",
    },
    ShortcutSpec {
        section: ShortcutSection::Terminal,
        keys: "Middle click",
        description: "Paste the primary selection",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "Up / Down or J / K",
        description: "Move through visible rows",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "Left / Right or H / L",
        description: "Collapse or expand a Workspace",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "Enter",
        description: "Activate the selected row",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "Tab / Shift + Tab",
        description: "Move between Workspaces and Agents",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "Ctrl + Enter",
        description: "Create a Shell in the selected row's Workspace",
    },
    ShortcutSpec {
        section: ShortcutSection::Sidebar,
        keys: "F2",
        description: "Rename the selected Workspace or Shell",
    },
];

#[derive(Clone, Debug)]
struct ResourceDialog {
    kind: ResourceDialogKind,
    target: SidebarResource,
    value: String,
    busy: bool,
    error: Option<String>,
}

fn visible_sidebar_items(
    overview: &BoomuxOverview,
    expanded_workspaces: &HashSet<String>,
) -> Vec<SidebarItem> {
    let mut items = Vec::new();
    for workspace in &overview.workspaces {
        items.push(SidebarItem::Workspace(workspace.id.clone()));
        if expanded_workspaces.contains(&workspace.id) {
            items.extend(workspace.shells.iter().map(|shell| SidebarItem::Shell {
                workspace_id: workspace.id.clone(),
                shell_id: shell.id.clone(),
            }));
        }
    }
    items.extend(overview.agents.iter().map(|agent| SidebarItem::Agent {
        agent_id: agent.id.clone(),
        shell_id: agent.shell_id.clone(),
    }));
    items
}

fn reconciled_sidebar_item(
    current: Option<&SidebarItem>,
    preferred: Option<&SidebarItem>,
    visible: &[SidebarItem],
) -> Option<SidebarItem> {
    current
        .filter(|item| visible.contains(item))
        .or_else(|| preferred.filter(|item| visible.contains(item)))
        .or_else(|| visible.first())
        .cloned()
}

fn workspace_id_for_sidebar_item(item: &SidebarItem, overview: &BoomuxOverview) -> Option<String> {
    match item {
        SidebarItem::Workspace(workspace_id) => overview
            .workspaces
            .iter()
            .find(|workspace| workspace.id == *workspace_id)
            .map(|workspace| workspace.id.clone()),
        SidebarItem::Shell {
            workspace_id,
            shell_id,
        } => overview
            .workspaces
            .iter()
            .find(|workspace| {
                workspace.id == *workspace_id
                    && workspace.shells.iter().any(|shell| shell.id == *shell_id)
            })
            .map(|workspace| workspace.id.clone()),
        SidebarItem::Agent { agent_id, shell_id } => {
            let agent = overview
                .agents
                .iter()
                .find(|agent| agent.id == *agent_id && agent.shell_id == *shell_id)?;
            overview.workspaces.iter().find_map(|workspace| {
                workspace
                    .shells
                    .iter()
                    .any(|shell| shell.id == agent.shell_id)
                    .then(|| workspace.id.clone())
            })
        }
    }
}

#[derive(Clone, Debug)]
struct FloatingPane {
    id: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Copy, Debug)]
enum PointerOperation {
    Move,
    Resize,
}

#[derive(Clone, Debug)]
struct PointerDrag {
    operation: PointerOperation,
    button: MouseButton,
    start_pointer: (f32, f32),
    pane_id: usize,
    subject: PointerSubject,
    activated: bool,
}

#[derive(Clone, Debug)]
struct LayoutAnimation {
    from: HashMap<usize, Rect>,
    generation: u64,
}

#[derive(Clone, Debug)]
enum PointerSubject {
    Floating(FloatingPane),
    Lifted(FloatingPane),
    Tiled(Node),
}

#[derive(Clone)]
struct TerminalScrollbarDrag(usize);

impl Render for TerminalScrollbarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone)]
struct TerminalSelectionDrag {
    pane_id: usize,
    started: Arc<AtomicBool>,
}

impl Render for TerminalSelectionDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSelection {
    anchor: (usize, usize),
    head: (usize, usize),
}

fn dragged_bounds(
    mut bounds: FloatingPane,
    operation: PointerOperation,
    delta: (f32, f32),
    panel_size: (f32, f32),
) -> FloatingPane {
    match operation {
        PointerOperation::Move => {
            bounds.x = (bounds.x + delta.0).clamp(0.0, (panel_size.0 - bounds.width).max(0.0));
            bounds.y = (bounds.y + delta.1).clamp(0.0, (panel_size.1 - bounds.height).max(0.0));
        }
        PointerOperation::Resize => {
            let available_width = (panel_size.0 - bounds.x).max(0.0);
            let available_height = (panel_size.1 - bounds.y).max(0.0);
            bounds.width = (bounds.width + delta.0)
                .clamp(MIN_FLOAT_WIDTH.min(available_width), available_width);
            bounds.height = (bounds.height + delta.1)
                .clamp(MIN_FLOAT_HEIGHT.min(available_height), available_height);
        }
    }
    bounds
}

fn resized_tiled_layout(
    mut layout: Node,
    pane_id: usize,
    delta: (f32, f32),
    panel_size: (f32, f32),
) -> Node {
    layout.resize_from_pointer(pane_id, Axis::Horizontal, delta.0 / panel_size.0.max(1.0));
    layout.resize_from_pointer(pane_id, Axis::Vertical, delta.1 / panel_size.1.max(1.0));
    layout
}

fn interpolate_rect(from: Rect, to: Rect, progress: f32) -> Rect {
    let lerp = |start: f32, end: f32| start + (end - start) * progress;
    Rect {
        x: lerp(from.x, to.x),
        y: lerp(from.y, to.y),
        width: lerp(from.width, to.width),
        height: lerp(from.height, to.height),
    }
}

fn swap_layout_direction(
    layout: &mut Node,
    focused: usize,
    direction: Direction,
) -> Option<HashMap<usize, Rect>> {
    let neighbor = layout.neighbor(focused, direction)?;
    let previous_rects = layout.rects().into_iter().collect();
    layout.swap_panes(focused, neighbor);
    Some(previous_rects)
}

fn window_point_to_panel(x: f32, y: f32) -> (f32, f32) {
    (x - SIDEBAR_WIDTH, y - HEADER_HEIGHT)
}

fn terminal_cell_from_offset(x: f32, y: f32, screen: &TerminalScreen) -> (usize, usize) {
    let col = ((x - 8.0) / TERMINAL_CELL_WIDTH).floor().max(0.0) as usize;
    let row = ((y - 8.0) / TERMINAL_CELL_HEIGHT).floor().max(0.0) as usize;
    (
        row.min(usize::from(screen.rows).saturating_sub(1)),
        col.min(usize::from(screen.cols).saturating_sub(1)),
    )
}

fn selection_indices(selection: TerminalSelection, cols: usize) -> (usize, usize) {
    let anchor = selection.anchor.0 * cols + selection.anchor.1;
    let head = selection.head.0 * cols + selection.head.1;
    (anchor.min(head), anchor.max(head))
}

fn terminal_selected_text(screen: &TerminalScreen, selection: TerminalSelection) -> String {
    let cols = usize::from(screen.cols);
    let (start, end) = selection_indices(selection, cols);
    let first_row = start / cols;
    let last_row = end / cols;
    (first_row..=last_row)
        .map(|row| {
            let start_col = if row == first_row { start % cols } else { 0 };
            let end_col = if row == last_row {
                end % cols
            } else {
                cols.saturating_sub(1)
            };
            let mut line = String::new();
            for col in start_col..=end_col {
                let cell = &screen.cells[row * cols + col];
                if !cell.continuation {
                    line.push_str(&cell.text);
                }
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn drop_placement(layout: &Node, point: (f32, f32)) -> Option<(usize, Axis, bool)> {
    let (id, rect) = layout.rects().into_iter().find(|(_, rect)| {
        point.0 >= rect.x
            && point.0 <= rect.x + rect.width
            && point.1 >= rect.y
            && point.1 <= rect.y + rect.height
    })?;
    let dx = (point.0 - (rect.x + rect.width / 2.0)) / rect.width.max(f32::EPSILON);
    let dy = (point.1 - (rect.y + rect.height / 2.0)) / rect.height.max(f32::EPSILON);

    if dx.abs() >= dy.abs() {
        Some((id, Axis::Horizontal, dx < 0.0))
    } else {
        Some((id, Axis::Vertical, dy < 0.0))
    }
}

fn scrollbar_thumb_fraction(screen: &TerminalScreen, track_height: f32) -> f32 {
    if screen.scroll_total == 0 {
        return 1.0;
    }
    let visible = screen.scroll_len as f32 / screen.scroll_total as f32;
    let minimum = 20.0 / track_height.max(20.0);
    visible.max(minimum).min(1.0)
}

fn desktop_window_title(workspace_name: Option<&str>) -> String {
    workspace_name.map_or_else(
        || "Boomux Desktop".into(),
        |name| format!("Boomux Desktop — {name}"),
    )
}

struct Workspace {
    layout: Option<Node>,
    floating: Vec<FloatingPane>,
    pointer_drag: Option<PointerDrag>,
    layout_animation: Option<LayoutAnimation>,
    animation_generation: u64,
    focused: usize,
    fullscreen: Option<usize>,
    boomux_shells: Vec<ShellChoice>,
    boomux_overview: BoomuxOverview,
    boomux_error: Option<String>,
    expanded_workspaces: HashSet<String>,
    navigation_region: NavigationRegion,
    sidebar_item: Option<SidebarItem>,
    sidebar_scroll_handle: ScrollHandle,
    sidebar_scroll_anchor: ScrollAnchor,
    sidebar_menu: Option<SidebarMenu>,
    resource_dialog: Option<ResourceDialog>,
    help_open: bool,
    help_scroll_handle: ScrollHandle,
    terminals: HashMap<usize, TerminalPane>,
    next_id: usize,
    focus_handle: FocusHandle,
}

#[derive(Default)]
struct TerminalPane {
    shell: Option<ShellChoice>,
    session: Option<TerminalSession>,
    screen: Option<Arc<TerminalScreen>>,
    attaching: bool,
    error: Option<String>,
    scroll_remainder: f32,
    selection: Option<TerminalSelection>,
    render_images: HashMap<u64, Arc<RenderImage>>,
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let (boomux_overview, boomux_error) = match terminal::discover_overview() {
            Ok(overview) => (overview, None),
            Err(error) => (BoomuxOverview::default(), Some(error)),
        };
        let boomux_shells = boomux_overview
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.shells.iter().cloned())
            .collect::<Vec<_>>();

        let layout = Node::pane(1);
        let mut terminals = HashMap::new();
        terminals.insert(1, TerminalPane::default());

        let requested_shell_id = std::env::var("BOOMUX_DESKTOP_SHELL_ID").ok();
        let initial_shell = requested_shell_id
            .as_ref()
            .and_then(|requested_id| {
                boomux_shells
                    .iter()
                    .find(|shell| shell.id == *requested_id)
                    .cloned()
            })
            .or_else(|| {
                boomux_overview
                    .focused_shell_id
                    .as_ref()
                    .and_then(|focused_id| {
                        boomux_shells
                            .iter()
                            .find(|shell| shell.id == *focused_id)
                            .cloned()
                    })
            })
            .or_else(|| boomux_shells.first().cloned());
        let expanded_workspaces = initial_shell
            .as_ref()
            .map(|shell| HashSet::from([shell.workspace_id.clone()]))
            .unwrap_or_default();
        let sidebar_scroll_handle = ScrollHandle::new();
        let sidebar_scroll_anchor = ScrollAnchor::for_handle(sidebar_scroll_handle.clone());
        let help_scroll_handle = ScrollHandle::new();
        let mut workspace = Self {
            layout: Some(layout),
            floating: Vec::new(),
            pointer_drag: None,
            layout_animation: None,
            animation_generation: 0,
            focused: 1,
            fullscreen: None,
            boomux_shells,
            boomux_overview,
            boomux_error,
            expanded_workspaces,
            navigation_region: NavigationRegion::Terminal,
            sidebar_item: None,
            sidebar_scroll_handle,
            sidebar_scroll_anchor,
            sidebar_menu: None,
            resource_dialog: None,
            help_open: false,
            help_scroll_handle,
            terminals,
            next_id: 2,
            focus_handle,
        };
        if let Some(shell) = initial_shell {
            let size = workspace.terminal_grid_size(1, window);
            workspace.start_terminal_attachment(1, shell, size, cx);
        }
        workspace.watch_boomux_overview(cx);
        workspace
    }

    fn focus_direction(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation_region == NavigationRegion::Sidebar {
            if direction == Direction::Right {
                self.leave_sidebar(cx);
            }
            return;
        }

        if let Some(id) = self
            .layout
            .as_ref()
            .and_then(|layout| layout.neighbor(self.focused, direction))
        {
            self.focused = id;
            cx.notify();
        } else if direction == Direction::Left {
            self.enter_sidebar(window, cx);
        }
    }

    fn preferred_sidebar_item(&self, visible: &[SidebarItem]) -> Option<SidebarItem> {
        let pane = self.terminals.get(&self.focused);
        let shell_id = pane
            .and_then(|pane| pane.session.as_ref())
            .map(|terminal| terminal.shell_id.as_str());
        if let Some(shell) = shell_id
            && let Some(item) = visible.iter().find(
                |item| matches!(item, SidebarItem::Shell { shell_id, .. } if shell_id == shell),
            )
        {
            return Some(item.clone());
        }

        let workspace_id = pane
            .and_then(|pane| pane.shell.as_ref())
            .map(|shell| shell.workspace_id.as_str());
        workspace_id.and_then(|workspace| {
            visible
                .iter()
                .find(|item| matches!(item, SidebarItem::Workspace(id) if id == workspace))
                .cloned()
        })
    }

    fn reconcile_sidebar_item(&mut self) {
        let visible = visible_sidebar_items(&self.boomux_overview, &self.expanded_workspaces);
        let preferred = self.preferred_sidebar_item(&visible);
        self.sidebar_item =
            reconciled_sidebar_item(self.sidebar_item.as_ref(), preferred.as_ref(), &visible);
    }

    fn reveal_sidebar_item(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_scroll_anchor.scroll_to(window, cx);
    }

    fn enter_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation_region = NavigationRegion::Sidebar;
        self.reconcile_sidebar_item();
        window.focus(&self.focus_handle, cx);
        self.reveal_sidebar_item(window, cx);
        cx.notify();
    }

    fn leave_sidebar(&mut self, cx: &mut Context<Self>) {
        self.navigation_region = NavigationRegion::Terminal;
        if let Some(terminal) = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.session.as_ref())
        {
            terminal.focus();
        }
        cx.notify();
    }

    fn toggle_sidebar_focus(
        &mut self,
        _: &ToggleSidebarFocus,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation_region == NavigationRegion::Sidebar {
            self.leave_sidebar(cx);
        } else {
            self.enter_sidebar(window, cx);
        }
    }

    fn toggle_help(&mut self, _: &ToggleHelp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.help_open && self.resource_dialog.is_some() {
            return;
        }
        self.help_open = !self.help_open;
        if self.help_open {
            self.sidebar_menu = None;
            self.help_scroll_handle.set_offset(point(px(0.0), px(0.0)));
        }
        cx.notify();
    }

    fn help_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let current = self.help_scroll_handle.offset();
        let maximum = self.help_scroll_handle.max_offset();
        let next_y = match event.keystroke.key.as_str() {
            "escape" => {
                self.help_open = false;
                current.y
            }
            "up" | "k" => (current.y + px(48.0)).min(px(0.0)),
            "down" | "j" => (current.y - px(48.0)).max(-maximum.y),
            "pageup" => (current.y + px(360.0)).min(px(0.0)),
            "pagedown" | "space" => (current.y - px(360.0)).max(-maximum.y),
            "home" => px(0.0),
            "end" => -maximum.y,
            _ => current.y,
        };
        if next_y != current.y {
            self.help_scroll_handle.set_offset(point(current.x, next_y));
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn sidebar_resource(&self, item: &SidebarItem) -> Option<SidebarResource> {
        match item {
            SidebarItem::Workspace(id) => self
                .boomux_overview
                .workspaces
                .iter()
                .find(|workspace| workspace.id == *id)
                .map(|workspace| SidebarResource::Workspace {
                    id: workspace.id.clone(),
                    name: workspace.name.clone(),
                }),
            SidebarItem::Shell { shell_id, .. } | SidebarItem::Agent { shell_id, .. } => self
                .boomux_shells
                .iter()
                .find(|shell| shell.id == *shell_id)
                .map(|shell| SidebarResource::Shell {
                    id: shell.id.clone(),
                    workspace_id: shell.workspace_id.clone(),
                    name: shell.name.clone(),
                }),
        }
    }

    fn focused_shell_resource(&self) -> Option<SidebarResource> {
        self.terminals
            .get(&self.focused)
            .and_then(|pane| pane.shell.as_ref())
            .map(|shell| SidebarResource::Shell {
                id: shell.id.clone(),
                workspace_id: shell.workspace_id.clone(),
                name: shell.name.clone(),
            })
    }

    fn keyboard_resource(&self) -> Option<SidebarResource> {
        if self.navigation_region == NavigationRegion::Sidebar {
            self.sidebar_item
                .as_ref()
                .and_then(|item| self.sidebar_resource(item))
        } else {
            self.focused_shell_resource()
        }
    }

    fn open_sidebar_menu(
        &mut self,
        target: SidebarResource,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu_height = if matches!(target, SidebarResource::Workspace { .. }) {
            112.0
        } else {
            78.0
        };
        let maximum = (f32::from(window.viewport_size().height) - menu_height - 8.0).max(8.0);
        self.sidebar_menu = Some(SidebarMenu {
            target,
            top: f32::from(event.position().y).clamp(8.0, maximum),
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn open_resource_dialog(&mut self, kind: ResourceDialogKind, target: SidebarResource) {
        let value = target.name().to_string();
        self.sidebar_menu = None;
        self.resource_dialog = Some(ResourceDialog {
            kind,
            target,
            value,
            busy: false,
            error: None,
        });
    }

    fn rename_resource(&mut self, _: &RenameResource, _: &mut Window, cx: &mut Context<Self>) {
        if self.resource_dialog.is_some() {
            return;
        }
        if let Some(target) = self.keyboard_resource() {
            self.open_resource_dialog(ResourceDialogKind::Rename, target);
            cx.notify();
        }
    }

    fn remove_shell(&mut self, _: &RemoveShell, _: &mut Window, cx: &mut Context<Self>) {
        if self.resource_dialog.is_some() {
            return;
        }
        if let Some(target @ SidebarResource::Shell { .. }) = self.keyboard_resource() {
            self.open_resource_dialog(ResourceDialogKind::Remove, target);
            cx.notify();
        }
    }

    fn remove_resource_panes(&mut self, target: &SidebarResource, window: &mut Window) {
        let pane_ids = self
            .terminals
            .iter()
            .filter_map(|(id, pane)| {
                let shell = pane.shell.as_ref()?;
                let matches = match target {
                    SidebarResource::Workspace { id, .. } => shell.workspace_id == *id,
                    SidebarResource::Shell { id, .. } => shell.id == *id,
                };
                matches.then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in &pane_ids {
            if let Some(pane) = self.terminals.remove(id) {
                for image in pane.render_images.into_values() {
                    let _ = window.drop_image(image);
                }
            }
            self.floating.retain(|pane| pane.id != *id);
            self.layout = self.layout.take().and_then(|layout| layout.remove(*id));
            if self.fullscreen == Some(*id) {
                self.fullscreen = None;
            }
        }
        if self
            .pointer_drag
            .as_ref()
            .is_some_and(|drag| pane_ids.contains(&drag.pane_id))
        {
            self.pointer_drag = None;
        }
        self.focus_after_removal();
    }

    fn resource_dialog_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.resource_dialog.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        if key == "escape" && !dialog.busy {
            self.resource_dialog = None;
        } else if key == "enter" {
            self.submit_resource_dialog(window, cx);
        } else if dialog.kind == ResourceDialogKind::Rename && !dialog.busy {
            if key == "backspace" {
                dialog.value.pop();
                dialog.error = None;
            } else if !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && !event.keystroke.modifiers.function
                && let Some(text) = event.keystroke.key_char.as_deref()
            {
                append_resource_name(&mut dialog.value, text);
                dialog.error = None;
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn submit_resource_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.resource_dialog.as_mut() else {
            return;
        };
        if dialog.busy {
            return;
        }
        if dialog.kind == ResourceDialogKind::Rename && dialog.value.trim().is_empty() {
            dialog.error = Some("Name cannot be empty".into());
            cx.notify();
            return;
        }
        dialog.busy = true;
        dialog.error = None;
        let kind = dialog.kind;
        let target = dialog.target.clone();
        let value = dialog.value.trim().to_string();
        if kind == ResourceDialogKind::Remove {
            self.remove_resource_panes(&target, window);
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let operation_target = target.clone();
            let operation_value = value.clone();
            let result = cx
                .background_spawn(async move {
                    match (&operation_target, kind) {
                        (SidebarResource::Workspace { id, .. }, ResourceDialogKind::Rename) => {
                            terminal::rename_workspace(id, &operation_value)?;
                        }
                        (SidebarResource::Shell { id, .. }, ResourceDialogKind::Rename) => {
                            terminal::rename_shell(id, &operation_value)?;
                        }
                        (SidebarResource::Workspace { id, .. }, ResourceDialogKind::Remove) => {
                            terminal::remove_workspace(id)?;
                        }
                        (SidebarResource::Shell { id, .. }, ResourceDialogKind::Remove) => {
                            terminal::close_shell(id)?;
                        }
                    }
                    terminal::discover_overview()
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(overview) => {
                        if kind == ResourceDialogKind::Rename
                            && let SidebarResource::Shell { id, .. } = &target
                        {
                            for pane in this.terminals.values_mut() {
                                if let Some(shell) =
                                    pane.shell.as_mut().filter(|shell| shell.id == *id)
                                {
                                    shell.name = value.clone();
                                }
                                if let Some(session) = pane
                                    .session
                                    .as_mut()
                                    .filter(|session| session.shell_id == *id)
                                {
                                    session.shell_name = value.clone();
                                }
                            }
                        }
                        this.boomux_shells = overview
                            .workspaces
                            .iter()
                            .flat_map(|workspace| workspace.shells.iter().cloned())
                            .collect();
                        this.boomux_overview = overview;
                        this.resource_dialog = None;
                        this.reconcile_sidebar_item();
                    }
                    Err(error) => {
                        if let Some(dialog) = this.resource_dialog.as_mut() {
                            dialog.busy = false;
                            dialog.error = Some(error);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn move_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.fullscreen.is_some() {
            return;
        }
        self.layout_animation = None;
        if let Some(floating) = self
            .floating
            .iter_mut()
            .find(|pane| pane.id == self.focused)
        {
            match direction {
                Direction::Left => floating.x -= 24.0,
                Direction::Right => floating.x += 24.0,
                Direction::Up => floating.y -= 24.0,
                Direction::Down => floating.y += 24.0,
            }
            floating.x = floating.x.max(0.0);
            floating.y = floating.y.max(0.0);
        } else if let Some(previous_rects) = self
            .layout
            .as_mut()
            .and_then(|layout| swap_layout_direction(layout, self.focused, direction))
        {
            self.begin_layout_animation(previous_rects);
        }
        cx.notify();
    }

    fn resize_direction(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.fullscreen.is_some() {
            return;
        }
        self.layout_animation = None;
        if let Some(floating) = self
            .floating
            .iter_mut()
            .find(|pane| pane.id == self.focused)
        {
            match direction {
                Direction::Left => floating.width = (floating.width - 24.0).max(220.0),
                Direction::Right => floating.width += 24.0,
                Direction::Up => floating.height = (floating.height - 24.0).max(160.0),
                Direction::Down => floating.height += 24.0,
            }
        } else {
            if let Some(layout) = &mut self.layout {
                layout.resize(self.focused, direction, 0.04);
            }
        }
        cx.notify();
    }

    fn new_pane(&mut self, _: &NewPane, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation_region == NavigationRegion::Sidebar
            && let Some(workspace_id) = self
                .sidebar_item
                .as_ref()
                .and_then(|item| workspace_id_for_sidebar_item(item, &self.boomux_overview))
        {
            self.create_and_attach_workspace_terminal(workspace_id, window, cx);
            return;
        }
        self.navigation_region = NavigationRegion::Terminal;
        self.fullscreen = None;
        self.layout_animation = None;
        let anchor = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.shell.clone())
            .or_else(|| self.boomux_shells.first().cloned());
        let id = self.insert_pane();
        self.focused = id;
        if let Some(anchor) = anchor {
            self.create_and_attach_terminal(id, anchor, window, cx);
        } else if let Some(pane) = self.terminals.get_mut(&id) {
            pane.error = Some("No Boomux workspace is available for a new terminal".into());
            cx.notify();
        }
    }

    fn insert_pane(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let axis = if id.is_multiple_of(2) {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        if let Some(layout) = &mut self.layout {
            let target = if layout.contains(self.focused) {
                self.focused
            } else {
                layout.pane_ids()[0]
            };
            layout.split(target, id, axis);
        } else {
            self.layout = Some(Node::pane(id));
        }
        self.terminals.insert(id, TerminalPane::default());
        id
    }

    fn close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        self.layout_animation = None;
        if self.fullscreen == Some(self.focused) {
            self.fullscreen = None;
        }
        if let Some(pane) = self.terminals.remove(&self.focused) {
            for image in pane.render_images.into_values() {
                let _ = window.drop_image(image);
            }
        }
        if let Some(index) = self
            .floating
            .iter()
            .position(|pane| pane.id == self.focused)
        {
            self.floating.remove(index);
            self.focus_after_removal();
        } else if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.contains(self.focused))
        {
            self.layout = self
                .layout
                .take()
                .and_then(|layout| layout.remove(self.focused));
            self.focus_after_removal();
        }
        cx.notify();
    }

    fn toggle_floating(&mut self, _: &ToggleFloating, _: &mut Window, cx: &mut Context<Self>) {
        self.layout_animation = None;
        if let Some(index) = self
            .floating
            .iter()
            .position(|pane| pane.id == self.focused)
        {
            let pane = self.floating.remove(index);
            if let Some(layout) = &mut self.layout {
                let target = layout.pane_ids()[0];
                layout.split(target, pane.id, Axis::Horizontal);
            } else {
                self.layout = Some(Node::pane(pane.id));
            }
        } else if self
            .layout
            .as_ref()
            .is_some_and(|layout| layout.contains(self.focused))
        {
            let id = self.focused;
            self.layout = self.layout.take().and_then(|layout| layout.remove(id));
            let offset = self.floating.len() as f32 * 28.0;
            self.floating.push(FloatingPane {
                id,
                x: 110.0 + offset,
                y: 80.0 + offset,
                width: 440.0,
                height: 270.0,
            });
        }
        cx.notify();
    }

    fn toggle_fullscreen(&mut self, _: &ToggleFullscreen, _: &mut Window, cx: &mut Context<Self>) {
        if self.pointer_drag.is_some() {
            return;
        }

        self.fullscreen = match self.fullscreen {
            Some(_) => None,
            None if self
                .layout
                .as_ref()
                .is_some_and(|layout| layout.contains(self.focused))
                || self.floating.iter().any(|pane| pane.id == self.focused) =>
            {
                Some(self.focused)
            }
            None => None,
        };
        cx.notify();
    }

    fn focus_after_removal(&mut self) {
        if let Some(pane) = self.floating.last() {
            self.focused = pane.id;
        } else if let Some(layout) = &self.layout {
            self.focused = layout.pane_ids()[0];
        }
    }

    fn panel_size(window: &Window) -> (f32, f32) {
        let viewport = window.viewport_size();
        (
            (f32::from(viewport.width) - SIDEBAR_WIDTH).max(0.0),
            (f32::from(viewport.height) - HEADER_HEIGHT - FOOTER_HEIGHT).max(0.0),
        )
    }

    fn pointer_in_panel(event: &MouseDownEvent) -> (f32, f32) {
        window_point_to_panel(f32::from(event.position.x), f32::from(event.position.y))
    }

    fn pointer_subject(&mut self, id: usize) -> Option<PointerSubject> {
        if let Some(index) = self.floating.iter().position(|pane| pane.id == id) {
            let pane = self.floating.remove(index);
            self.floating.push(pane.clone());
            return Some(PointerSubject::Floating(pane));
        }

        self.layout
            .as_ref()
            .filter(|layout| layout.contains(id))
            .cloned()
            .map(PointerSubject::Tiled)
    }

    fn begin_layout_animation(&mut self, from: HashMap<usize, Rect>) {
        self.animation_generation = self.animation_generation.wrapping_add(1);
        self.layout_animation = Some(LayoutAnimation {
            from,
            generation: self.animation_generation,
        });
    }

    fn normalized_panel_point(pointer: (f32, f32), window: &Window) -> (f32, f32) {
        let (panel_width, panel_height) = Self::panel_size(window);
        let inner_width = (panel_width - PANEL_PADDING * 2.0).max(1.0);
        let inner_height = (panel_height - PANEL_PADDING * 2.0).max(1.0);
        (
            ((pointer.0 - PANEL_PADDING) / inner_width).clamp(0.0, 1.0),
            ((pointer.1 - PANEL_PADDING) / inner_height).clamp(0.0, 1.0),
        )
    }

    fn lift_tiled_pane(&mut self, id: usize, window: &Window) -> Option<FloatingPane> {
        let layout = self.layout.as_ref()?;
        let previous_rects = layout.rects().into_iter().collect::<HashMap<_, _>>();
        let (panel_width, panel_height) = Self::panel_size(window);
        let inner_width = (panel_width - PANEL_PADDING * 2.0).max(0.0);
        let inner_height = (panel_height - PANEL_PADDING * 2.0).max(0.0);
        let (_, rect) = layout
            .rects()
            .into_iter()
            .find(|(pane_id, _)| *pane_id == id)?;
        let pane = FloatingPane {
            id,
            x: PANEL_PADDING + rect.x * inner_width,
            y: PANEL_PADDING + rect.y * inner_height,
            width: (rect.width * inner_width - PANEL_PADDING)
                .max(MIN_FLOAT_WIDTH)
                .min(inner_width),
            height: (rect.height * inner_height - PANEL_PADDING)
                .max(MIN_FLOAT_HEIGHT)
                .min(inner_height),
        };

        self.layout = self.layout.take().and_then(|layout| layout.remove(id));
        self.floating.push(pane.clone());
        self.begin_layout_animation(previous_rects);
        Some(pane)
    }

    fn finish_pointer_drag(&mut self, pointer: (f32, f32), window: &Window) {
        let Some(drag) = self.pointer_drag.take() else {
            return;
        };
        if !matches!(drag.subject, PointerSubject::Lifted(_)) {
            return;
        }

        let dragged_pane = self
            .floating
            .iter()
            .find(|pane| pane.id == drag.pane_id)
            .cloned();
        let mut previous_rects = self
            .layout
            .as_ref()
            .map(|layout| layout.rects().into_iter().collect::<HashMap<_, _>>())
            .unwrap_or_default();
        if let Some(pane) = &dragged_pane {
            let (panel_width, panel_height) = Self::panel_size(window);
            let inner_width = (panel_width - PANEL_PADDING * 2.0).max(1.0);
            let inner_height = (panel_height - PANEL_PADDING * 2.0).max(1.0);
            previous_rects.insert(
                pane.id,
                Rect {
                    x: ((pane.x - PANEL_PADDING) / inner_width).clamp(0.0, 1.0),
                    y: ((pane.y - PANEL_PADDING) / inner_height).clamp(0.0, 1.0),
                    width: (pane.width / inner_width).clamp(0.0, 1.0),
                    height: (pane.height / inner_height).clamp(0.0, 1.0),
                },
            );
        }
        self.floating.retain(|pane| pane.id != drag.pane_id);
        let point = Self::normalized_panel_point(pointer, window);
        if let Some(layout) = &mut self.layout {
            if let Some((target, axis, before)) = drop_placement(layout, point) {
                layout.split_at(target, drag.pane_id, axis, before);
            } else {
                let target = layout.pane_ids()[0];
                layout.split(target, drag.pane_id, Axis::Horizontal);
            }
        } else {
            self.layout = Some(Node::pane(drag.pane_id));
        }
        self.begin_layout_animation(previous_rects);
    }

    fn begin_pointer_interaction(
        &mut self,
        id: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focused = id;
        self.navigation_region = NavigationRegion::Terminal;
        window.focus(&self.focus_handle, cx);
        if let Some(terminal) = self
            .terminals
            .get(&id)
            .and_then(|pane| pane.session.as_ref())
        {
            terminal.focus();
        }

        if !event.modifiers.control || self.fullscreen == Some(id) {
            cx.notify();
            return;
        }

        let operation = match event.button {
            MouseButton::Left => PointerOperation::Move,
            MouseButton::Right => PointerOperation::Resize,
            _ => return,
        };
        if matches!(operation, PointerOperation::Resize) {
            self.layout_animation = None;
        }
        let subject = self.pointer_subject(id);
        let Some(subject) = subject else {
            return;
        };

        self.pointer_drag = Some(PointerDrag {
            operation,
            button: event.button,
            start_pointer: Self::pointer_in_panel(event),
            pane_id: id,
            subject,
            activated: false,
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn focus_pane_on_hover(
        &mut self,
        id: usize,
        _: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keep the dragged pane focused even while it crosses other panes.
        if self.pointer_drag.is_some() {
            return;
        }
        if self.focused == id && self.navigation_region == NavigationRegion::Terminal {
            return;
        }
        self.focused = id;
        self.navigation_region = NavigationRegion::Terminal;
        window.focus(&self.focus_handle, cx);
        if let Some(terminal) = self
            .terminals
            .get(&id)
            .and_then(|pane| pane.session.as_ref())
        {
            terminal.focus();
        }
        cx.notify();
    }

    fn scroll_terminal(
        &mut self,
        id: usize,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pointer_drag.is_some() {
            return;
        }

        let Some(pane) = self.terminals.get_mut(&id) else {
            return;
        };
        let Some(terminal) = pane.session.as_ref() else {
            return;
        };

        let pixels = f32::from(event.delta.pixel_delta(px(TERMINAL_CELL_HEIGHT)).y);
        if pixels == 0.0 {
            return;
        }
        if pane.scroll_remainder != 0.0 && pane.scroll_remainder.signum() != pixels.signum() {
            pane.scroll_remainder = 0.0;
        }
        pane.scroll_remainder += pixels;
        let lines = (pane.scroll_remainder / TERMINAL_CELL_HEIGHT).trunc() as isize;
        if lines == 0 {
            return;
        }
        pane.scroll_remainder -= lines as f32 * TERMINAL_CELL_HEIGHT;
        if terminal.scroll(-lines) {
            cx.stop_propagation();
        }
    }

    fn drag_terminal_scrollbar(
        &mut self,
        event: &DragMoveEvent<TerminalScrollbarDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = event.drag(cx).0;
        let Some(pane) = self.terminals.get(&pane_id) else {
            return;
        };
        let Some(terminal) = &pane.session else {
            return;
        };
        let Some(screen) = &pane.screen else {
            return;
        };
        let maximum = screen.scroll_total.saturating_sub(screen.scroll_len);
        if maximum == 0 || event.bounds.size.height <= px(0.0) {
            return;
        }

        let pointer = f32::from(event.event.position.y - event.bounds.top());
        let track_height = f32::from(event.bounds.size.height);
        let thumb_fraction = scrollbar_thumb_fraction(screen, track_height);
        let progress = ((pointer / track_height - thumb_fraction / 2.0)
            / (1.0 - thumb_fraction).max(f32::EPSILON))
        .clamp(0.0, 1.0);
        let row = (progress * maximum as f32).round() as usize;
        terminal.scroll_to(row);
        cx.stop_propagation();
    }

    fn drag_terminal_selection(
        &mut self,
        event: &DragMoveEvent<TerminalSelectionDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The terminal surface also owns ordinary left-drag selection. Let a
        // compositor drag bubble to the workspace-level pointer handler instead
        // of consuming its mouse moves as selection updates.
        if self.pointer_drag.is_some() || event.event.modifiers.control {
            return;
        }
        let drag = event.drag(cx).clone();
        let Some(pane) = self.terminals.get_mut(&drag.pane_id) else {
            return;
        };
        let Some(screen) = pane.screen.as_ref() else {
            return;
        };
        let offset_x = f32::from(event.event.position.x - event.bounds.left());
        let offset_y = f32::from(event.event.position.y - event.bounds.top());
        let cell = terminal_cell_from_offset(offset_x, offset_y, screen);
        if !drag.started.swap(true, Ordering::AcqRel) {
            pane.selection = Some(TerminalSelection {
                anchor: cell,
                head: cell,
            });
        } else if let Some(selection) = &mut pane.selection {
            selection.head = cell;
        }
        let selected = pane
            .selection
            .map(|selection| terminal_selected_text(screen, selection));
        if let Some(text) = selected.filter(|text| !text.is_empty()) {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn copy_selection(&mut self, _: &CopySelection, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.terminals.get(&self.focused).and_then(|pane| {
            Some(terminal_selected_text(
                pane.screen.as_ref()?,
                pane.selection?,
            ))
        }) else {
            return;
        };
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.stop_propagation();
        }
    }

    fn paste_clipboard(&mut self, _: &PasteClipboard, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if let Some(ResourceDialog {
                kind: ResourceDialogKind::Rename,
                value,
                busy: false,
                ..
            }) = self.resource_dialog.as_mut()
            {
                append_resource_name(value, &text);
                cx.stop_propagation();
                cx.notify();
            } else {
                self.paste_into_focused(&text, cx);
            }
        }
    }

    fn paste_primary(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            self.paste_into_focused(&text, cx);
        }
    }

    fn paste_into_focused(&mut self, text: &str, cx: &mut Context<Self>) {
        let pasted = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.session.as_ref())
            .is_some_and(|terminal| terminal.paste(text));
        if pasted {
            if let Some(pane) = self.terminals.get_mut(&self.focused) {
                pane.selection = None;
            }
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_pointer_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pointer =
            window_point_to_panel(f32::from(event.position.x), f32::from(event.position.y));
        let Some(mut drag) = self.pointer_drag.clone() else {
            return;
        };
        if event.pressed_button != Some(drag.button) {
            self.finish_pointer_drag(pointer, window);
            cx.notify();
            return;
        }

        let dx = pointer.0 - drag.start_pointer.0;
        let dy = pointer.1 - drag.start_pointer.1;
        if !drag.activated {
            if dx.hypot(dy) < DRAG_ACTIVATION_DISTANCE {
                return;
            }
            drag.activated = true;
            if matches!(drag.operation, PointerOperation::Move)
                && matches!(drag.subject, PointerSubject::Tiled(_))
            {
                let Some(bounds) = self.lift_tiled_pane(drag.pane_id, window) else {
                    self.pointer_drag = None;
                    return;
                };
                drag.subject = PointerSubject::Lifted(bounds);
            }
            self.pointer_drag = Some(drag.clone());
        }
        let (panel_width, panel_height) = Self::panel_size(window);

        match drag.subject {
            PointerSubject::Floating(start_bounds) | PointerSubject::Lifted(start_bounds) => {
                let Some(pane) = self
                    .floating
                    .iter_mut()
                    .find(|pane| pane.id == drag.pane_id)
                else {
                    self.pointer_drag = None;
                    return;
                };
                *pane = dragged_bounds(
                    start_bounds,
                    drag.operation,
                    (dx, dy),
                    (panel_width, panel_height),
                );
            }
            PointerSubject::Tiled(start_layout) => match drag.operation {
                PointerOperation::Move => unreachable!("tiled moves are lifted before dragging"),
                PointerOperation::Resize => {
                    self.layout = Some(resized_tiled_layout(
                        start_layout,
                        drag.pane_id,
                        (dx, dy),
                        (panel_width, panel_height),
                    ));
                }
            },
        }
        cx.notify();
    }

    fn end_pointer_interaction(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pointer =
            window_point_to_panel(f32::from(event.position.x), f32::from(event.position.y));
        self.finish_pointer_drag(pointer, window);
        cx.notify();
    }

    fn focus_left(&mut self, _: &FocusLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Left, window, cx);
    }
    fn focus_right(&mut self, _: &FocusRight, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Right, window, cx);
    }
    fn focus_up(&mut self, _: &FocusUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Up, window, cx);
    }
    fn focus_down(&mut self, _: &FocusDown, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_direction(Direction::Down, window, cx);
    }
    fn move_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_direction(Direction::Left, cx);
    }
    fn move_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_direction(Direction::Right, cx);
    }
    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_direction(Direction::Up, cx);
    }
    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_direction(Direction::Down, cx);
    }
    fn resize_left(&mut self, _: &ResizeLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_direction(Direction::Left, cx);
    }
    fn resize_right(&mut self, _: &ResizeRight, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_direction(Direction::Right, cx);
    }
    fn resize_up(&mut self, _: &ResizeUp, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_direction(Direction::Up, cx);
    }
    fn resize_down(&mut self, _: &ResizeDown, _: &mut Window, cx: &mut Context<Self>) {
        self.resize_direction(Direction::Down, cx);
    }

    fn move_sidebar_selection(
        &mut self,
        offset: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = visible_sidebar_items(&self.boomux_overview, &self.expanded_workspaces);
        if visible.is_empty() {
            self.sidebar_item = None;
            return;
        }
        let current = self
            .sidebar_item
            .as_ref()
            .and_then(|item| visible.iter().position(|candidate| candidate == item))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(offset)
            .min(visible.len().saturating_sub(1));
        self.sidebar_item = Some(visible[next].clone());
        self.reveal_sidebar_item(window, cx);
        cx.notify();
    }

    fn move_sidebar_to_edge(&mut self, last: bool, window: &mut Window, cx: &mut Context<Self>) {
        let visible = visible_sidebar_items(&self.boomux_overview, &self.expanded_workspaces);
        self.sidebar_item = if last {
            visible.last().cloned()
        } else {
            visible.first().cloned()
        };
        self.reveal_sidebar_item(window, cx);
        cx.notify();
    }

    fn navigate_sidebar_tree(&mut self, expand: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.sidebar_item.clone();
        match selected {
            Some(SidebarItem::Workspace(workspace_id)) => {
                if expand {
                    self.expanded_workspaces.insert(workspace_id);
                } else {
                    self.expanded_workspaces.remove(&workspace_id);
                }
            }
            Some(SidebarItem::Shell { workspace_id, .. }) if !expand => {
                self.sidebar_item = Some(SidebarItem::Workspace(workspace_id));
            }
            _ => return,
        }
        self.reconcile_sidebar_item();
        self.reveal_sidebar_item(window, cx);
        cx.notify();
    }

    fn jump_sidebar_section(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = visible_sidebar_items(&self.boomux_overview, &self.expanded_workspaces);
        let in_agents = matches!(self.sidebar_item, Some(SidebarItem::Agent { .. }));
        let target = if in_agents {
            if backwards {
                visible
                    .iter()
                    .rev()
                    .find(|item| matches!(item, SidebarItem::Workspace(_)))
            } else {
                visible
                    .iter()
                    .find(|item| matches!(item, SidebarItem::Workspace(_)))
            }
            .or_else(|| visible.first())
        } else {
            if backwards {
                visible
                    .iter()
                    .rev()
                    .find(|item| matches!(item, SidebarItem::Agent { .. }))
            } else {
                visible
                    .iter()
                    .find(|item| matches!(item, SidebarItem::Agent { .. }))
            }
            .or_else(|| visible.first())
        };
        self.sidebar_item = target.cloned();
        self.reveal_sidebar_item(window, cx);
        cx.notify();
    }

    fn activate_sidebar_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.sidebar_item.clone() else {
            return;
        };
        match item {
            SidebarItem::Workspace(workspace_id) => self.toggle_workspace(&workspace_id, cx),
            SidebarItem::Shell { shell_id, .. } | SidebarItem::Agent { shell_id, .. } => {
                self.activate_sidebar_shell(&shell_id, window, cx);
            }
        }
    }

    fn sidebar_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let handled = match event.keystroke.key.as_str() {
            "up" | "k" => {
                self.move_sidebar_selection(-1, window, cx);
                true
            }
            "down" | "j" => {
                self.move_sidebar_selection(1, window, cx);
                true
            }
            "left" | "h" => {
                self.navigate_sidebar_tree(false, window, cx);
                true
            }
            "right" | "l" => {
                self.navigate_sidebar_tree(true, window, cx);
                true
            }
            "home" => {
                self.move_sidebar_to_edge(false, window, cx);
                true
            }
            "end" => {
                self.move_sidebar_to_edge(true, window, cx);
                true
            }
            "tab" => {
                self.jump_sidebar_section(modifiers.shift, window, cx);
                true
            }
            "enter" | "space" => {
                self.activate_sidebar_item(window, cx);
                true
            }
            "escape" => {
                self.leave_sidebar(cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.help_open {
            self.help_key_down(event, cx);
            return;
        }
        if self.resource_dialog.is_some() {
            self.resource_dialog_key_down(event, window, cx);
            return;
        }
        if self.sidebar_menu.is_some() && event.keystroke.key == "escape" {
            self.sidebar_menu = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.navigation_region == NavigationRegion::Sidebar {
            self.sidebar_key_down(event, window, cx);
            return;
        }
        let Some(pane) = self.terminals.get(&self.focused) else {
            return;
        };
        let modifiers = event.keystroke.modifiers;
        let scroll_modifier = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.shift
        };
        if scroll_modifier && !modifiers.control && !modifiers.alt && !modifiers.function {
            let handled = match event.keystroke.key.as_str() {
                "pageup" => pane.session.as_ref().is_some_and(|terminal| {
                    terminal.scroll(-pane.screen.as_ref().map_or(1, |screen| {
                        isize::try_from(screen.scroll_len).unwrap_or(isize::MAX)
                    }))
                }),
                "pagedown" => pane.session.as_ref().is_some_and(|terminal| {
                    terminal.scroll(pane.screen.as_ref().map_or(1, |screen| {
                        isize::try_from(screen.scroll_len).unwrap_or(isize::MAX)
                    }))
                }),
                "home" => pane.session.as_ref().is_some_and(|terminal| {
                    terminal.scroll_to_top();
                    true
                }),
                "end" => pane.session.as_ref().is_some_and(|terminal| {
                    terminal.scroll_to_bottom();
                    true
                }),
                _ => false,
            };
            if handled {
                cx.stop_propagation();
                return;
            }
        }
        if workspace_keystroke(&event.keystroke) {
            return;
        }
        let sent = pane
            .session
            .as_ref()
            .is_some_and(|terminal| terminal.send_key(&event.keystroke));
        if sent {
            if let Some(pane) = self.terminals.get_mut(&self.focused) {
                pane.selection = None;
            }
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn start_terminal_attachment(
        &mut self,
        pane_id: usize,
        shell: ShellChoice,
        (rows, cols, pixel_width, pixel_height): (u16, u16, u16, u16),
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.terminals.get_mut(&pane_id) else {
            return;
        };
        pane.attaching = true;
        pane.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let attached_shell = shell.clone();
            let result = cx
                .background_spawn(async move {
                    TerminalSession::attach(shell, rows, cols, pixel_width, pixel_height)
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(pane) = this.terminals.get_mut(&pane_id) else {
                    return;
                };
                pane.attaching = false;
                match result {
                    Ok(terminal) => {
                        let shell_id = terminal.shell_id.clone();
                        pane.screen = Some(Arc::new(terminal.screen()));
                        pane.shell = Some(attached_shell);
                        pane.session = Some(terminal);
                        this.watch_terminal(pane_id, shell_id, cx);
                    }
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_and_attach_terminal(
        &mut self,
        pane_id: usize,
        anchor: ShellChoice,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let size = self.terminal_grid_size(pane_id, window);
        if let Some(pane) = self.terminals.get_mut(&pane_id) {
            pane.attaching = true;
            pane.error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let shell = terminal::create_shell(&anchor)?;
                    let session = match TerminalSession::attach(
                        shell.clone(),
                        size.0,
                        size.1,
                        size.2,
                        size.3,
                    ) {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = terminal::close_shell(&shell.id);
                            return Err(error);
                        }
                    };
                    let overview = terminal::discover_overview().ok();
                    Ok::<_, String>((shell, session, overview))
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(pane) = this.terminals.get_mut(&pane_id) else {
                    return;
                };
                pane.attaching = false;
                match result {
                    Ok((shell, session, overview)) => {
                        let shell_id = session.shell_id.clone();
                        pane.screen = Some(Arc::new(session.screen()));
                        pane.shell = Some(shell);
                        pane.session = Some(session);
                        if let Some(overview) = overview {
                            this.boomux_shells = overview
                                .workspaces
                                .iter()
                                .flat_map(|workspace| workspace.shells.iter().cloned())
                                .collect();
                            this.boomux_overview = overview;
                        }
                        this.watch_terminal(pane_id, shell_id, cx);
                    }
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_and_attach_workspace_terminal(
        &mut self,
        workspace_id: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_menu = None;
        self.navigation_region = NavigationRegion::Terminal;
        self.fullscreen = None;
        self.layout_animation = None;
        let pane_id = self.insert_pane();
        self.focused = pane_id;
        let size = self.terminal_grid_size(pane_id, window);
        if let Some(pane) = self.terminals.get_mut(&pane_id) {
            pane.attaching = true;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let shell = terminal::create_shell_in_workspace(&workspace_id)?;
                    let session = match TerminalSession::attach(
                        shell.clone(),
                        size.0,
                        size.1,
                        size.2,
                        size.3,
                    ) {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = terminal::close_shell(&shell.id);
                            return Err(error);
                        }
                    };
                    let overview = terminal::discover_overview().ok();
                    Ok::<_, String>((shell, session, overview))
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(pane) = this.terminals.get_mut(&pane_id) else {
                    return;
                };
                pane.attaching = false;
                match result {
                    Ok((shell, session, overview)) => {
                        let shell_id = session.shell_id.clone();
                        pane.screen = Some(Arc::new(session.screen()));
                        pane.shell = Some(shell);
                        pane.session = Some(session);
                        if let Some(overview) = overview {
                            this.boomux_shells = overview
                                .workspaces
                                .iter()
                                .flat_map(|workspace| workspace.shells.iter().cloned())
                                .collect();
                            this.boomux_overview = overview;
                        }
                        this.watch_terminal(pane_id, shell_id, cx);
                    }
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create_and_attach_new_workspace(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.sidebar_menu = None;
        self.navigation_region = NavigationRegion::Terminal;
        self.fullscreen = None;
        self.layout_animation = None;
        let pane_id = self.insert_pane();
        self.focused = pane_id;
        let size = self.terminal_grid_size(pane_id, window);
        if let Some(pane) = self.terminals.get_mut(&pane_id) {
            pane.attaching = true;
            pane.error = None;
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let shell = terminal::create_workspace_with_shell()?;
                    let session = match TerminalSession::attach(
                        shell.clone(),
                        size.0,
                        size.1,
                        size.2,
                        size.3,
                    ) {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = terminal::remove_workspace(&shell.workspace_id);
                            return Err(error);
                        }
                    };
                    let overview = terminal::discover_overview().ok();
                    Ok::<_, String>((shell, session, overview))
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(pane) = this.terminals.get_mut(&pane_id) else {
                    return;
                };
                pane.attaching = false;
                match result {
                    Ok((shell, session, overview)) => {
                        let shell_id = session.shell_id.clone();
                        let workspace_id = shell.workspace_id.clone();
                        pane.screen = Some(Arc::new(session.screen()));
                        pane.shell = Some(shell);
                        pane.session = Some(session);
                        this.expanded_workspaces.insert(workspace_id);
                        if let Some(overview) = overview {
                            this.boomux_shells = overview
                                .workspaces
                                .iter()
                                .flat_map(|workspace| workspace.shells.iter().cloned())
                                .collect();
                            this.boomux_overview = overview;
                        }
                        this.watch_terminal(pane_id, shell_id, cx);
                    }
                    Err(error) => pane.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn watch_boomux_overview(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let result = cx
                    .background_spawn(async { terminal::discover_overview() })
                    .await;
                let keep_watching = this
                    .update(cx, |this, cx| {
                        if let Ok(overview) = result
                            && (overview != this.boomux_overview || this.boomux_error.is_some())
                        {
                            this.boomux_shells = overview
                                .workspaces
                                .iter()
                                .flat_map(|workspace| workspace.shells.iter().cloned())
                                .collect();
                            this.boomux_overview = overview;
                            this.boomux_error = None;
                            if this.navigation_region == NavigationRegion::Sidebar {
                                this.reconcile_sidebar_item();
                            }
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_watching {
                    return;
                }
            }
        })
        .detach();
    }

    fn toggle_workspace(&mut self, workspace_id: &str, cx: &mut Context<Self>) {
        if !self.expanded_workspaces.remove(workspace_id) {
            self.expanded_workspaces.insert(workspace_id.to_string());
        }
        cx.notify();
    }

    fn activate_sidebar_shell(
        &mut self,
        shell_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((pane_id, terminal)) = self.terminals.iter().find_map(|(pane_id, pane)| {
            pane.session
                .as_ref()
                .filter(|terminal| terminal.shell_id == shell_id)
                .map(|terminal| (*pane_id, terminal))
        }) {
            self.navigation_region = NavigationRegion::Terminal;
            self.focused = pane_id;
            terminal.focus();
            window.focus(&self.focus_handle, cx);
            cx.notify();
            return;
        }

        let Some(shell) = self
            .boomux_shells
            .iter()
            .find(|shell| shell.id == shell_id)
            .cloned()
        else {
            self.boomux_error = Some("That Boomux shell is no longer available".into());
            cx.notify();
            return;
        };
        self.navigation_region = NavigationRegion::Terminal;
        self.fullscreen = None;
        self.layout_animation = None;
        let pane_id = self.insert_pane();
        self.focused = pane_id;
        let size = self.terminal_grid_size(pane_id, window);
        self.start_terminal_attachment(pane_id, shell, size, cx);
    }

    fn watch_terminal(&self, pane_id: usize, shell_id: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut revision = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let keep_watching = this
                    .update(cx, |this, cx| {
                        let Some(pane) = this.terminals.get_mut(&pane_id) else {
                            return false;
                        };
                        let Some(terminal) = pane
                            .session
                            .as_ref()
                            .filter(|terminal| terminal.shell_id == shell_id)
                        else {
                            return false;
                        };
                        let next_revision = terminal.revision();
                        if next_revision != revision {
                            pane.screen = Some(Arc::new(terminal.screen()));
                            revision = next_revision;
                            cx.notify();
                        }
                        !terminal.is_closed()
                    })
                    .unwrap_or(false);
                if !keep_watching {
                    return;
                }
            }
        })
        .detach();
    }

    fn terminal_grid_size(&self, id: usize, window: &Window) -> (u16, u16, u16, u16) {
        let viewport = window.viewport_size();
        let (width, height) = if self.fullscreen == Some(id) {
            (f32::from(viewport.width), f32::from(viewport.height))
        } else if let Some(pane) = self.floating.iter().find(|pane| pane.id == id) {
            (pane.width, pane.height)
        } else if let Some((_, rect)) = self.layout.as_ref().and_then(|layout| {
            layout
                .rects()
                .into_iter()
                .find(|(pane_id, _)| *pane_id == id)
        }) {
            let (panel_width, panel_height) = Self::panel_size(window);
            let inner_width = (panel_width - PANEL_PADDING * 2.0).max(1.0);
            let inner_height = (panel_height - PANEL_PADDING * 2.0).max(1.0);
            (rect.width * inner_width, rect.height * inner_height)
        } else {
            (640.0, 400.0)
        };
        let content_width = (width - TERMINAL_PADDING).max(TERMINAL_CELL_WIDTH * 2.0);
        let content_height = (height - 38.0 - TERMINAL_PADDING).max(TERMINAL_CELL_HEIGHT * 2.0);
        let cols = (content_width / TERMINAL_CELL_WIDTH)
            .floor()
            .clamp(2.0, f32::from(u16::MAX)) as u16;
        let rows = (content_height / TERMINAL_CELL_HEIGHT)
            .floor()
            .clamp(2.0, f32::from(u16::MAX)) as u16;
        (
            rows,
            cols,
            content_width.min(f32::from(u16::MAX)) as u16,
            content_height.min(f32::from(u16::MAX)) as u16,
        )
    }

    fn refresh_terminal_images(&mut self, window: &mut Window) {
        for pane in self.terminals.values_mut() {
            let Some(screen) = pane.screen.as_ref() else {
                continue;
            };
            let active = screen
                .images
                .iter()
                .map(|image| image.generation)
                .collect::<HashSet<_>>();
            let stale = pane
                .render_images
                .keys()
                .filter(|generation| !active.contains(generation))
                .copied()
                .collect::<Vec<_>>();
            for generation in stale {
                if let Some(image) = pane.render_images.remove(&generation) {
                    let _ = window.drop_image(image);
                }
            }
            for terminal_image in &screen.images {
                pane.render_images
                    .entry(terminal_image.generation)
                    .or_insert_with(|| {
                        let buffer = image::RgbaImage::from_raw(
                            terminal_image.width,
                            terminal_image.height,
                            terminal_image.bgra.to_vec(),
                        )
                        .expect("validated Ghostty image dimensions");
                        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                            buffer,
                        )]))
                    });
            }
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let focused_shell_id = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.session.as_ref())
            .map(|terminal| terminal.shell_id.as_str());
        let focused_workspace_id = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.shell.as_ref())
            .map(|shell| shell.workspace_id.as_str());

        let workspace_rows = self
            .boomux_overview
            .workspaces
            .iter()
            .cloned()
            .map(|workspace| {
                let workspace_id = workspace.id.clone();
                let workspace_item = SidebarItem::Workspace(workspace.id.clone());
                let workspace_keyboard_selected = self.navigation_region
                    == NavigationRegion::Sidebar
                    && self.sidebar_item.as_ref() == Some(&workspace_item);
                let expanded = self.expanded_workspaces.contains(&workspace.id);
                let active = focused_workspace_id == Some(workspace.id.as_str());
                let shell_count = workspace.shells.len();
                let shell_rows = workspace
                    .shells
                    .into_iter()
                    .map(|shell| {
                        let shell_id = shell.id.clone();
                        let shell_target = SidebarResource::Shell {
                            id: shell.id.clone(),
                            workspace_id: workspace.id.clone(),
                            name: shell.name.clone(),
                        };
                        let shell_item = SidebarItem::Shell {
                            workspace_id: workspace.id.clone(),
                            shell_id: shell.id.clone(),
                        };
                        let keyboard_selected = self.navigation_region == NavigationRegion::Sidebar
                            && self.sidebar_item.as_ref() == Some(&shell_item);
                        let selected = focused_shell_id == Some(shell.id.as_str());
                        let status = shell.status_label();
                        div()
                            .id(SharedString::from(format!("sidebar-shell-{}", shell.id)))
                            .ml_6()
                            .h(px(39.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .anchor_scroll(
                                keyboard_selected.then(|| self.sidebar_scroll_anchor.clone()),
                            )
                            .bg(if keyboard_selected {
                                rgb(0x45475a)
                            } else if selected {
                                rgb(0x252536)
                            } else {
                                rgb(0x181825)
                            })
                            .hover(|element| element.bg(rgb(0x29293d)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_sidebar_shell(&shell_id, window, cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(if status == "running" {
                                        0x89b4fa
                                    } else {
                                        0x6c7086
                                    }))
                                    .child("○"),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(0xcdd6f4))
                                            .child(shell.name),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x6c7086))
                                            .child(format!("shell · {status}")),
                                    ),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "sidebar-shell-menu-{}",
                                        shell.id
                                    )))
                                    .w(px(24.0))
                                    .h(px(28.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_color(rgb(0x7f849c))
                                    .hover(|element| {
                                        element.bg(rgb(0x45475a)).text_color(rgb(0xcdd6f4))
                                    })
                                    .on_click(cx.listener(move |this, event, window, cx| {
                                        this.open_sidebar_menu(
                                            shell_target.clone(),
                                            event,
                                            window,
                                            cx,
                                        );
                                    }))
                                    .child("⋮"),
                            )
                    })
                    .collect::<Vec<_>>();

                let workspace_target = SidebarResource::Workspace {
                    id: workspace.id.clone(),
                    name: workspace.name.clone(),
                };

                div()
                    .id(SharedString::from(format!(
                        "sidebar-workspace-{}",
                        workspace.id
                    )))
                    .w_full()
                    .rounded_md()
                    .bg(if active { rgb(0x202235) } else { rgb(0x181825) })
                    .when(active, |element| {
                        element.border_l_2().border_color(rgb(0x45475a))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "sidebar-workspace-header-{}",
                                workspace_id
                            )))
                            .anchor_scroll(
                                workspace_keyboard_selected
                                    .then(|| self.sidebar_scroll_anchor.clone()),
                            )
                            .h(px(52.0))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(workspace_keyboard_selected, |element| {
                                element
                                    .bg(rgb(0x45475a))
                                    .border_l_2()
                                    .border_color(rgb(0xcba6f7))
                            })
                            .hover(|element| element.bg(rgb(0x29293d)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigation_region = NavigationRegion::Sidebar;
                                this.sidebar_item = Some(workspace_item.clone());
                                window.focus(&this.focus_handle, cx);
                                this.toggle_workspace(&workspace_id, cx);
                            }))
                            .child(
                                div()
                                    .w(px(14.0))
                                    .text_xs()
                                    .text_color(if active { rgb(0x89b4fa) } else { rgb(0x6c7086) })
                                    .child(if expanded { "▾" } else { "▸" }),
                            )
                            .child(div().size_2().rounded_full().bg(if active {
                                rgb(0x89b4fa)
                            } else {
                                rgb(0x6c7086)
                            }))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(if active {
                                                gpui::FontWeight::SEMIBOLD
                                            } else {
                                                gpui::FontWeight::NORMAL
                                            })
                                            .child(workspace.name),
                                    )
                                    .child(div().text_xs().text_color(rgb(0x6c7086)).child(
                                        format!(
                                            "{shell_count} {} · {} {}",
                                            if shell_count == 1 { "shell" } else { "shells" },
                                            workspace.agent_count,
                                            if workspace.agent_count == 1 {
                                                "agent"
                                            } else {
                                                "agents"
                                            }
                                        ),
                                    )),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "sidebar-workspace-menu-{}",
                                        workspace.id
                                    )))
                                    .w(px(26.0))
                                    .h(px(30.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_color(rgb(0x7f849c))
                                    .hover(|element| {
                                        element.bg(rgb(0x45475a)).text_color(rgb(0xcdd6f4))
                                    })
                                    .on_click(cx.listener(move |this, event, window, cx| {
                                        this.open_sidebar_menu(
                                            workspace_target.clone(),
                                            event,
                                            window,
                                            cx,
                                        );
                                    }))
                                    .child("⋮"),
                            ),
                    )
                    .when(expanded, |element| element.children(shell_rows))
            })
            .collect::<Vec<_>>();

        let agent_rows = self
            .boomux_overview
            .agents
            .iter()
            .cloned()
            .map(|agent| self.sidebar_agent(agent, focused_shell_id, cx))
            .collect::<Vec<_>>();

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(rgb(0x181825))
            .border_r_1()
            .border_color(if self.navigation_region == NavigationRegion::Sidebar {
                rgb(0xcba6f7)
            } else {
                rgb(0x313244)
            })
            .child(
                div()
                    .h(px(64.0))
                    .flex_none()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(0x313244))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(28.0))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(rgb(0x11111b))
                                    .text_color(rgb(0xf9e2af))
                                    .child("✦"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div().font_weight(gpui::FontWeight::BOLD).child("BOOMUX"),
                                    )
                                    .child(div().text_xs().text_color(rgb(0x89b4fa)).child(
                                        format!(
                                            "active · {} workspaces",
                                            self.boomux_overview.workspaces.len()
                                        ),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("create-workspace")
                            .size(px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x45475a))
                            .cursor_pointer()
                            .text_color(rgb(0xa6adc8))
                            .hover(|button| button.bg(rgb(0x313244)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.create_and_attach_new_workspace(window, cx);
                            }))
                            .child("+"),
                    ),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .track_scroll(&self.sidebar_scroll_handle)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .child(
                        div()
                            .mb_2()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x7f849c))
                            .child("WORKSPACES"),
                    )
                    .when(workspace_rows.is_empty(), |element| {
                        element.child(
                            div()
                                .py_4()
                                .text_sm()
                                .text_color(rgb(0x6c7086))
                                .child("No Boomux workspaces"),
                        )
                    })
                    .children(workspace_rows)
                    .child(div().mt_4().mb_3().h(px(1.0)).w_full().bg(rgb(0x313244)))
                    .child(
                        div()
                            .mb_2()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x7f849c))
                            .child("AGENTS"),
                    )
                    .when(agent_rows.is_empty(), |element| {
                        element.child(
                            div()
                                .py_4()
                                .text_sm()
                                .text_color(rgb(0x6c7086))
                                .child("No active Boomux agents"),
                        )
                    })
                    .children(agent_rows),
            )
    }

    fn sidebar_menu_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let menu = self.sidebar_menu.as_ref()?;
        let target = menu.target.clone();
        let create_workspace_id = match &target {
            SidebarResource::Workspace { id, .. } => Some(id.clone()),
            SidebarResource::Shell { .. } => None,
        };
        let rename_target = target.clone();
        let remove_target = target.clone();

        Some(
            div()
                .id("sidebar-resource-menu")
                .absolute()
                .occlude()
                .left(px(SIDEBAR_WIDTH - 188.0))
                .top(px(menu.top))
                .w(px(178.0))
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(rgb(0x45475a))
                .bg(rgb(0x1e1e2e))
                .shadow_lg()
                .when_some(create_workspace_id, |element, workspace_id| {
                    element.child(
                        div()
                            .id("sidebar-menu-create-shell")
                            .h(px(34.0))
                            .px_3()
                            .flex()
                            .items_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|row| row.bg(rgb(0x313244)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.create_and_attach_workspace_terminal(
                                    workspace_id.clone(),
                                    window,
                                    cx,
                                );
                            }))
                            .child("Create shell"),
                    )
                })
                .child(
                    div()
                        .id("sidebar-menu-rename")
                        .h(px(34.0))
                        .px_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|row| row.bg(rgb(0x313244)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_resource_dialog(
                                ResourceDialogKind::Rename,
                                rename_target.clone(),
                            );
                            cx.notify();
                        }))
                        .child("Rename")
                        .child(div().text_xs().text_color(rgb(0x6c7086)).child("F2")),
                )
                .child(
                    div()
                        .id("sidebar-menu-remove")
                        .h(px(34.0))
                        .px_3()
                        .flex()
                        .items_center()
                        .rounded_md()
                        .text_color(rgb(0xf38ba8))
                        .cursor_pointer()
                        .hover(|row| row.bg(rgb(0x313244)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_resource_dialog(
                                ResourceDialogKind::Remove,
                                remove_target.clone(),
                            );
                            cx.notify();
                        }))
                        .child("Remove"),
                )
                .into_any_element(),
        )
    }

    fn resource_dialog_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let dialog = self.resource_dialog.as_ref()?;
        let kind_label = dialog.target.kind_label();
        let title = match dialog.kind {
            ResourceDialogKind::Rename => format!("Rename {kind_label}"),
            ResourceDialogKind::Remove => format!("Remove {kind_label}?"),
        };
        let detail = match (&dialog.target, dialog.kind) {
            (_, ResourceDialogKind::Rename) => {
                "Type a new name, then press Enter to save it.".to_string()
            }
            (SidebarResource::Shell { name, .. }, ResourceDialogKind::Remove) => format!(
                "This permanently terminates and removes the Boomux Shell “{name}”. Ctrl+W only detaches its pane."
            ),
            (SidebarResource::Workspace { id, name }, ResourceDialogKind::Remove) => {
                let shell_count = self
                    .boomux_overview
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == *id)
                    .map_or(0, |workspace| workspace.shells.len());
                format!(
                    "This permanently removes “{name}” and its {shell_count} Boomux shell{}.",
                    if shell_count == 1 { "" } else { "s" }
                )
            }
        };
        let busy = dialog.busy;
        let confirm_label = match (dialog.kind, busy) {
            (ResourceDialogKind::Rename, false) => "Rename",
            (ResourceDialogKind::Rename, true) => "Renaming…",
            (ResourceDialogKind::Remove, false) => "Remove",
            (ResourceDialogKind::Remove, true) => "Removing…",
        };

        Some(
            div()
                .id("resource-dialog-backdrop")
                .absolute()
                .occlude()
                .left_0()
                .top_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000aa))
                .child(
                    div()
                        .id("resource-dialog")
                        .w(px(440.0))
                        .p_5()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(0x45475a))
                        .bg(rgb(0x1e1e2e))
                        .shadow_lg()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(div().text_sm().text_color(rgb(0xa6adc8)).child(detail))
                        .when(dialog.kind == ResourceDialogKind::Rename, |element| {
                            element.child(
                                div()
                                    .h(px(40.0))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(0x89b4fa))
                                    .bg(rgb(0x11111b))
                                    .child(format!(
                                        "{}{}",
                                        dialog.value,
                                        if busy { "" } else { "▏" }
                                    )),
                            )
                        })
                        .when_some(dialog.error.clone(), |element, error| {
                            element.child(div().text_sm().text_color(rgb(0xf38ba8)).child(error))
                        })
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    div()
                                        .id("resource-dialog-cancel")
                                        .h(px(34.0))
                                        .px_4()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(rgb(0x45475a))
                                        .when(!busy, |button| {
                                            button
                                                .cursor_pointer()
                                                .hover(|hovered| hovered.bg(rgb(0x313244)))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.resource_dialog = None;
                                                    cx.notify();
                                                }))
                                        })
                                        .child("Cancel"),
                                )
                                .child(
                                    div()
                                        .id("resource-dialog-confirm")
                                        .h(px(34.0))
                                        .px_4()
                                        .flex()
                                        .items_center()
                                        .rounded_md()
                                        .bg(rgb(if dialog.kind == ResourceDialogKind::Remove {
                                            0x89384c
                                        } else {
                                            0x365a8c
                                        }))
                                        .when(!busy, |button| {
                                            button
                                                .cursor_pointer()
                                                .hover(|hovered| hovered.opacity(0.85))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    cx.stop_propagation();
                                                    this.submit_resource_dialog(window, cx);
                                                }))
                                        })
                                        .child(confirm_label),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn help_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.help_open {
            return None;
        }
        let sections = if self.navigation_region == NavigationRegion::Sidebar {
            [
                ShortcutSection::Sidebar,
                ShortcutSection::General,
                ShortcutSection::Navigation,
                ShortcutSection::Panes,
                ShortcutSection::Terminal,
            ]
        } else {
            [
                ShortcutSection::General,
                ShortcutSection::Navigation,
                ShortcutSection::Panes,
                ShortcutSection::Terminal,
                ShortcutSection::Sidebar,
            ]
        };
        let section_elements = sections.into_iter().map(|section| {
            let rows = HELP_SHORTCUTS
                .iter()
                .filter(move |shortcut| shortcut.section == section)
                .map(|shortcut| {
                    div()
                        .min_h(px(34.0))
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div().w(px(190.0)).flex_none().child(
                                div()
                                    .flex()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(0x45475a))
                                    .bg(rgb(0x11111b))
                                    .text_xs()
                                    .text_color(rgb(0xcdd6f4))
                                    .child(shortcut.keys),
                            ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .text_sm()
                                .text_color(rgb(0xa6adc8))
                                .child(shortcut.description),
                        )
                })
                .collect::<Vec<_>>();
            div()
                .mb_4()
                .child(
                    div()
                        .mb_2()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(0x89b4fa))
                        .child(section.label()),
                )
                .children(rows)
        });

        Some(
            div()
                .id("help-backdrop")
                .absolute()
                .occlude()
                .left_0()
                .top_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000aa))
                .child(
                    div()
                        .id("help-dialog")
                        .w(px(720.0))
                        .h(relative(0.84))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(0x45475a))
                        .bg(rgb(0x1e1e2e))
                        .shadow_lg()
                        .child(
                            div()
                                .h(px(64.0))
                                .flex_none()
                                .px_5()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_b_1()
                                .border_color(rgb(0x313244))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_lg()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .child("Keyboard shortcuts"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(0x7f849c))
                                                .child("Boomux Desktop"),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("help-close")
                                        .size(px(32.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .text_color(rgb(0xa6adc8))
                                        .hover(|button| button.bg(rgb(0x313244)))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.help_open = false;
                                            cx.notify();
                                        }))
                                        .child("×"),
                                ),
                        )
                        .child(
                            div()
                                .id("help-scroll")
                                .track_scroll(&self.help_scroll_handle)
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px_5()
                                .py_4()
                                .children(section_elements),
                        )
                        .child(
                            div()
                                .h(px(38.0))
                                .flex_none()
                                .px_5()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_t_1()
                                .border_color(rgb(0x313244))
                                .text_xs()
                                .text_color(rgb(0x7f849c))
                                .child("↑/↓ or J/K · Page Up/Down to scroll")
                                .child("Esc or F1 to close"),
                        ),
                )
                .into_any_element(),
        )
    }

    fn sidebar_agent(
        &self,
        agent: AgentChoice,
        focused_shell_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let shell_id = agent.shell_id.clone();
        let agent_item = SidebarItem::Agent {
            agent_id: agent.id.clone(),
            shell_id: agent.shell_id.clone(),
        };
        let keyboard_selected = self.navigation_region == NavigationRegion::Sidebar
            && self.sidebar_item.as_ref() == Some(&agent_item);
        let selected = focused_shell_id == Some(agent.shell_id.as_str());
        let state = agent.state_label();
        let glyph = if agent.needs_attention {
            "!"
        } else if state == "working" {
            "●"
        } else if state == "finished" {
            "✓"
        } else {
            "○"
        };
        let glyph_color = if agent.needs_attention {
            0xf38ba8
        } else if selected || matches!(state, "working" | "finished") {
            0x89b4fa
        } else {
            0x6c7086
        };
        div()
            .id(SharedString::from(format!("sidebar-agent-{}", agent.id)))
            .h(px(56.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .anchor_scroll(keyboard_selected.then(|| self.sidebar_scroll_anchor.clone()))
            .bg(if keyboard_selected {
                rgb(0x45475a)
            } else if selected {
                rgb(0x25283c)
            } else {
                rgb(0x181825)
            })
            .hover(|element| element.bg(rgb(0x29293d)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_sidebar_shell(&shell_id, window, cx);
            }))
            .child(div().w(px(14.0)).text_color(rgb(glyph_color)).child(glyph))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xcdd6f4))
                                    .child(agent.shell_name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x6c7086))
                                    .child(relative_time(agent.updated_at_ms)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if agent.needs_attention {
                                rgb(0xf38ba8)
                            } else {
                                rgb(0x7f849c)
                            })
                            .child(format!(
                                "{} · {} · {}",
                                state, agent.workspace, agent.integration
                            )),
                    ),
            )
    }

    fn boomux_body(&self, pane_id: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let pane = self.terminals.get(&pane_id);
        if pane.and_then(|pane| pane.session.as_ref()).is_some()
            && let Some(screen) = pane.and_then(|pane| pane.screen.as_ref())
        {
            let maximum = screen.scroll_total.saturating_sub(screen.scroll_len);
            let estimated_track_height =
                f32::from(screen.rows) * TERMINAL_CELL_HEIGHT + TERMINAL_PADDING;
            let thumb_fraction = scrollbar_thumb_fraction(screen, estimated_track_height);
            let progress = if maximum == 0 {
                1.0
            } else {
                (screen.scroll_offset as f32 / maximum as f32).clamp(0.0, 1.0)
            };
            let thumb_top = progress * (1.0 - thumb_fraction);
            let scrollbar = div()
                .id(("terminal-scrollbar", pane_id))
                .absolute()
                .right(px(1.0))
                .top(px(2.0))
                .bottom(px(2.0))
                .w(px(10.0))
                .rounded_full()
                .bg(rgb(0x1e1e2e))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_drag(TerminalScrollbarDrag, move |_, _, _, cx| {
                    cx.new(move |_| TerminalScrollbarDrag(pane_id))
                })
                .on_drag_move(cx.listener(Self::drag_terminal_scrollbar))
                .child(
                    div()
                        .absolute()
                        .left(px(2.0))
                        .right(px(2.0))
                        .top(relative(thumb_top))
                        .h(relative(thumb_fraction))
                        .rounded_full()
                        .bg(rgb(if maximum == 0 { 0x313244 } else { 0x6c7086 })),
                );
            return div()
                .relative()
                .size_full()
                .child(terminal_view(
                    Arc::clone(screen),
                    pane.and_then(|pane| pane.selection),
                    screen
                        .image_placements
                        .iter()
                        .filter_map(|placement| {
                            pane.and_then(|pane| {
                                pane.render_images.get(&placement.image_generation).cloned()
                            })
                            .map(|image| (placement.clone(), image))
                        })
                        .collect(),
                ))
                .child(scrollbar)
                .into_any_element();
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .overflow_hidden()
            .child(div().text_xs().text_color(rgb(0xa6adc8)).child(
                if pane.is_some_and(|pane| pane.attaching) {
                    "Opening terminal…"
                } else {
                    "No Boomux terminal is available."
                },
            ))
            .when_some(
                pane.and_then(|pane| pane.error.clone())
                    .or_else(|| self.boomux_error.clone()),
                |element, error| {
                    element.child(div().text_xs().text_color(rgb(0xf38ba8)).child(error))
                },
            )
            .into_any_element()
    }

    fn pane(&self, id: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let focused = self.focused == id;
        let pane = self.terminals.get(&id);
        let title: SharedString = pane.and_then(|pane| pane.session.as_ref()).map_or_else(
            || "Boomux terminals".into(),
            |terminal| terminal.shell_name.clone().into(),
        );
        let accent = rgb(0xa6e3a1);

        div()
            .id(("pane", id))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .border_2()
            .border_color(if focused {
                rgb(0xcba6f7)
            } else {
                rgb(0x313244)
            })
            .bg(rgb(0x181825))
            .shadow_lg()
            .on_mouse_move(cx.listener(move |this, event, window, cx| {
                this.focus_pane_on_hover(id, event, window, cx);
            }))
            .on_scroll_wheel(cx.listener(move |this, event, window, cx| {
                this.scroll_terminal(id, event, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event, window, cx| {
                    this.begin_pointer_interaction(id, event, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event, window, cx| {
                    this.begin_pointer_interaction(id, event, window, cx);
                }),
            )
            .child(
                div()
                    .h(px(38.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .bg(if focused {
                        rgb(0x313244)
                    } else {
                        rgb(0x1e1e2e)
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size_2().rounded_full().bg(accent))
                            .child(title),
                    )
                    .child(div().text_xs().text_color(rgb(0x7f849c)).child(
                        pane.and_then(|pane| pane.session.as_ref()).map_or_else(
                            || "local shells".into(),
                            |terminal| {
                                pane.and_then(|pane| pane.screen.as_ref()).map_or_else(
                                    || terminal.status(),
                                    |screen| {
                                        format!(
                                            "{} · {}×{}",
                                            terminal.status(),
                                            screen.cols,
                                            screen.rows
                                        )
                                    },
                                )
                            },
                        ),
                    )),
            )
            .child(
                div()
                    .id(("terminal-interaction", id))
                    .flex_1()
                    .min_h_0()
                    .on_drag(
                        TerminalSelectionDrag {
                            pane_id: id,
                            started: Arc::new(AtomicBool::new(false)),
                        },
                        move |_, _, _, cx| {
                            cx.new(move |_| TerminalSelectionDrag {
                                pane_id: id,
                                started: Arc::new(AtomicBool::new(true)),
                            })
                        },
                    )
                    .on_drag_move(cx.listener(Self::drag_terminal_selection))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::paste_primary))
                    .child(self.boomux_body(id, cx)),
            )
    }

    fn render_layout(&self, layout: &Node, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panes = layout
            .rects()
            .into_iter()
            .map(|(id, target)| {
                let pane = self.pane(id, cx);
                let base = div().absolute().p_1().child(pane);
                if let Some(animation) = &self.layout_animation {
                    let from = animation.from.get(&id).copied().unwrap_or(target);
                    let animation_id =
                        SharedString::from(format!("layout-reflow-{}-{id}", animation.generation));
                    base.with_animation(
                        animation_id,
                        Animation::new(LAYOUT_ANIMATION_DURATION).with_easing(ease_out_quint()),
                        move |element, progress| {
                            let rect = interpolate_rect(from, target, progress);
                            element
                                .left(relative(rect.x))
                                .top(relative(rect.y))
                                .w(relative(rect.width))
                                .h(relative(rect.height))
                        },
                    )
                    .into_any_element()
                } else {
                    base.left(relative(target.x))
                        .top(relative(target.y))
                        .w(relative(target.width))
                        .h(relative(target.height))
                        .into_any_element()
                }
            })
            .collect::<Vec<_>>();
        div()
            .relative()
            .size_full()
            .children(panes)
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_name = self
            .terminals
            .get(&self.focused)
            .and_then(|pane| pane.shell.as_ref())
            .and_then(|shell| {
                self.boomux_overview
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == shell.workspace_id)
            })
            .map(|workspace| workspace.name.as_str());
        let desktop_title = desktop_window_title(workspace_name);
        if window.window_title() != desktop_title {
            window.set_window_title(&desktop_title);
        }
        let terminal_sizes = self
            .terminals
            .keys()
            .copied()
            .map(|id| (id, self.terminal_grid_size(id, window)))
            .collect::<Vec<_>>();
        for (id, (rows, cols, pixel_width, pixel_height)) in terminal_sizes {
            if let Some(terminal) = self
                .terminals
                .get(&id)
                .and_then(|pane| pane.session.as_ref())
            {
                terminal.resize(rows, cols, pixel_width, pixel_height);
            }
        }
        self.refresh_terminal_images(window);
        let content = if let Some(id) = self.fullscreen {
            div()
                .size_full()
                .child(self.pane(id, cx))
                .into_any_element()
        } else {
            let tiled = if let Some(layout) = &self.layout {
                self.render_layout(layout, cx)
            } else {
                div().size_full().into_any_element()
            };
            let lifted_id = match &self.pointer_drag {
                Some(PointerDrag {
                    pane_id,
                    subject: PointerSubject::Lifted(_),
                    ..
                }) => Some(*pane_id),
                _ => None,
            };
            let floating = self
                .floating
                .clone()
                .into_iter()
                .map(|pane| {
                    div()
                        .absolute()
                        // Floating panes paint above the tiled layout and must
                        // also own the corresponding pointer hitbox. Without
                        // occlusion, both overlapping pane hover handlers run
                        // and the tiled pane behind wins focus last.
                        .occlude()
                        // An occluding hitbox also prevents the workspace's
                        // behind-the-pane move listener from receiving pointer
                        // events. Track an active drag on the floating layer so
                        // every pointer sample updates its geometry smoothly.
                        .on_mouse_move(cx.listener(Self::on_pointer_move))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(Self::end_pointer_interaction),
                        )
                        .on_mouse_up(
                            MouseButton::Right,
                            cx.listener(Self::end_pointer_interaction),
                        )
                        .left(px(pane.x))
                        .top(px(pane.y))
                        .w(px(pane.width))
                        .h(px(pane.height))
                        .when(lifted_id == Some(pane.id), |element| element.opacity(0.92))
                        .child(self.pane(pane.id, cx))
                })
                .collect::<Vec<_>>();

            let terminal_area = div()
                .h_full()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(HEADER_HEIGHT))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_4()
                        .bg(rgb(0x181825))
                        .border_b_1()
                        .border_color(rgb(0x313244))
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(desktop_title.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(0x7f849c))
                                        .child("workspace 1  •  master layout"),
                                )
                                .child(
                                    div()
                                        .id("open-help")
                                        .size(px(28.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(rgb(0x45475a))
                                        .cursor_pointer()
                                        .text_color(rgb(0xa6adc8))
                                        .hover(|button| button.bg(rgb(0x313244)))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.toggle_help(&ToggleHelp, window, cx);
                                        }))
                                        .child("?"),
                                ),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .p_2()
                        .child(tiled)
                        .children(floating),
                )
                .child(
                    div()
                        .h(px(FOOTER_HEIGHT))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_4()
                        .bg(rgb(0x181825))
                        .text_xs()
                        .text_color(rgb(0x7f849c))
                        .child(
                            "F1: help   F6: sidebar   F2: rename   Ctrl+Arrow: focus   Ctrl+Enter: new   Ctrl+W: detach   Ctrl+Shift+W: remove",
                        ),
                );

            div()
                .size_full()
                .flex()
                .child(self.sidebar(cx))
                .child(terminal_area)
                .into_any_element()
        };
        let sidebar_menu = self.sidebar_menu_overlay(cx);
        let resource_dialog = self.resource_dialog_overlay(cx);
        let help = self.help_overlay(cx);

        div()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .key_context(if self.help_open { "Help" } else { "Workspace" })
            .on_action(cx.listener(Self::focus_left))
            .on_action(cx.listener(Self::focus_right))
            .on_action(cx.listener(Self::focus_up))
            .on_action(cx.listener(Self::focus_down))
            .on_action(cx.listener(Self::move_left))
            .on_action(cx.listener(Self::move_right))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::resize_left))
            .on_action(cx.listener(Self::resize_right))
            .on_action(cx.listener(Self::resize_up))
            .on_action(cx.listener(Self::resize_down))
            .on_action(cx.listener(Self::new_pane))
            .on_action(cx.listener(Self::close_pane))
            .on_action(cx.listener(Self::toggle_floating))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::toggle_sidebar_focus))
            .on_action(cx.listener(Self::toggle_help))
            .on_action(cx.listener(Self::rename_resource))
            .on_action(cx.listener(Self::remove_shell))
            .on_action(cx.listener(Self::copy_selection))
            .on_action(cx.listener(Self::paste_clipboard))
            .on_key_down(cx.listener(Self::terminal_key_down))
            .on_mouse_move(cx.listener(Self::on_pointer_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(Self::end_pointer_interaction),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(Self::end_pointer_interaction),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(Self::end_pointer_interaction),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(Self::end_pointer_interaction),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x11111b))
            .text_color(rgb(0xcdd6f4))
            .child(content)
            .when_some(sidebar_menu, |element, menu| element.child(menu))
            .when_some(resource_dialog, |element, dialog| element.child(dialog))
            .when_some(help, |element, help| element.child(help))
    }
}

fn workspace_keystroke(keystroke: &gpui::Keystroke) -> bool {
    if !keystroke.modifiers.secondary() {
        return false;
    }
    (keystroke.modifiers.shift && matches!(keystroke.key.as_str(), "c" | "v"))
        || matches!(
            keystroke.key.as_str(),
            "h" | "j"
                | "k"
                | "l"
                | "left"
                | "right"
                | "up"
                | "down"
                | "enter"
                | "w"
                | "f"
                | "space"
        )
}

fn append_resource_name(value: &mut String, text: &str) {
    const MAX_RESOURCE_NAME_CHARS: usize = 128;
    let remaining = MAX_RESOURCE_NAME_CHARS.saturating_sub(value.chars().count());
    value.extend(
        text.chars()
            .filter(|character| !character.is_control())
            .take(remaining),
    );
}

fn relative_time(timestamp_ms: u64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let seconds = now_ms.saturating_sub(timestamp_ms) / 1_000;
    match seconds {
        0..=59 => "now".into(),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

type RenderedTerminalImage = (TerminalImagePlacement, Arc<RenderImage>);

fn terminal_view(
    screen: Arc<TerminalScreen>,
    selection: Option<TerminalSelection>,
    images: Vec<RenderedTerminalImage>,
) -> Div {
    div().size_full().overflow_hidden().bg(rgb(0x11111b)).child(
        canvas(
            move |bounds, window, _| {
                let cols = usize::from(screen.cols);
                let mut lines = Vec::with_capacity(usize::from(screen.rows));
                let mut backgrounds = Vec::new();
                let mut base_font = font("JetBrainsMono Nerd Font");
                base_font.features = gpui::FontFeatures::disable_ligatures();

                for (row, cells) in screen.cells.chunks(cols).enumerate() {
                    let mut text = String::new();
                    let mut runs = Vec::with_capacity(cells.len());
                    for (col, cell) in cells.iter().enumerate() {
                        let selected = selection.is_some_and(|selection| {
                            let (start, end) = selection_indices(selection, cols);
                            let index = row * cols + col;
                            (start..=end).contains(&index)
                        });
                        let (foreground, background) = if selected {
                            (0xcdd6f4, 0x45475a)
                        } else if cell.cursor {
                            (0x1e1e2e, 0xcba6f7)
                        } else {
                            (cell.foreground, cell.background)
                        };
                        if background != 0x11111b {
                            backgrounds.push(fill(
                                Bounds::new(
                                    point(
                                        bounds.left() + px(8.0 + col as f32 * TERMINAL_CELL_WIDTH),
                                        bounds.top() + px(8.0 + row as f32 * TERMINAL_CELL_HEIGHT),
                                    ),
                                    size(px(TERMINAL_CELL_WIDTH), px(TERMINAL_CELL_HEIGHT)),
                                ),
                                rgb(background),
                            ));
                        }
                        let start = text.len();
                        // Keep one shaped glyph slot for every terminal cell.
                        // A wide character paints from its leading cell; its
                        // continuation is represented by an inkless space so
                        // subsequent glyphs still land on the correct column.
                        if cell.continuation {
                            text.push(' ');
                        } else {
                            text.push_str(&cell.text);
                        }
                        let mut cell_font = base_font.clone();
                        if cell.bold {
                            cell_font = cell_font.bold();
                        }
                        if cell.italic {
                            cell_font = cell_font.italic();
                        }
                        runs.push(TextRun {
                            len: text.len() - start,
                            font: cell_font,
                            color: rgb_to_hsla(rgb(foreground)),
                            underline: cell.underline.then_some(UnderlineStyle {
                                thickness: px(1.0),
                                color: Some(rgb_to_hsla(rgb(foreground))),
                                wavy: false,
                            }),
                            ..Default::default()
                        });
                    }
                    lines.push(window.text_system().shape_line(
                        text.into(),
                        px(13.0),
                        &runs,
                        Some(px(TERMINAL_CELL_WIDTH)),
                    ));
                }
                (lines, backgrounds, images)
            },
            move |bounds, (lines, backgrounds, images), window, cx| {
                paint_terminal_images(bounds, &images, |z| z < i32::MIN / 2, window);
                for background in backgrounds {
                    window.paint_quad(background);
                }
                paint_terminal_images(bounds, &images, |z| (i32::MIN / 2..0).contains(&z), window);
                for (row, line) in lines.iter().enumerate() {
                    let origin = point(
                        bounds.left() + px(8.0),
                        bounds.top() + px(8.0 + row as f32 * TERMINAL_CELL_HEIGHT),
                    );
                    let _ = line.paint(
                        origin,
                        px(TERMINAL_CELL_HEIGHT),
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                paint_terminal_images(bounds, &images, |z| z >= 0, window);
            },
        )
        .size_full(),
    )
}

fn paint_terminal_images(
    terminal_bounds: Bounds<gpui::Pixels>,
    images: &[RenderedTerminalImage],
    layer: impl Fn(i32) -> bool,
    window: &mut Window,
) {
    for (placement, image) in images {
        if !layer(placement.z)
            || placement.source_width == 0
            || placement.source_height == 0
            || placement.cell_width == 0
            || placement.cell_height == 0
        {
            continue;
        }

        let cell_scale_x = TERMINAL_CELL_WIDTH / placement.cell_width as f32;
        let cell_scale_y = TERMINAL_CELL_HEIGHT / placement.cell_height as f32;
        let destination = Bounds::new(
            point(
                terminal_bounds.left()
                    + px(8.0
                        + placement.viewport_col as f32 * TERMINAL_CELL_WIDTH
                        + placement.x_offset as f32 * cell_scale_x),
                terminal_bounds.top()
                    + px(8.0
                        + placement.viewport_row as f32 * TERMINAL_CELL_HEIGHT
                        + placement.y_offset as f32 * cell_scale_y),
            ),
            size(
                px(placement.pixel_width as f32 * cell_scale_x),
                px(placement.pixel_height as f32 * cell_scale_y),
            ),
        );
        let image_scale_x = f32::from(destination.size.width) / placement.source_width as f32;
        let image_scale_y = f32::from(destination.size.height) / placement.source_height as f32;
        let image_size = image.size(0);
        let image_bounds = Bounds::new(
            point(
                destination.left() - px(placement.source_x as f32 * image_scale_x),
                destination.top() - px(placement.source_y as f32 * image_scale_y),
            ),
            size(
                px(image_size.width.0 as f32 * image_scale_x),
                px(image_size.height.0 as f32 * image_scale_y),
            ),
        );
        let clip = terminal_bounds.intersect(&destination);
        let _ = window.paint_image(
            clip,
            image_bounds,
            Corners::default(),
            Arc::clone(image),
            0,
            false,
        );
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("secondary-h", FocusLeft, Some("Workspace")),
            KeyBinding::new("secondary-l", FocusRight, Some("Workspace")),
            KeyBinding::new("secondary-k", FocusUp, Some("Workspace")),
            KeyBinding::new("secondary-j", FocusDown, Some("Workspace")),
            KeyBinding::new("secondary-left", FocusLeft, Some("Workspace")),
            KeyBinding::new("secondary-right", FocusRight, Some("Workspace")),
            KeyBinding::new("secondary-up", FocusUp, Some("Workspace")),
            KeyBinding::new("secondary-down", FocusDown, Some("Workspace")),
            KeyBinding::new("secondary-shift-h", MoveLeft, Some("Workspace")),
            KeyBinding::new("secondary-shift-l", MoveRight, Some("Workspace")),
            KeyBinding::new("secondary-shift-k", MoveUp, Some("Workspace")),
            KeyBinding::new("secondary-shift-j", MoveDown, Some("Workspace")),
            KeyBinding::new("secondary-shift-left", MoveLeft, Some("Workspace")),
            KeyBinding::new("secondary-shift-right", MoveRight, Some("Workspace")),
            KeyBinding::new("secondary-shift-up", MoveUp, Some("Workspace")),
            KeyBinding::new("secondary-shift-down", MoveDown, Some("Workspace")),
            KeyBinding::new("secondary-alt-h", ResizeLeft, Some("Workspace")),
            KeyBinding::new("secondary-alt-l", ResizeRight, Some("Workspace")),
            KeyBinding::new("secondary-alt-k", ResizeUp, Some("Workspace")),
            KeyBinding::new("secondary-alt-j", ResizeDown, Some("Workspace")),
            KeyBinding::new(KEY_NEW_PANE, NewPane, Some("Workspace")),
            KeyBinding::new(KEY_REMOVE_SHELL, RemoveShell, Some("Workspace")),
            KeyBinding::new(KEY_DETACH_PANE, ClosePane, Some("Workspace")),
            KeyBinding::new(KEY_TOGGLE_FLOATING, ToggleFloating, Some("Workspace")),
            KeyBinding::new(KEY_TOGGLE_FULLSCREEN, ToggleFullscreen, Some("Workspace")),
            KeyBinding::new(KEY_TOGGLE_HELP, ToggleHelp, Some("Workspace")),
            KeyBinding::new(KEY_TOGGLE_HELP, ToggleHelp, Some("Help")),
            KeyBinding::new(KEY_TOGGLE_SIDEBAR, ToggleSidebarFocus, Some("Workspace")),
            KeyBinding::new(KEY_RENAME_RESOURCE, RenameResource, Some("Workspace")),
            KeyBinding::new("secondary-shift-c", CopySelection, Some("Workspace")),
            KeyBinding::new("secondary-shift-v", PasteClipboard, Some("Workspace")),
            // Omarchy's universal Super+C / Super+V bindings translate terminal
            // clipboard actions to these conventional terminal chords.
            KeyBinding::new("ctrl-insert", CopySelection, Some("Workspace")),
            KeyBinding::new("shift-insert", PasteClipboard, Some("Workspace")),
        ]);

        let bounds = Bounds::centered(None, gpui::size(px(1180.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // Omarchy tags org.omarchy.* windows as terminals, which makes
                // its universal clipboard binding choose Ctrl/Shift+Insert.
                app_id: Some("org.omarchy.boomux-desktop".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod pointer_tests {
    use super::*;
    use boomux::protocol::{AgentState, ShellStatus};

    fn pane() -> FloatingPane {
        FloatingPane {
            id: 1,
            x: 100.0,
            y: 80.0,
            width: 400.0,
            height: 240.0,
        }
    }

    #[test]
    fn pointer_coordinates_use_the_terminal_panel_origin() {
        assert_eq!(window_point_to_panel(425.0, 92.0), (125.0, 50.0));
    }

    #[test]
    fn window_title_includes_the_focused_workspace() {
        assert_eq!(
            desktop_window_title(Some("lively-dolphin")),
            "Boomux Desktop — lively-dolphin"
        );
        assert_eq!(desktop_window_title(None), "Boomux Desktop");
    }

    #[test]
    fn layout_rects_interpolate_without_overshoot() {
        let from = Rect {
            x: 0.5,
            y: 0.25,
            width: 0.5,
            height: 0.75,
        };
        let to = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        assert_eq!(interpolate_rect(from, to, 0.0).x, 0.5);
        let middle = interpolate_rect(from, to, 0.5);
        assert_eq!(middle.x, 0.25);
        assert_eq!(middle.width, 0.75);
        assert_eq!(interpolate_rect(from, to, 1.0).width, 1.0);
    }

    #[test]
    fn keyboard_swap_animates_each_pane_from_its_previous_rect() {
        let mut layout = Node::pane(1);
        layout.split(1, 2, Axis::Horizontal);
        let before = layout.rects().into_iter().collect::<HashMap<_, _>>();

        let animation = swap_layout_direction(&mut layout, 1, Direction::Right).unwrap();
        let after = layout.rects().into_iter().collect::<HashMap<_, _>>();

        assert_eq!(animation.get(&1).unwrap().x, before.get(&1).unwrap().x);
        assert_eq!(animation.get(&2).unwrap().x, before.get(&2).unwrap().x);
        assert_eq!(after.get(&1).unwrap().x, before.get(&2).unwrap().x);
        assert_eq!(after.get(&2).unwrap().x, before.get(&1).unwrap().x);
    }

    #[test]
    fn terminal_selection_extracts_rows_in_either_drag_direction() {
        let cells = "abc efg "
            .chars()
            .map(|character| terminal::TerminalCell {
                text: character.to_string(),
                foreground: 0xffffff,
                background: 0,
                bold: false,
                italic: false,
                underline: false,
                wide: false,
                continuation: false,
                cursor: false,
            })
            .collect();
        let screen = TerminalScreen {
            rows: 2,
            cols: 4,
            cells,
            scroll_total: 2,
            scroll_offset: 0,
            scroll_len: 2,
            images: Vec::new(),
            image_placements: Vec::new(),
        };
        let selection = TerminalSelection {
            anchor: (1, 1),
            head: (0, 1),
        };
        assert_eq!(terminal_selected_text(&screen, selection), "bc\nef");
    }

    #[test]
    fn moving_is_clamped_to_panel() {
        let moved = dragged_bounds(
            pane(),
            PointerOperation::Move,
            (900.0, -900.0),
            (800.0, 600.0),
        );
        assert_eq!(moved.x, 400.0);
        assert_eq!(moved.y, 0.0);
    }

    #[test]
    fn resizing_respects_minimum_and_panel_edge() {
        let small = dragged_bounds(
            pane(),
            PointerOperation::Resize,
            (-900.0, -900.0),
            (800.0, 600.0),
        );
        assert_eq!(small.width, MIN_FLOAT_WIDTH);
        assert_eq!(small.height, MIN_FLOAT_HEIGHT);

        let large = dragged_bounds(
            pane(),
            PointerOperation::Resize,
            (900.0, 900.0),
            (800.0, 600.0),
        );
        assert_eq!(large.width, 700.0);
        assert_eq!(large.height, 520.0);
    }

    #[test]
    fn tiled_resize_applies_both_pointer_axes_at_once() {
        let mut layout = Node::pane(1);
        layout.split(1, 2, Axis::Horizontal);
        layout.split(2, 3, Axis::Vertical);

        let resized = resized_tiled_layout(layout, 3, (120.0, -120.0), (1200.0, 600.0));
        let pane = resized
            .rects()
            .into_iter()
            .find(|(id, _)| *id == 3)
            .unwrap()
            .1;

        assert!((pane.width - 0.4).abs() < f32::EPSILON);
        assert!((pane.y - 0.3).abs() < f32::EPSILON);
        assert!((pane.height - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn drop_side_selects_insertion_axis_and_order() {
        let mut layout = Node::pane(1);
        layout.split(1, 2, Axis::Horizontal);

        assert_eq!(
            drop_placement(&layout, (0.05, 0.5)),
            Some((1, Axis::Horizontal, true))
        );
        assert_eq!(
            drop_placement(&layout, (0.75, 0.95)),
            Some((2, Axis::Vertical, false))
        );
    }

    fn sidebar_overview() -> BoomuxOverview {
        let shell = ShellChoice {
            id: "shell-1".into(),
            name: "lively-dolphin".into(),
            workspace_id: "workspace-1".into(),
            cwd: "/tmp".into(),
            status: ShellStatus::Running,
            run_id: Some("run-1".into()),
        };
        BoomuxOverview {
            workspaces: vec![terminal::WorkspaceChoice {
                id: "workspace-1".into(),
                name: "boomux-desktop".into(),
                shells: vec![shell],
                agent_count: 1,
            }],
            agents: vec![AgentChoice {
                id: "agent-1".into(),
                shell_name: "lively-dolphin".into(),
                workspace: "boomux-desktop".into(),
                shell_id: "shell-1".into(),
                integration: "codex".into(),
                state: AgentState::Idle,
                updated_at_ms: 1,
                needs_attention: false,
            }],
            focused_shell_id: Some("shell-1".into()),
        }
    }

    #[test]
    fn sidebar_navigation_contains_only_visible_tree_rows() {
        let overview = sidebar_overview();
        let collapsed = visible_sidebar_items(&overview, &HashSet::new());
        assert_eq!(
            collapsed,
            vec![
                SidebarItem::Workspace("workspace-1".into()),
                SidebarItem::Agent {
                    agent_id: "agent-1".into(),
                    shell_id: "shell-1".into(),
                },
            ]
        );

        let expanded =
            visible_sidebar_items(&overview, &HashSet::from([String::from("workspace-1")]));
        assert!(matches!(
            &expanded[1],
            SidebarItem::Shell {
                workspace_id,
                shell_id,
            } if workspace_id == "workspace-1" && shell_id == "shell-1"
        ));
    }

    #[test]
    fn sidebar_selection_survives_refresh_by_identity() {
        let selected = SidebarItem::Agent {
            agent_id: "agent-1".into(),
            shell_id: "shell-1".into(),
        };
        let visible = vec![
            SidebarItem::Workspace("workspace-1".into()),
            selected.clone(),
        ];
        assert_eq!(
            reconciled_sidebar_item(Some(&selected), None, &visible),
            Some(selected)
        );
    }

    #[test]
    fn sidebar_selection_falls_back_to_preferred_visible_item() {
        let stale = SidebarItem::Workspace("removed".into());
        let preferred = SidebarItem::Workspace("workspace-1".into());
        let visible = vec![preferred.clone()];
        assert_eq!(
            reconciled_sidebar_item(Some(&stale), Some(&preferred), &visible),
            Some(preferred)
        );
    }

    #[test]
    fn sidebar_rows_resolve_their_exact_workspace_for_shell_creation() {
        let overview = sidebar_overview();
        for item in [
            SidebarItem::Workspace("workspace-1".into()),
            SidebarItem::Shell {
                workspace_id: "workspace-1".into(),
                shell_id: "shell-1".into(),
            },
            SidebarItem::Agent {
                agent_id: "agent-1".into(),
                shell_id: "shell-1".into(),
            },
        ] {
            assert_eq!(
                workspace_id_for_sidebar_item(&item, &overview).as_deref(),
                Some("workspace-1")
            );
        }

        assert_eq!(
            workspace_id_for_sidebar_item(
                &SidebarItem::Shell {
                    workspace_id: "workspace-1".into(),
                    shell_id: "stale-shell".into(),
                },
                &overview,
            ),
            None
        );
    }

    #[test]
    fn resource_names_reject_controls_and_have_a_fixed_bound() {
        let mut name = String::from("shell-");
        append_resource_name(&mut name, "one\n\ttwo");
        assert_eq!(name, "shell-onetwo");

        append_resource_name(&mut name, &"x".repeat(256));
        assert_eq!(name.chars().count(), 128);
    }

    #[test]
    fn help_catalog_covers_every_shortcut_section() {
        for section in [
            ShortcutSection::General,
            ShortcutSection::Navigation,
            ShortcutSection::Panes,
            ShortcutSection::Terminal,
            ShortcutSection::Sidebar,
        ] {
            assert!(
                HELP_SHORTCUTS
                    .iter()
                    .any(|shortcut| shortcut.section == section),
                "missing help shortcuts for {}",
                section.label()
            );
        }
    }
}
