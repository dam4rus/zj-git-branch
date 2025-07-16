use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use crate::branch::{Branch, LocalBranch, RemoteBranch, RemoteBranchRef, UpstreamInfo};
use nucleo_matcher::{
    Matcher,
    pattern::{CaseMatching, Normalization, Pattern},
};
use zellij_mason::{Rect, table::TableState};
use zellij_tile::prelude::*;

#[derive(Clone)]
pub struct BranchesView<T> {
    pub branches: Vec<T>,
    pub table_state: TableState,
}

impl<T> Default for BranchesView<T> {
    fn default() -> Self {
        Self {
            branches: Vec::default(),
            table_state: TableState::default(),
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
        self.table_state
            .selected_index()
            .and_then(|selected_index| self.branches.get(selected_index))
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
    pub fn select_down(&mut self) {
        self.mut_current_view().table_state.offset_selected_index(1);
    }

    pub fn select_up(&mut self) {
        self.mut_current_view()
            .table_state
            .offset_selected_index(-1);
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
    pub fn push_to_input(&mut self, c: char) {
        self.input.push(c);
        self.update_filtered_view();
    }

    pub fn pop_from_input(&mut self) {
        self.input.pop();
        if self.input.is_empty() {
            self.filtered_view = None;
        } else {
            self.update_filtered_view();
        }
    }

    pub fn update_filtered_view(&mut self) {
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
            }
            filtered_view @ None => {
                *filtered_view = Some(BranchesView::new(visible_branches));
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

    pub fn render_branch_list(&mut self, rect: Rect) {
        let current_view = self.mut_current_view();
        let table_rows = current_view
            .branches
            .iter()
            .map(|branch| {
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
                [
                    name,
                    Text::new(upstream_text).color_range(1, ..),
                    Text::new(branch.commit_sha.clone()),
                    Text::new(branch.commit_message.clone()),
                ]
            })
            .collect::<Vec<_>>();
        zellij_mason::table::draw(
            ["Name", "Upstream", "Sha", "Message"],
            &table_rows,
            rect,
            &mut current_view.table_state,
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

    pub fn render_branch_list(&mut self, rect: Rect) {
        let current_view = self.mut_current_view();
        let table_rows = current_view
            .branches
            .iter()
            .map(|branch| {
                let name = Text::new(branch.name.clone());
                match &branch.reference {
                    RemoteBranchRef::Branch(ref_branch) => [
                        name,
                        Text::new(" "),
                        Text::new(ref_branch.clone()),
                        Text::new(" "),
                    ],
                    RemoteBranchRef::Commit { sha, message } => [
                        name,
                        Text::new(sha.clone()),
                        Text::new(" "),
                        Text::new(message.clone()),
                    ],
                }
            })
            .collect::<Vec<_>>();
        zellij_mason::table::draw(
            ["Name", "Sha", "Ref", "Message"],
            &table_rows,
            rect,
            &mut current_view.table_state,
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
