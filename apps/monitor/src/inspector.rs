use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use unbound_toolkit::ui::{
    Button, Cell, Column, Cursor, Event, Key, Row, SEGMENTED_HEIGHT, SegmentedControl, Table,
    WidgetId,
};
use unbound_toolkit::{Appearance, Header, Palette, Rect, SHEET_INSET, Scene, TextSystem};

use crate::L10N;
use crate::format::{format_bytes, format_cpu_time, format_percent};
use crate::inspect::{Details, read_details, state_name};
use crate::list::{List, Signal, sender};
use crate::model::StopTarget;
use crate::sampler::{Process, Snapshot};
use crate::view::{
    ALERT_ICON, FORCE_QUIT, Icons, QUIT, Request, Screen, answer_stop, stop_alert, stop_button,
};

pub const INSPECTOR_SIZE: (u32, u32) = (680, 520);
const HEADER: f32 = unbound_toolkit::WINDOW_HEADER_HEIGHT as f32;
const SIDE: f32 = 12.0;
const LEADING: f32 = 94.0;
const BUTTON_SIZE: f32 = 28.0;
const ITEM_GAP: f32 = 16.0;
const SEGMENT_WIDTH: f32 = 96.0;
const LABEL_WIDTH: f32 = 190.0;
const PARENT: &str = "parent:";
const CHILD: &str = "child:";
const FILE: &str = "file:";
const SECTION: &str = "section:";
const EXECUTABLE: &str = "executable";

fn find_row<'a>(rows: &'a [Row], key: &str) -> Option<&'a Row> {
    rows.iter().find_map(|row| match row.key == key {
        true => Some(row),
        false => find_row(&row.children, key),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Info,
    Threads,
    Files,
    Memory,
}

impl Section {
    pub const ALL: [Section; 4] = [
        Section::Info,
        Section::Threads,
        Section::Files,
        Section::Memory,
    ];

    fn title(self) -> String {
        L10N.tr(match self {
            Section::Info => "inspector-info",
            Section::Threads => "inspector-threads",
            Section::Files => "inspector-files",
            Section::Memory => "inspector-memory",
        })
    }

    fn columns(self) -> Vec<Column> {
        let column = |message: &str, width: f32| Column::new(&L10N.tr(message), width);
        match self {
            Section::Info => vec![
                column("column-name", LABEL_WIDTH).unsortable(),
                column("inspector-value", 200.0).unsortable().flexible(),
            ],
            Section::Threads => vec![
                column("inspector-tid", 72.0).numeric(),
                column("column-name", 200.0).flexible(),
                column("inspector-state", 140.0),
                column("column-cpu-time", 96.0).numeric(),
            ],
            Section::Files => vec![
                column("inspector-fd", 56.0).numeric(),
                column("inspector-path", 300.0).flexible(),
            ],
            Section::Memory => vec![
                column("inspector-address", 220.0),
                column("inspector-size", 88.0).numeric(),
                column("inspector-permissions", 64.0),
                column("inspector-mapping", 200.0).flexible(),
            ],
        }
    }
}

struct Family {
    parent: Option<(u32, String)>,
    children: Vec<(u32, String)>,
}

pub struct InspectorView {
    pid: u32,
    start_ticks: u64,
    name: String,
    root: PathBuf,
    ticks_per_second: f64,
    process: Option<Process>,
    family: Family,
    details: Details,
    section: Section,
    palette: Palette,
    icons: Icons,
    list: List,
    stop: WidgetId,
    sections: WidgetId,
    size: (f32, f32),
    full_screen: bool,
    stopping: Option<StopTarget>,
    requests: Vec<Request>,
    finished: bool,
    expanded: HashSet<String>,
}

fn named(pid: u32, name: &str) -> String {
    L10N.tr_args(
        "inspector-process",
        [("name", name.to_owned().into()), ("pid", pid.into())],
    )
}

impl InspectorView {
    pub fn new(
        process: &Process,
        root: &Path,
        ticks_per_second: u64,
        palette: Palette,
        text: TextSystem,
        icons: Icons,
    ) -> Self {
        let table = Table::new(Section::Info.columns()).without_header();
        let mut list = List::new(table, palette, text);
        let send = sender(list.signals());
        let stop = list.insert_in_header(stop_button(move || send(Signal::Stop)));
        let send = sender(list.signals());
        let titles = Section::ALL.map(Section::title);
        let sections = list.insert_in_header(SegmentedControl::new(
            &L10N.tr("tab-label"),
            &titles,
            Some(0),
            move |index| send(Signal::Tab(index)),
        ));
        let mut view = Self {
            pid: process.pid,
            start_ticks: process.start_ticks,
            name: process.identity.name.clone(),
            root: root.to_path_buf(),
            ticks_per_second: ticks_per_second as f64,
            process: Some(process.clone()),
            family: Family {
                parent: None,
                children: Vec::new(),
            },
            details: Details::default(),
            section: Section::Info,
            palette,
            icons,
            list,
            stop,
            sections,
            size: (0.0, 0.0),
            full_screen: false,
            stopping: None,
            requests: Vec::new(),
            finished: false,
            expanded: HashSet::new(),
        };
        view.refresh();
        view
    }

    pub fn title(&self) -> String {
        named(self.pid, &self.name)
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn table(&self) -> Option<&Table> {
        self.list.shown_table()
    }

    fn refresh(&mut self) {
        if self.process.is_some()
            && let Some(details) = read_details(&self.root, self.pid, self.ticks_per_second)
        {
            self.details = details;
        }
        let ended = self.process.is_none();
        self.list
            .update::<Button>(self.stop, |button, _| button.set_enabled(!ended));
        let rows = self.rows();
        if let Some(table) = self.list.table() {
            table.set_rows(rows.clone());
        }
        self.expand_sections(&rows);
        self.list.take_signals();
    }

    fn info_rows(&self) -> Vec<Row> {
        let row = |key: &str, label: &str, value: String| {
            Row::new(
                key,
                vec![
                    Cell::new(L10N.tr(label)).muted().label(),
                    Cell::new(value).tabular(),
                ],
            )
        };
        let section = |key: &str, title: &str, children: Vec<Row>| {
            Row::new(
                format!("{SECTION}{key}"),
                vec![Cell::new(L10N.tr(title)).muted().label(), Cell::new("")],
            )
            .children(children)
        };
        let state = match &self.process {
            Some(_) => state_name(self.details.state),
            None => L10N.tr("state-ended"),
        };
        let mut process_rows = vec![
            row("name", "column-name", self.name.clone()),
            row("pid", "column-pid", self.pid.to_string()),
            row("state", "inspector-state", state),
        ];
        let mut family = Vec::new();
        if let Some((pid, name)) = &self.family.parent {
            family.push(row(
                &format!("{PARENT}{pid}"),
                "inspector-parent",
                named(*pid, name),
            ));
        }
        for (index, (pid, name)) in self.family.children.iter().enumerate() {
            let label = match index {
                0 => L10N.tr("inspector-children"),
                _ => String::new(),
            };
            family.push(Row::new(
                format!("{CHILD}{pid}"),
                vec![
                    Cell::new(label).muted().label(),
                    Cell::new(named(*pid, name)).tabular(),
                ],
            ));
        }
        let Some(process) = &self.process else {
            return vec![section(
                "process",
                "inspector-section-process",
                process_rows,
            )];
        };
        let identity = &process.identity;
        process_rows.push(row("user", "column-user", identity.user.clone()));
        if let Some(unit) = &identity.unit {
            process_rows.push(row("unit", "inspector-unit", unit.clone()));
        }
        if let Some(reason) = &process.preventing_sleep {
            process_rows.push(row("sleep", "inspector-preventing-sleep", reason.clone()));
        }
        let mut command = vec![row(
            "command",
            "inspector-command",
            identity.command.join(" "),
        )];
        if let Some(executable) = &identity.executable {
            command.push(row(
                EXECUTABLE,
                "inspector-executable",
                executable.display().to_string(),
            ));
        }
        let mut resources = vec![
            row(
                "cpu",
                "column-cpu",
                format_percent(f64::from(process.cpu_percent)),
            ),
            row(
                "cpu-time",
                "column-cpu-time",
                format_cpu_time(process.cpu_seconds),
            ),
            row("threads", "column-threads", process.threads.to_string()),
            row(
                "resident",
                "column-resident",
                format_bytes(process.resident),
            ),
        ];
        if let Some(proportional) = process.proportional {
            resources.push(row("memory", "column-memory", format_bytes(proportional)));
        }
        if let Some(size) = self.details.virtual_size {
            resources.push(row("virtual", "inspector-virtual", format_bytes(size)));
        }
        if let Some(seconds) = self.details.running_seconds {
            resources.push(row(
                "running",
                "inspector-running",
                format_cpu_time(seconds),
            ));
        }
        if let Some(disk) = &process.disk {
            resources.push(row(
                "read",
                "column-read-total",
                format_bytes(disk.read_total),
            ));
            resources.push(row(
                "written",
                "column-written-total",
                format_bytes(disk.written_total),
            ));
        }
        let mut sections = vec![section(
            "process",
            "inspector-section-process",
            process_rows,
        )];
        if !family.is_empty() {
            sections.push(section("family", "inspector-section-family", family));
        }
        sections.push(section("command", "inspector-section-command", command));
        sections.push(section(
            "resources",
            "inspector-section-resources",
            resources,
        ));
        sections
    }

    fn expand_sections(&mut self, rows: &[Row]) {
        let Some(table) = self.list.table() else {
            return;
        };
        for row in rows {
            if !row.children.is_empty() && self.expanded.insert(row.key.clone()) {
                table.set_row_expanded(&row.key, true);
            }
        }
    }

    fn copy_selection(&mut self) -> bool {
        let Some(selection) = self.list.shown_table().map(Table::selection) else {
            return false;
        };
        let rows = self.rows();
        let values = selection
            .iter()
            .filter_map(|key| find_row(&rows, key))
            .filter_map(|row| match self.section {
                Section::Info => row.cells.get(1),
                _ => row.cells.iter().find(|cell| !cell.text.is_empty()),
            })
            .map(|cell| cell.text.clone())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>();
        if values.is_empty() {
            return false;
        }
        self.requests.push(Request::Copy(values.join("\n")));
        true
    }

    fn rows(&self) -> Vec<Row> {
        let unavailable = || {
            vec![Row::new(
                "unavailable",
                vec![
                    Cell::new(""),
                    Cell::new(L10N.tr("inspector-not-permitted")).muted(),
                ],
            )]
        };
        match self.section {
            Section::Info => self.info_rows(),
            Section::Threads => self
                .details
                .threads
                .iter()
                .map(|thread| {
                    Row::new(
                        thread.tid.to_string(),
                        vec![
                            Cell::new(thread.tid.to_string()),
                            Cell::new(thread.name.as_str()),
                            Cell::new(state_name(thread.state)),
                            Cell::new(format_cpu_time(thread.cpu_seconds)),
                        ],
                    )
                })
                .collect(),
            Section::Files => match &self.details.files {
                Some(files) => files
                    .iter()
                    .map(|file| {
                        Row::new(
                            format!("{FILE}{}", file.fd),
                            vec![
                                Cell::new(file.fd.to_string()),
                                Cell::new(file.target.as_str()),
                            ],
                        )
                    })
                    .collect(),
                None => unavailable(),
            },
            Section::Memory => match &self.details.maps {
                Some(maps) => maps
                    .iter()
                    .enumerate()
                    .map(|(index, map)| {
                        Row::new(
                            index.to_string(),
                            vec![
                                Cell::new(format!("{:012x}–{:012x}", map.start, map.end)),
                                Cell::new(format_bytes(map.end.saturating_sub(map.start))),
                                Cell::new(map.permissions.as_str()),
                                Cell::new(map.path.as_str()),
                            ],
                        )
                    })
                    .collect(),
                None => unavailable(),
            },
        }
    }

    fn set_section(&mut self, section: Section) {
        if let Some(index) = Section::ALL.iter().position(|shown| *shown == section) {
            self.list
                .update::<SegmentedControl>(self.sections, |control, cx| {
                    control.set_selected(cx, index)
                });
        }
        if section == self.section {
            return;
        }
        self.section = section;
        let rows = self.rows();
        if let Some(table) = self.list.table() {
            table.set_columns(section.columns());
            table.set_rows(rows.clone());
        }
        self.expanded.clear();
        self.expand_sections(&rows);
    }

    fn activate(&mut self, keys: &[String]) {
        for key in keys {
            if key == EXECUTABLE
                && let Some(executable) = self
                    .process
                    .as_ref()
                    .and_then(|process| process.identity.executable.clone())
            {
                self.requests.push(Request::ShowInFiles(executable));
            }
            if let Some(pid) = key
                .strip_prefix(PARENT)
                .or_else(|| key.strip_prefix(CHILD))
                .and_then(|pid| pid.parse().ok())
            {
                self.requests.push(Request::Inspect(pid));
            }
            if let Some(file) = key
                .strip_prefix(FILE)
                .and_then(|fd| fd.parse::<u32>().ok())
                .and_then(|fd| {
                    self.details
                        .files
                        .iter()
                        .flatten()
                        .find(|file| file.fd == fd)
                })
                .filter(|file| file.target.starts_with('/'))
            {
                self.requests
                    .push(Request::ShowInFiles(PathBuf::from(&file.target)));
            }
        }
    }

    fn open_stop_dialog(&mut self) {
        if self.process.is_none() {
            return;
        }
        let id = (self.pid, self.start_ticks);
        let target = StopTarget {
            name: self.name.clone(),
            count: 1,
            roots: vec![id],
            apps: Vec::new(),
            processes: vec![id],
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let alert = stop_alert(&target, false, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        self.stopping = Some(target);
        self.list.present_dialog(alert);
    }

    fn chosen(&mut self, choice: &str) {
        let Some(target) = self.stopping.take() else {
            return;
        };
        let icon = self.icons.get(ALERT_ICON);
        let send = sender(self.list.signals());
        let denied = answer_stop(&target, choice, icon, move |choice| {
            send(Signal::Choice(choice.to_owned()))
        });
        if matches!(choice, QUIT | FORCE_QUIT) {
            self.requests.push(Request::Resample);
        }
        if let Some(alert) = denied {
            self.list.present_dialog(alert);
        }
        self.list.focus();
    }

    fn drain(&mut self) {
        loop {
            let signals = self.list.take_signals();
            if signals.is_empty() {
                break;
            }
            for signal in signals {
                match signal {
                    Signal::Activate(keys) => self.activate(&keys),
                    Signal::Tab(index) => {
                        if let Some(section) = Section::ALL.get(index) {
                            self.set_section(*section);
                        }
                    }
                    Signal::Stop => self.open_stop_dialog(),
                    Signal::Choice(choice) => self.chosen(&choice),
                    Signal::Selection(_)
                    | Signal::Sort(_)
                    | Signal::Search(_)
                    | Signal::ClearSearch
                    | Signal::Inspect => {}
                }
            }
        }
        let requests = self.list.take_requests();
        self.requests.extend(requests);
    }

    fn pass(&mut self, event: &Event) -> bool {
        let handled = self.list.handle(event);
        self.drain();
        handled
    }

    fn inset(&self) -> f32 {
        match self.full_screen {
            true => 0.0,
            false => SHEET_INSET as f32,
        }
    }

    fn toolbar_frames(&self) -> [(WidgetId, Rect); 2] {
        let middle = |height: f32| ((HEADER - height) / 2.0).round();
        let width = SEGMENT_WIDTH * Section::ALL.len() as f32;
        let sections = Rect::new(
            self.size.0 - SIDE - width,
            middle(SEGMENTED_HEIGHT),
            width,
            SEGMENTED_HEIGHT,
        );
        let stop = Rect::new(
            sections.x - ITEM_GAP - BUTTON_SIZE,
            middle(BUTTON_SIZE),
            BUTTON_SIZE,
            BUTTON_SIZE,
        );
        [(self.stop, stop), (self.sections, sections)]
    }
}

impl Screen for InspectorView {
    fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.size = (width as f32, height as f32);
        for (id, frame) in self.toolbar_frames() {
            self.list.place(id, frame);
        }
        let inset = self.inset();
        self.list.set_frame(Rect::new(
            inset,
            HEADER,
            (self.size.0 - 2.0 * inset).max(0.0),
            (self.size.1 - HEADER - inset).max(0.0),
        ));
        let (mut items, dialog) = self.list.paint(width, height);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items,
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.list.set_appearance(appearance);
    }

    fn set_focused(&mut self, focused: bool) {
        match focused {
            true if self.list.focused().is_none() => self.list.focus(),
            true => {}
            false => self.list.blur(),
        }
    }

    fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } if !self.list.has_dialog() => match key {
                Key::Escape => {
                    self.finished = true;
                    true
                }
                Key::Character('w') if modifiers.logo => {
                    self.finished = true;
                    true
                }
                Key::Character('c') if modifiers.logo => self.copy_selection(),
                Key::Character(digit @ '1'..='4') if modifiers.logo => {
                    let index = *digit as usize - '1' as usize;
                    if let Some(section) = Section::ALL.get(index) {
                        self.set_section(*section);
                    }
                    true
                }
                _ => self.pass(event),
            },
            _ => self.pass(event),
        }
    }

    fn update(&mut self, snapshot: Snapshot) {
        let pid = self.pid;
        let start_ticks = self.start_ticks;
        self.process = snapshot
            .processes
            .iter()
            .find(|process| process.pid == pid && process.start_ticks == start_ticks)
            .cloned();
        if let Some(process) = &self.process {
            self.family = Family {
                parent: snapshot
                    .processes
                    .iter()
                    .find(|parent| parent.pid == process.ppid)
                    .map(|parent| (parent.pid, parent.identity.name.clone())),
                children: snapshot
                    .processes
                    .iter()
                    .filter(|child| child.ppid == pid)
                    .map(|child| (child.pid, child.identity.name.clone()))
                    .collect(),
            };
        }
        self.refresh();
    }

    fn measures_proportional_memory(&self) -> bool {
        false
    }

    fn measures_sleep_blockers(&self) -> bool {
        true
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    fn menu_action(&mut self, action: &str) -> bool {
        let handled = self.list.menu_action(action);
        self.drain();
        handled
    }

    fn menu_closed(&mut self) -> bool {
        self.list.menu_closed()
    }

    fn pasted(&mut self, text: &str) -> bool {
        let pasted = self.list.pasted(text);
        self.drain();
        pasted
    }

    fn finished(&self) -> bool {
        self.finished
    }

    fn set_responding(&mut self, _pid: u32, _app_id: &str, _responding: bool) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.list.accessibility())
    }

    fn perform(&mut self, request: &ActionRequest) -> bool {
        let performed = self.list.perform(request);
        self.drain();
        performed
    }

    fn cursor(&self) -> Cursor {
        self.list.cursor()
    }

    fn animating(&self) -> bool {
        self.list.animating()
    }

    fn deadline(&self) -> Option<Instant> {
        self.list.deadline()
    }

    fn header(&self) -> Option<Header> {
        Some(Header {
            title_x: match self.full_screen {
                true => SIDE,
                false => LEADING,
            },
            items: self
                .toolbar_frames()
                .into_iter()
                .map(|(_, frame)| frame)
                .collect(),
            tabs: Vec::new(),
        })
    }

    fn set_full_screen(&mut self, full_screen: bool) {
        self.full_screen = full_screen;
    }

    fn inspects(&self, pid: u32) -> bool {
        self.pid == pid
    }
}
