use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    io::BufRead,
    path::PathBuf,
    result::Result,
    str::FromStr,
};

use nom::{
    bytes::complete::{tag, take_till1, take_until1},
    character::complete::{self, hex_digit1, multispace0, not_line_ending},
    combinator::{map, opt},
    error::{context, ParseError},
    sequence::delimited,
    AsChar, IResult, Parser,
};
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Matcher,
};
use zellij_tile::{prelude::*, shim::subscribe, ZellijPlugin};

#[derive(Default, Clone)]
struct Branch {
    name: String,
    current: bool,
    commit_sha: String,
    upstream_branch: Option<String>,
    commit_message: String,
}

impl Branch {
    fn parse_current(value: &str) -> IResult<&str, bool> {
        context("current", map(opt(complete::char('*')), |c| c.is_some())).parse(value)
    }

    fn parse_name(value: &str) -> IResult<&str, String> {
        context("name", map(take_till1(AsChar::is_space), String::from)).parse(value)
    }

    fn parse_commit_sha(value: &str) -> IResult<&str, String> {
        context("commit_sha", map(hex_digit1, String::from)).parse(value)
    }

    fn parse_upstream_branch(value: &str) -> IResult<&str, Option<String>> {
        context(
            "upstream_branch",
            opt(delimited(
                tag("["),
                map(take_until1("]"), String::from),
                tag("]"),
            )),
        )
        .parse(value)
    }

    fn parse_commit_message(value: &str) -> IResult<&str, String> {
        context("commit_message", map(not_line_ending, String::from)).parse(value)
    }
}

impl FromStr for Branch {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (current, name, commit_sha, upstream_branch, commit_message) = (
            ws(Self::parse_current),
            ws(Self::parse_name),
            ws(Self::parse_commit_sha),
            ws(Self::parse_upstream_branch),
            Self::parse_commit_message,
        )
            .parse(s)
            .map_err(|e| anyhow!("Failed to parse branch line: {}", e.to_owned()))?
            .1;

        Ok(Self {
            name,
            current,
            commit_sha,
            upstream_branch,
            commit_message,
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
        self.height - 8
    }

    fn branches_row_count(&self) -> usize {
        self.branches_view_height() + 1
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
        let branch_name_map: HashMap<&str, &Branch> = HashMap::from_iter(
            self.view
                .branches
                .iter()
                .map(|branch| (branch.name.as_str(), branch)),
        );
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        let visible_branches =
            Pattern::parse(&self.input, CaseMatching::Smart, Normalization::Smart)
                .match_list(
                    self.view.branches.iter().map(|branch| branch.name.as_str()),
                    &mut matcher,
                )
                .iter()
                .map(|(branch_name, _)| branch_name_map[branch_name])
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
        let render_area = self.render_area;
        let current_view = self.mut_current_view();
        let header = vec!["name", "upstream", "sha", "message"];
        let table = current_view
            .branches
            .iter()
            .enumerate()
            .skip(current_view.scroll_offset)
            .take(render_area.unwrap().branches_row_count())
            .fold(Table::new().add_row(header), |acc, (i, branch)| {
                let name = Text::new(branch.name.clone());
                let name = if branch.current {
                    name.color_range(2, ..)
                } else {
                    name
                };
                let row = vec![
                    name,
                    Text::new(branch.upstream_branch.clone().unwrap_or(String::from(" ")))
                        .color_range(1, ..),
                    Text::new(branch.commit_sha.clone()),
                    Text::new(branch.commit_message.clone()),
                ];
                let row = if current_view.selected_index == i {
                    row.into_iter().map(|text| text.selected()).collect()
                } else {
                    row
                };
                acc.add_styled_row(row)
            });

        print_table_with_coordinates(table, 1, 3, Some(cols - 3), Some(rows - 6));
    }

    fn successful_command_update(
        &mut self,
        context: BTreeMap<String, String>,
        stdout: Vec<u8>,
    ) -> bool {
        match context.get("command").map(String::as_str) {
            Some("list") => {
                let branches: Result<Vec<Branch>, anyhow::Error> = stdout
                    .lines()
                    .map_while(Result::ok)
                    .map(|line| line.parse())
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
                    Err(err) => self.error_message = Some(err.to_string()),
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
        let cmd = &["git", "branch", "-vv"];
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
        if let Some(selected_branch) = self.current_view().selected_branch() {
            args.push(selected_branch.name.as_str());
        }

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

        let text = "Help: ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.chars().count();
        let text = "<Ctrl-r>";
        print_text_with_coordinates(Text::new(text).color_range(3, 0..), x, y, None, None);

        x += text.chars().count();
        let text = " - Refresh, ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.chars().count();
        let text = "<Ctrl-c>";
        print_text_with_coordinates(Text::new(text).color_range(3, 0..), x, y, None, None);

        x += text.chars().count();
        let text = " - Create, ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.chars().count();
        let text = "<Ctrl-d>";
        print_text_with_coordinates(Text::new(text).color_range(3, 0..), x, y, None, None);

        x += text.chars().count();
        let text = " - Delete, ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.chars().count();
        let text = "<Ctrl-x>";
        print_text_with_coordinates(Text::new(text).color_range(3, 0..), x, y, None, None);

        x += text.chars().count();
        let text = " - Force delete, ";
        print_text_with_coordinates(Text::new(text), x, y, None, None);

        x += text.chars().count();
        let text = "<Ctrl-l>";
        print_text_with_coordinates(Text::new(text).color_range(3, 0..), x, y, None, None);

        x += text.chars().count();
        let text = " - Open log";
        print_text_with_coordinates(Text::new(text), x, y, None, None);
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
            1,
            1,
            Some(cols - 3),
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

pub fn ws<'a, O, E: ParseError<&'a str>, F>(inner: F) -> impl Parser<&'a str, Output = O, Error = E>
where
    F: Parser<&'a str, Output = O, Error = E>,
{
    delimited(multispace0, inner, multispace0)
}

register_plugin!(Git);
