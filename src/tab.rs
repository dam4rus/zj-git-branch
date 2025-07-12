use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    path::Path,
};

use crate::branch::{Branch, LocalBranch, RemoteBranch, RemoteBranchRef, UpstreamInfo};
use nucleo_matcher::{
    Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
};
use zellij_tile::prelude::*;

#[derive(Default, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: Option<usize>,
    pub height: Option<usize>,
}

#[derive(Default, Clone, Copy)]
pub struct RenderArea {
    pub width: usize,
    pub height: usize,
}

impl RenderArea {
    const Y_PADDING: usize = 1;
    const FOOTER_HEIGHT: usize = 2;

    pub fn new(width: usize, height: usize) -> RenderArea {
        Self { width, height }
    }

    pub fn input_coordinates(&self) -> Rect {
        Rect {
            x: 1,
            y: 2,
            width: Some(self.width - 3),
            height: Some(1),
        }
    }

    fn branch_list_coordinates(&self) -> Rect {
        let input_coordinates = self.input_coordinates();
        let y = input_coordinates.y + input_coordinates.height.unwrap() + Self::Y_PADDING;
        Rect {
            x: 1,
            y,
            width: Some(self.width - 3),
            height: Some(self.height - y - Self::Y_PADDING - Self::FOOTER_HEIGHT),
        }
    }

    fn visible_branch_count(&self) -> usize {
        self.branch_list_coordinates().height.unwrap() - 2
    }
}

#[derive(Clone)]
pub struct BranchesView<T> {
    pub branches: Vec<T>,
    pub selected_index: usize,
    pub scroll_offset: usize,
}

impl<T> Default for BranchesView<T> {
    fn default() -> Self {
        Self {
            branches: Vec::default(),
            selected_index: usize::default(),
            scroll_offset: usize::default(),
        }
    }
}

impl<T> BranchesView<T> {
    fn new(branches: Vec<T>) -> Self {
        Self {
            branches,
            ..Self::default()
        }
    }
}

impl<T> BranchesView<T> {
    pub fn selected_branch(&self) -> Option<&T> {
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

    pub fn scroll_selected_to_view(&mut self, render_area: RenderArea) {
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        } else if self.selected_index > self.scroll_offset + render_area.visible_branch_count() {
            self.scroll_offset = self.selected_index - render_area.visible_branch_count();
        }
    }
}

#[derive(Clone)]
pub struct Tab<T> {
    pub inited: bool,
    pub input: String,
    pub view: BranchesView<T>,
    pub filtered_view: Option<BranchesView<T>>,
}

impl<T> Default for Tab<T> {
    fn default() -> Self {
        Self {
            inited: bool::default(),
            input: String::default(),
            view: BranchesView::default(),
            filtered_view: Option::default(),
        }
    }
}

impl<T> Tab<T> {
    pub fn select_down(&mut self, render_area: Option<RenderArea>) {
        let current_view = self.mut_current_view();
        current_view.offset_selected_index(1);
        if let Some(render_area) = render_area {
            current_view.scroll_selected_to_view(render_area);
        }
    }

    pub fn select_up(&mut self, render_area: Option<RenderArea>) {
        let current_view = self.mut_current_view();
        current_view.offset_selected_index(-1);
        if let Some(render_area) = render_area {
            current_view.scroll_selected_to_view(render_area);
        }
    }

    pub fn current_view(&self) -> &BranchesView<T> {
        match &self.filtered_view {
            Some(filtered_view) => filtered_view,
            None => &self.view,
        }
    }

    fn mut_current_view(&mut self) -> &mut BranchesView<T> {
        match &mut self.filtered_view {
            Some(filtered_view) => filtered_view,
            None => &mut self.view,
        }
    }
}

impl<T: Branch + Clone> Tab<T> {
    pub fn push_to_input(&mut self, c: char, render_area: Option<RenderArea>) {
        self.input.push(c);
        self.update_filtered_view(render_area);
    }

    pub fn pop_from_input(&mut self, render_area: Option<RenderArea>) {
        self.input.pop();
        if self.input.is_empty() {
            self.filtered_view = None;
        } else {
            self.update_filtered_view(render_area);
        }
    }

    pub fn update_filtered_view(&mut self, render_area: Option<RenderArea>) {
        let branch_name_map: HashMap<&str, &T> = HashMap::from_iter(
            self.view
                .branches
                .iter()
                .map(|branch| (branch.name(), branch)),
        );
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        let visible_branches = Pattern::parse(
            self.input.as_str(),
            CaseMatching::Smart,
            Normalization::Smart,
        )
        .match_list(
            self.view.branches.iter().map(|branch| branch.name()),
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
                if let Some(render_area) = render_area {
                    filtered_view.scroll_selected_to_view(render_area);
                }
            }
            filtered_view @ None => {
                let mut view = BranchesView::new(visible_branches);
                if let Some(render_area) = render_area {
                    view.scroll_selected_to_view(render_area);
                }
                *filtered_view = Some(view);
            }
        }
    }
}

impl Tab<LocalBranch> {
    pub fn create_branch(&mut self, cwd: Option<&impl AsRef<Path>>) {
        match cwd {
            Some(cwd) => run_command_with_env_variables_and_cwd(
                &["git", "checkout", "-b", &self.input],
                BTreeMap::new(),
                cwd.as_ref().to_owned(),
                BTreeMap::from([(String::from("command"), String::from("create"))]),
            ),
            None => run_command(
                &["git", "checkout", "-b", &self.input],
                BTreeMap::from([(String::from("command"), String::from("create"))]),
            ),
        }
    }

    pub fn render_help(&self, rows: usize) {
        let x = 0;
        let y = rows - 2;

        let (x, y) = print_command_help("<Ctrl-r>", "Refresh", x, y);
        let (x, y) = print_help_separator(x, y);
        let (x, y) = print_command_help("<Ctrl-c>", "Create", x, y);
        let (x, y) = print_help_separator(x, y);
        let (x, y) = print_command_help("<Ctrl-d>", "Delete", x, y);
        let (x, y) = print_help_separator(x, y);
        let (x, y) = print_command_help("<Ctrl-x>", "Force delete", x, y);
        let (x, y) = print_help_separator(x, y);
        let (x, y) = print_command_help("<Ctrl-l>", "Open log", x, y);
        let (x, y) = print_help_separator(x, y);
        let (x, y) = print_command_help("<Ctrl-p>", "Previous branch", x, y);
        let (x, y) = print_help_separator(x, y);
        print_command_help("<Ctrl-f>", "Fetch", x, y);
    }

    pub fn render_branch_list(&mut self, render_area: RenderArea) {
        let current_view = self.mut_current_view();
        let header = vec!["Name", "Upstream", "Sha", "Message"];
        let table = current_view
            .branches
            .iter()
            .enumerate()
            .skip(current_view.scroll_offset)
            .take(render_area.branch_list_coordinates().height.unwrap() - 1)
            .fold(Table::new().add_row(header), |acc, (i, branch)| {
                let name = Text::new(branch.name.clone());
                let name = if branch.current {
                    name.color_range(2, ..)
                } else {
                    name
                };
                let upstream_text = match &branch.upstream_info {
                    Some(UpstreamInfo {
                        name,
                        relationship: Some(relationship),
                    }) => format!("{name}: {relationship}"),
                    Some(UpstreamInfo {
                        name,
                        relationship: None,
                    }) => name.clone(),
                    None => String::from(" "),
                };
                let row = vec![
                    name,
                    Text::new(upstream_text).color_range(1, ..),
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

        let branch_list_coordinates = render_area.branch_list_coordinates();
        print_table_with_coordinates(
            table,
            branch_list_coordinates.x,
            branch_list_coordinates.y,
            branch_list_coordinates.width,
            branch_list_coordinates.height,
        );
    }
}

impl Tab<RemoteBranch> {
    pub fn render_help(&self, rows: usize) {
        let x = 0;
        let y = rows - 2;

        let (x, y) = print_command_help("<Ctrl-r>", "Refresh", x, y);
        let (x, y) = print_help_separator(x, y);
        print_command_help("<Ctrl-l>", "Open log", x, y);
    }

    pub fn render_branch_list(&mut self, render_area: RenderArea) {
        let current_view = self.mut_current_view();
        let header = vec!["Name", "Sha", "Ref", "Message"];
        let table = current_view
            .branches
            .iter()
            .enumerate()
            .skip(current_view.scroll_offset)
            .take(render_area.branch_list_coordinates().height.unwrap() - 1)
            .fold(Table::new().add_row(header), |acc, (i, branch)| {
                let name = Text::new(branch.name.clone());
                let mut row = vec![name];
                match &branch.reference {
                    RemoteBranchRef::Branch(ref_branch) => row.extend([
                        Text::new(" "),
                        Text::new(ref_branch.clone()),
                        Text::new(" "),
                    ]),
                    RemoteBranchRef::Commit { sha, message } => row.extend([
                        Text::new(sha.clone()),
                        Text::new(" "),
                        Text::new(message.clone()),
                    ]),
                }
                let row = if current_view.selected_index == i {
                    row.into_iter().map(|text| text.selected()).collect()
                } else {
                    row
                };
                acc.add_styled_row(row)
            });

        let branch_list_coordinates = render_area.branch_list_coordinates();
        print_table_with_coordinates(
            table,
            branch_list_coordinates.x,
            branch_list_coordinates.y,
            branch_list_coordinates.width,
            branch_list_coordinates.height,
        );
    }
}

fn print_command_help(
    key: impl AsRef<str> + ToString,
    help_text: impl AsRef<str> + ToString,
    mut x: usize,
    y: usize,
) -> (usize, usize) {
    print_text_with_coordinates(
        Text::new(key.as_ref()).color_range(3, 0..),
        x,
        y,
        None,
        None,
    );

    x += key.as_ref().chars().count();
    let separator = " - ";
    print_text_with_coordinates(Text::new(separator), x, y, None, None);

    x += separator.chars().count();

    print_text_with_coordinates(Text::new(help_text.as_ref()), x, y, None, None);
    x += help_text.as_ref().chars().count();
    (x, y)
}

fn print_help_separator(x: usize, y: usize) -> (usize, usize) {
    let text = ", ";
    print_text_with_coordinates(Text::new(text), x, y, None, None);

    (x + text.chars().count(), y)
}
