use std::{collections::BTreeMap, io::BufRead, path::PathBuf};

use zellij_tile::{prelude::*, shim::subscribe, ZellijPlugin};

#[derive(Default, Clone)]
struct Branch {
    name: String,
    current: bool,
}

impl TryFrom<&str> for Branch {
    type Error = String;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        let mut chars = value.chars();

        let current = chars.next().ok_or_else(|| String::from("value is empty"))? == '*';
        chars
            .next()
            .ok_or_else(|| String::from("value too short"))?;
        Ok(Self {
            name: chars.collect(),
            current,
        })
    }
}

#[derive(Default, Clone, Copy)]
pub struct RenderArea {
    height: usize,
}

impl RenderArea {
    fn new(height: usize) -> RenderArea {
        Self { height }
    }

    fn branches_view_height(&self) -> usize {
        self.height - 4
    }
}

#[derive(Default, Clone)]
struct BranchesView {
    branches: Vec<Branch>,
    selected_index: usize,
    scroll_offset: usize,
}

impl BranchesView {
    fn new(branches: Vec<Branch>) -> Self {
        Self {
            branches,
            ..Self::default()
        }
    }

    fn selected_branch(&self) -> Option<&Branch> {
        if !self.branches.is_empty() {
            Some(&self.branches[self.selected_index])
        } else {
            None
        }
    }

    fn offset_selected_index(&mut self, offset: isize) {
        if offset > 0 {
            self.selected_index = usize::min(
                self.selected_index + offset as usize,
                self.branches.len().saturating_sub(1),
            );
        } else if offset < 0 {
            self.selected_index = self.selected_index.saturating_sub(offset.abs() as usize);
        }
    }

    fn scroll_selected_to_view(&mut self, render_area: RenderArea) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index > self.scroll_offset + render_area.branches_view_height() {
            self.scroll_offset = self.selected_index - render_area.branches_view_height();
        }
    }
}

#[derive(Default)]
struct Git {
    inited: bool,
    cwd: Option<PathBuf>,
    view: BranchesView,
    filtered_view: Option<BranchesView>,
    render_area: Option<RenderArea>,
    message: Option<String>,
    input: String,
}

impl Git {
    fn current_view(&self) -> &BranchesView {
        match &self.filtered_view {
            Some(filtered_view) => filtered_view,
            None => &self.view,
        }
    }

    fn mut_current_view(&mut self) -> &mut BranchesView {
        match &mut self.filtered_view {
            Some(filtered_view) => filtered_view,
            None => &mut self.view,
        }
    }

    fn request_branches(&self) {
        match &self.cwd {
            Some(cwd) => run_command_with_env_variables_and_cwd(
                &["git", "branch"],
                BTreeMap::new(),
                cwd.clone(),
                BTreeMap::from([(String::from("command"), String::from("list"))]),
            ),
            None => run_command(
                &["git", "branch"],
                BTreeMap::from([(String::from("command"), String::from("list"))]),
            ),
        }
    }

    fn update_filtered_view(&mut self) {
        let visible_branches: Vec<Branch> = self
            .view
            .branches
            .iter()
            .filter(|branch| branch.name.starts_with(&self.input))
            .cloned()
            .collect();
        match &mut self.filtered_view {
            Some(filtered_view) => {
                filtered_view.branches = visible_branches;
                if filtered_view.selected_index >= filtered_view.branches.len() {
                    filtered_view.selected_index = 0;
                }
                if let Some(render_area) = self.render_area {
                    filtered_view.scroll_selected_to_view(render_area);
                }
            }
            filtered_view @ None => {
                let mut view = BranchesView::new(visible_branches);
                if let Some(render_area) = self.render_area {
                    view.scroll_selected_to_view(render_area);
                }
                *filtered_view = Some(view);
            }
        }
    }
}

impl ZellijPlugin for Git {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        let plugin_ids = get_plugin_ids();
        self.cwd = Some(plugin_ids.initial_cwd.clone());
        subscribe(&[
            EventType::Key,
            EventType::RunCommandResult,
            EventType::Visible,
        ]);
        request_permission(&[PermissionType::RunCommands]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Visible(true) => {
                self.request_branches();
                true
            }
            Event::RunCommandResult(Some(0), stdout, _stderr, context) => {
                match context.get("command").map(String::as_str) {
                    Some("list") => {
                        let branches: Result<Vec<Branch>, String> = stdout
                            .lines()
                            .filter_map(|line| line.ok())
                            .map(|line| Branch::try_from(line.as_str()))
                            .collect();

                        match branches {
                            Ok(branches) => {
                                self.view.selected_index = branches
                                    .iter()
                                    .position(|branch| branch.current)
                                    .unwrap_or(0);
                                self.view.branches = branches;
                                if let Some(render_area) = self.render_area {
                                    self.view.scroll_selected_to_view(render_area);
                                }

                                if !self.input.is_empty() {
                                    self.update_filtered_view();
                                }

                                self.message = None;
                            }
                            Err(err) => self.message = Some(err),
                        }
                        true
                    }
                    Some("switch") | Some("create") => {
                        self.request_branches();
                        true
                    }
                    _ => false,
                }
            }
            Event::RunCommandResult(Some(_err_code), _stdout, stderr, _context) => {
                self.message = Some(String::from_utf8_lossy(&stderr).to_string());
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Down,
                ..
            }) => {
                let render_area = self.render_area;
                let current_view = self.mut_current_view();
                current_view.offset_selected_index(1);
                if let Some(render_area) = render_area {
                    current_view.scroll_selected_to_view(render_area);
                }
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Up,
                ..
            }) => {
                let render_area = self.render_area;
                let current_view = self.mut_current_view();
                current_view.offset_selected_index(-1);
                if let Some(render_area) = render_area {
                    current_view.scroll_selected_to_view(render_area);
                }
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Enter,
                ..
            }) => match self.current_view().selected_branch() {
                Some(branch) => {
                    match &self.cwd {
                        Some(cwd) => run_command_with_env_variables_and_cwd(
                            &["git", "switch", &branch.name],
                            BTreeMap::new(),
                            cwd.clone(),
                            BTreeMap::from([(String::from("command"), String::from("switch"))]),
                        ),
                        None => run_command(
                            &["git", "switch", &branch.name],
                            BTreeMap::from([(String::from("command"), String::from("switch"))]),
                        ),
                    }
                    true
                }
                None => false,
            },
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Backspace,
                ..
            }) => {
                self.input.pop();
                if self.input.is_empty() {
                    self.filtered_view = None;
                } else {
                    self.update_filtered_view();
                }
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Esc,
                ..
            }) => {
                hide_self();
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Char(c),
                key_modifiers,
            }) => {
                if key_modifiers.contains(&KeyModifier::Ctrl) {
                    match c {
                        'c' => {
                            match &self.cwd {
                                Some(cwd) => run_command_with_env_variables_and_cwd(
                                    &["git", "checkout", "-b", &self.input],
                                    BTreeMap::new(),
                                    cwd.clone(),
                                    BTreeMap::from([(
                                        String::from("command"),
                                        String::from("create"),
                                    )]),
                                ),
                                None => run_command(
                                    &["git", "checkout", "-b", &self.input],
                                    BTreeMap::from([(
                                        String::from("command"),
                                        String::from("create"),
                                    )]),
                                ),
                            }

                            true
                        }
                        'r' => {
                            self.request_branches();
                            true
                        }
                        _ => false,
                    }
                } else {
                    self.input.push(c);
                    self.update_filtered_view();
                    true
                }
            }
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name == "cwd" {
            if let Some(payload) = pipe_message.payload {
                let cwd = PathBuf::from(payload);
                self.cwd = Some(cwd.clone());
                self.request_branches();
                return true;
            }
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.render_area = Some(RenderArea::new(rows));
        if !self.inited {
            self.inited = true;
            self.request_branches();
            return;
        }

        print_text_with_coordinates(
            Text::new(format!("Branch: {}█", self.input.clone())),
            0,
            0,
            None,
            None,
        );

        let current_view = self.mut_current_view();
        let list_items: Vec<NestedListItem> = current_view
            .branches
            .iter()
            .enumerate()
            .skip(current_view.scroll_offset)
            .map(|(i, branch)| {
                let list_item = NestedListItem::new(branch.name.clone());
                let list_item = if branch.current {
                    list_item.color_range(2, 0..)
                } else {
                    list_item
                };
                if current_view.selected_index == i {
                    list_item.selected()
                } else {
                    list_item
                }
            })
            .collect();

        print_nested_list_with_coordinates(list_items, 0, 1, Some(cols), Some(rows - 3));

        {
            let mut x = 0;
            let y = rows - 2;
            let text = "Ctrl +";
            print_ribbon_with_coordinates(Text::new(text), x, y, None, None);
            x += text.len() + 4;
            let text = "<c> CREATE";
            print_ribbon_with_coordinates(Text::new(text), x, y, None, None);
            x += text.len() + 4;
            let text = "<r> REFRESH";
            print_ribbon_with_coordinates(Text::new(text), x, y, None, None);
        }

        if let Some(error_message) = &self.message {
            if let Some(first_line) = error_message.lines().next() {
                print_text_with_coordinates(Text::new(first_line), 0, rows - 1, None, None);
            }
        }
    }
}

register_plugin!(Git);
