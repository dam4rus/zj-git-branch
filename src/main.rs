use std::{cmp::Ordering, collections::BTreeMap, io::BufRead, path::PathBuf};

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
        match offset.cmp(&0) {
            Ordering::Greater => {
                self.selected_index = usize::min(
                    self.selected_index + offset as usize,
                    self.branches.len().saturating_sub(1),
                );
            }
            Ordering::Less => {
                self.selected_index = self.selected_index.saturating_sub(offset.unsigned_abs());
            }
            Ordering::Equal => (),
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
    open_log_in_floating: bool,
    log_args: Vec<String>,
    view: BranchesView,
    filtered_view: Option<BranchesView>,
    render_area: Option<RenderArea>,
    error_message: Option<String>,
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

    fn render_branch_list(&mut self, cols: usize, rows: usize) {
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
    }

    fn successful_command_update(
        &mut self,
        context: BTreeMap<String, String>,
        stdout: Vec<u8>,
    ) -> bool {
        match context.get("command").map(String::as_str) {
            Some("list") => {
                let branches: Result<Vec<Branch>, String> = stdout
                    .lines()
                    .map_while(Result::ok)
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

                        self.error_message = None;
                    }
                    Err(err) => self.error_message = Some(err),
                }
                true
            }
            Some("switch") | Some("create") | Some("delete") => {
                self.list_branches();
                true
            }
            _ => false,
        }
    }

    fn select_down(&mut self) {
        let render_area = self.render_area;
        let current_view = self.mut_current_view();
        current_view.offset_selected_index(1);
        if let Some(render_area) = render_area {
            current_view.scroll_selected_to_view(render_area);
        }
    }

    fn select_up(&mut self) {
        let render_area = self.render_area;
        let current_view = self.mut_current_view();
        current_view.offset_selected_index(-1);
        if let Some(render_area) = render_area {
            current_view.scroll_selected_to_view(render_area);
        }
    }

    fn list_branches(&self) {
        let cmd = &["git", "branch"];
        let context = BTreeMap::from([(String::from("command"), String::from("list"))]);
        match &self.cwd {
            Some(cwd) => {
                run_command_with_env_variables_and_cwd(cmd, BTreeMap::new(), cwd.clone(), context)
            }
            None => run_command(cmd, context),
        }
    }

    fn delete_branch(&self, branch_name: &str, force_delete: bool) {
        let cmd = &[
            "git",
            "branch",
            if force_delete { "-D" } else { "-d" },
            branch_name,
        ];
        let context = BTreeMap::from([(String::from("command"), String::from("delete"))]);
        match &self.cwd {
            Some(cwd) => {
                run_command_with_env_variables_and_cwd(cmd, BTreeMap::new(), cwd.clone(), context)
            }
            None => run_command(cmd, context),
        }
    }

    fn switch_to_branch(&self, branch: &Branch) {
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
    }

    fn create_branch(&mut self) {
        match &self.cwd {
            Some(cwd) => run_command_with_env_variables_and_cwd(
                &["git", "checkout", "-b", &self.input],
                BTreeMap::new(),
                cwd.clone(),
                BTreeMap::from([(String::from("command"), String::from("create"))]),
            ),
            None => run_command(
                &["git", "checkout", "-b", &self.input],
                BTreeMap::from([(String::from("command"), String::from("create"))]),
            ),
        }
    }

    fn push_to_input(&mut self, c: char) {
        self.input.push(c);
        self.update_filtered_view();
    }

    fn pop_from_input(&mut self) {
        self.input.pop();
        if self.input.is_empty() {
            self.filtered_view = None;
        } else {
            self.update_filtered_view();
        }
    }

    fn open_log_pane(&self) {
        let mut args = vec!["log"];
        args.extend(self.log_args.iter().map(|arg| arg.as_str()));

        let mut command_to_run = CommandToRun::new_with_args("git", args);
        command_to_run.cwd = self.cwd.clone();
        if self.open_log_in_floating {
            open_command_pane_floating(command_to_run, None, BTreeMap::new());
        } else {
            open_command_pane(command_to_run, BTreeMap::new());
        }
    }

    fn render_help(&self, rows: usize) {
        let mut x = 0;
        let y = rows - 2;

        let text = "Ctrl + ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.len();
        let text = "<c> Create";
        print_ribbon_with_coordinates(Text::new(text), x, y, None, None);

        x += text.len() + 4;
        let text = "<r> Refresh";
        print_ribbon_with_coordinates(Text::new(text), x, y, None, None);

        x += text.len() + 4;
        let text = "<d> Delete";
        print_ribbon_with_coordinates(Text::new(text), x, y, None, None);

        x += text.len() + 4;
        let text = "<x> Force delete";
        print_ribbon_with_coordinates(Text::new(text), x, y, None, None);

        x += text.len() + 4;
        let text = "<l> Log";
        print_ribbon_with_coordinates(Text::new(text), x, y, None, None);
    }
}

impl ZellijPlugin for Git {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let plugin_ids = get_plugin_ids();
        self.cwd = Some(plugin_ids.initial_cwd.clone());
        self.open_log_in_floating = configuration
            .get("open_log_in_floating")
            .map(|value| value.parse::<bool>().unwrap_or(false))
            .unwrap_or(false);
        self.log_args = configuration
            .get("log_args")
            .map(|value| value.split(" ").map(String::from).collect())
            .unwrap_or_default();

        subscribe(&[EventType::Key, EventType::RunCommandResult]);
        request_permission(&[PermissionType::RunCommands]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::RunCommandResult(Some(0), stdout, _stderr, context) => {
                self.successful_command_update(context, stdout)
            }
            Event::RunCommandResult(Some(_err_code), _stdout, stderr, _context) => {
                self.error_message = Some(String::from_utf8_lossy(&stderr).to_string());
                true
            }
            Event::Key(..) if self.error_message.is_some() => {
                self.error_message = None;
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Down,
                ..
            }) => {
                self.select_down();
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Up,
                ..
            }) => {
                self.select_up();
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Enter,
                ..
            }) => match self.current_view().selected_branch() {
                Some(branch) => {
                    self.switch_to_branch(branch);
                    true
                }
                None => false,
            },
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Backspace,
                ..
            }) => {
                self.pop_from_input();
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Esc,
                ..
            }) => {
                close_self();
                true
            }
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Char(c),
                key_modifiers,
            }) if key_modifiers.contains(&KeyModifier::Ctrl) => match c {
                'c' => {
                    self.create_branch();
                    true
                }
                'r' => {
                    self.list_branches();
                    true
                }
                'd' => {
                    if let Some(selected_branch) = self.current_view().selected_branch() {
                        self.delete_branch(&selected_branch.name, false);
                        true
                    } else {
                        false
                    }
                }
                'x' => {
                    if let Some(selected_branch) = self.current_view().selected_branch() {
                        self.delete_branch(&selected_branch.name, true);
                        true
                    } else {
                        false
                    }
                }
                'l' => {
                    self.open_log_pane();
                    true
                }
                _ => false,
            },
            Event::Key(KeyWithModifier {
                bare_key: BareKey::Char(c),
                ..
            }) => {
                self.push_to_input(c);
                true
            }
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name == "cwd" {
            if let Some(payload) = pipe_message.payload {
                let cwd = PathBuf::from(payload);
                self.cwd = Some(cwd.clone());
                self.list_branches();
                return true;
            }
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        self.render_area = Some(RenderArea::new(rows));
        if !self.inited {
            self.inited = true;
            self.list_branches();
            return;
        }

        if let Some(message) = &self.error_message {
            print_text_with_coordinates(Text::new("ERROR").color_range(2, ..), 0, 0, None, None);
            for (y, line) in message.lines().enumerate() {
                print_text_with_coordinates(Text::new(line), 0, y + 1, None, None);
            }
            return;
        }

        print_text_with_coordinates(
            Text::new(format!("Branch: {}|", self.input.clone())),
            0,
            0,
            None,
            None,
        );

        self.render_branch_list(cols, rows);
        self.render_help(rows);
        if let Some(cwd) = &self.cwd {
            print_text_with_coordinates(
                Text::new(cwd.to_string_lossy().to_string()),
                0,
                rows - 1,
                None,
                None,
            );
        }
    }
}

register_plugin!(Git);
