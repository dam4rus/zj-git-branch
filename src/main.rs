mod branch;
mod tab;

use std::{collections::BTreeMap, io::BufRead, path::PathBuf};

use branch::{LocalBranch, RemoteBranch};
use tab::Tab;
use zellij_mason::Rect;
use zellij_tile::prelude::*;

#[derive(Default, Clone, Copy)]
enum BranchType {
    #[default]
    Local,
    Remote,
}

#[derive(Default)]
struct Git {
    cwd: Option<PathBuf>,
    open_log_in_floating: bool,
    log_args: Vec<String>,
    branch_type: BranchType,
    local_branches_tab: Tab<LocalBranch>,
    remote_branches_tab: Tab<RemoteBranch>,
    error_message: Option<String>,
}

impl Git {
    const TEXT_LOCAL_TAB: &'static str = "Local";
    const TEXT_REMOTE_TAB: &'static str = "Remote";

    fn successful_command_update(
        &mut self,
        context: BTreeMap<String, String>,
        stdout: Vec<u8>,
    ) -> bool {
        match context.get("command").map(String::as_str) {
            Some("list_local_branches") => {
                let branches: anyhow::Result<Vec<LocalBranch>> = stdout
                    .lines()
                    .map_while(Result::ok)
                    .map(|line| line.parse())
                    .collect();

                match branches {
                    Ok(branches) => {
                        self.local_branches_tab.view.table_state.select_index(
                            branches
                                .iter()
                                .position(|branch| branch.current)
                                .unwrap_or(0),
                        );
                        self.local_branches_tab.view.branches = branches;
                        if !self.local_branches_tab.input.is_empty() {
                            self.local_branches_tab.update_filtered_view();
                        }

                        self.error_message = None;
                    }
                    Err(err) => self.error_message = Some(err.to_string()),
                }
                true
            }
            Some("list_remote_branches") => {
                let branches: anyhow::Result<Vec<RemoteBranch>> = stdout
                    .lines()
                    .map_while(Result::ok)
                    .map(|line| line.parse())
                    .collect();

                match branches {
                    Ok(branches) => {
                        self.remote_branches_tab.view.branches = branches;
                        if !self.remote_branches_tab.input.is_empty() {
                            self.remote_branches_tab.update_filtered_view();
                        }

                        self.error_message = None;
                    }
                    Err(err) => self.error_message = Some(err.to_string()),
                }
                true
            }
            Some("switch") | Some("create") | Some("delete") | Some("fetch") => {
                self.list_local_branches();
                true
            }
            Some("track_remote") => {
                self.branch_type = BranchType::Local;
                self.list_local_branches();
                true
            }
            _ => false,
        }
    }

    fn list_local_branches(&self) {
        let cmd = &["git", "branch", "-vv"];
        let context =
            BTreeMap::from([(String::from("command"), String::from("list_local_branches"))]);
        match &self.cwd {
            Some(cwd) => {
                run_command_with_env_variables_and_cwd(cmd, BTreeMap::new(), cwd.clone(), context)
            }
            None => run_command(cmd, context),
        }
    }

    fn list_remote_branches(&self) {
        let cmd = &["git", "branch", "-r", "-v"];
        let context = BTreeMap::from([(
            String::from("command"),
            String::from("list_remote_branches"),
        )]);
        match &self.cwd {
            Some(cwd) => {
                run_command_with_env_variables_and_cwd(cmd, BTreeMap::new(), cwd.clone(), context)
            }
            None => run_command(cmd, context),
        }
    }

    fn handle_key_input(&mut self, key: KeyWithModifier) -> bool {
        if self.error_message.is_some() {
            self.error_message = None;
            return true;
        }
        if let KeyWithModifier {
            bare_key: BareKey::Esc,
            ..
        } = key
        {
            close_self();
            return true;
        }
        match self.branch_type {
            BranchType::Local => self.handle_local_tab_key_input(key),
            BranchType::Remote => self.handle_remote_tab_key_input(key),
        }
    }

    fn handle_local_tab_key_input(&mut self, key: KeyWithModifier) -> bool {
        match key {
            KeyWithModifier {
                bare_key: BareKey::Tab,
                ..
            } => {
                self.branch_type = BranchType::Remote;
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Down,
                ..
            } => {
                self.local_branches_tab.select_down();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Up,
                ..
            } => {
                self.local_branches_tab.select_up();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Enter,
                ..
            } => match self.local_branches_tab.current_view().selected_branch() {
                Some(branch) => {
                    self.switch_to_branch(branch);
                    true
                }
                None => false,
            },
            KeyWithModifier {
                bare_key: BareKey::Char('c'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                self.local_branches_tab.create_branch(self.cwd.as_ref());
                true
            }

            KeyWithModifier {
                bare_key: BareKey::Char('r'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                self.list_local_branches();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Char('d'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                if let Some(selected_branch) =
                    self.local_branches_tab.current_view().selected_branch()
                {
                    self.delete_branch(&selected_branch.name, false);
                    true
                } else {
                    false
                }
            }
            KeyWithModifier {
                bare_key: BareKey::Char('x'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                if let Some(selected_branch) =
                    self.local_branches_tab.current_view().selected_branch()
                {
                    self.delete_branch(&selected_branch.name, true);
                    true
                } else {
                    false
                }
            }
            KeyWithModifier {
                bare_key: BareKey::Char('l'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                if let Some(selected_branch) =
                    self.local_branches_tab.current_view().selected_branch()
                {
                    self.open_log_pane(&selected_branch.name);
                    true
                } else {
                    false
                }
            }
            KeyWithModifier {
                bare_key: BareKey::Char('p'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                self.switch_to_previous_branch();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Char('f'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                if let Some(selected_branch) =
                    self.local_branches_tab.current_view().selected_branch()
                {
                    if let Err(err) = self.fetch(selected_branch) {
                        self.error_message = Some(err.to_string());
                    }
                    true
                } else {
                    false
                }
            }
            KeyWithModifier {
                bare_key: BareKey::Char(c),
                ..
            } => {
                self.local_branches_tab.push_to_input(c);
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Backspace,
                ..
            } => {
                self.local_branches_tab.pop_from_input();
                true
            }
            _ => false,
        }
    }

    fn handle_remote_tab_key_input(&mut self, key: KeyWithModifier) -> bool {
        match key {
            KeyWithModifier {
                bare_key: BareKey::Tab,
                ..
            } => {
                self.branch_type = BranchType::Local;
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Down,
                ..
            } => {
                self.remote_branches_tab.select_down();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Up,
                ..
            } => {
                self.remote_branches_tab.select_up();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Enter,
                ..
            } => match self.remote_branches_tab.current_view().selected_branch() {
                Some(branch) => {
                    self.track_remote_branch(branch);
                    true
                }
                None => false,
            },
            KeyWithModifier {
                bare_key: BareKey::Char('r'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                self.list_remote_branches();
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Char('l'),
                key_modifiers,
            } if key_modifiers.contains(&KeyModifier::Ctrl) => {
                if let Some(selected_branch) =
                    self.remote_branches_tab.current_view().selected_branch()
                {
                    self.open_log_pane(&selected_branch.name);
                    true
                } else {
                    false
                }
            }
            KeyWithModifier {
                bare_key: BareKey::Char(c),
                ..
            } => {
                self.remote_branches_tab.push_to_input(c);
                true
            }
            KeyWithModifier {
                bare_key: BareKey::Backspace,
                ..
            } => {
                self.remote_branches_tab.pop_from_input();
                true
            }
            _ => false,
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

    fn switch_to_previous_branch(&self) {
        let cmd = &["git", "switch", "-"];
        let context = BTreeMap::from([(String::from("command"), String::from("switch"))]);
        match &self.cwd {
            Some(cwd) => {
                run_command_with_env_variables_and_cwd(cmd, BTreeMap::new(), cwd.clone(), context)
            }
            None => run_command(cmd, context),
        }
    }

    fn fetch(&self, branch: &LocalBranch) -> Result<()> {
        if let Some(upstream_info) = &branch.upstream_info {
            let Some((remote, remote_ref)) = upstream_info.name.split_once('/') else {
                bail!("Invalid upstream")
            };
            let refspec = format!(
                "{}:{}",
                remote_ref
                    .split_once(':')
                    .map(|(prefix, _)| prefix)
                    .unwrap_or(remote_ref),
                branch.name
            );
            let cmd = &["git", "fetch", remote, &refspec];
            let context = BTreeMap::from([(String::from("command"), String::from("fetch"))]);
            match &self.cwd {
                Some(cwd) => run_command_with_env_variables_and_cwd(
                    cmd,
                    BTreeMap::new(),
                    cwd.clone(),
                    context,
                ),
                None => run_command(cmd, context),
            }
            Ok(())
        } else {
            bail!("Local branch does not track any remote branch")
        }
    }

    fn switch_to_branch(&self, branch: &LocalBranch) {
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

    fn track_remote_branch(&self, remote_branch: &RemoteBranch) {
        let command = &["git", "checkout", "--track", &remote_branch.name];
        let context = BTreeMap::from([(String::from("command"), String::from("track_remote"))]);
        match &self.cwd {
            Some(cwd) => run_command_with_env_variables_and_cwd(
                command,
                BTreeMap::new(),
                cwd.clone(),
                context,
            ),
            None => run_command(command, context),
        }
    }

    fn open_log_pane(&self, branch_name: impl AsRef<str>) {
        let mut args = vec!["log"];
        args.extend(self.log_args.iter().map(|arg| arg.as_str()));
        args.push(branch_name.as_ref());

        let mut command_to_run = CommandToRun::new_with_args("git", args);
        command_to_run.cwd = self.cwd.clone();
        if self.open_log_in_floating {
            open_command_pane_floating(command_to_run, None, BTreeMap::new());
        } else {
            open_command_pane(command_to_run, BTreeMap::new());
        }
    }

    fn render_tab_bar(&self) {
        let (local_text, remote_text) = match self.branch_type {
            BranchType::Local => (
                Text::new(Self::TEXT_LOCAL_TAB).selected(),
                Text::new(Self::TEXT_REMOTE_TAB),
            ),
            BranchType::Remote => (
                Text::new(Self::TEXT_LOCAL_TAB),
                Text::new(Self::TEXT_REMOTE_TAB).selected(),
            ),
        };

        print_ribbon_with_coordinates(local_text, 0, 0, None, None);
        print_ribbon_with_coordinates(remote_text, Self::TEXT_LOCAL_TAB.len() + 4, 0, None, None);
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
            Event::Key(key) => self.handle_key_input(key),
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name == "cwd" {
            if let Some(payload) = pipe_message.payload {
                let cwd = PathBuf::from(payload);
                self.cwd = Some(cwd.clone());
                self.list_local_branches();
                self.list_remote_branches();
                return true;
            }
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        match self.branch_type {
            BranchType::Local => {
                if !self.local_branches_tab.inited {
                    self.local_branches_tab.inited = true;
                    self.list_local_branches();
                    return;
                }
            }
            BranchType::Remote => {
                if !self.remote_branches_tab.inited {
                    self.remote_branches_tab.inited = true;
                    self.list_remote_branches();
                    return;
                }
            }
        };

        if let Some(message) = &self.error_message {
            print_text_with_coordinates(Text::new("ERROR").color_range(2, ..), 0, 0, None, None);
            for (y, line) in message.lines().enumerate() {
                print_text_with_coordinates(Text::new(line), 0, y + 1, None, None);
            }
            return;
        }

        const PADDING: usize = 1;
        const FOOTER_HEIGHT: usize = 2;
        const TAB_BAR_HEIGHT: usize = 1;

        self.render_tab_bar();
        let input_rect = Rect {
            x: PADDING,
            y: PADDING + TAB_BAR_HEIGHT,
            width: cols - (2 * PADDING),
            height: 1,
        };
        let table_y = input_rect.y + input_rect.height + PADDING;
        let table_rect = Rect {
            x: PADDING,
            y: table_y,
            width: cols - (2 * PADDING),
            height: rows - table_y - PADDING - FOOTER_HEIGHT,
        };
        match self.branch_type {
            BranchType::Local => {
                print_text_with_coordinates(
                    Text::new(format!(
                        "Branch: {}|",
                        self.local_branches_tab.input.clone()
                    )),
                    input_rect.x,
                    input_rect.y,
                    Some(input_rect.width),
                    Some(input_rect.height),
                );
                self.local_branches_tab.render_branch_list(table_rect);
                self.local_branches_tab.render_help(rows);
            }
            BranchType::Remote => {
                print_text_with_coordinates(
                    Text::new(format!(
                        "Branch: {}|",
                        self.remote_branches_tab.input.clone()
                    )),
                    input_rect.x,
                    input_rect.y,
                    Some(input_rect.width),
                    Some(input_rect.height),
                );
                self.remote_branches_tab.render_branch_list(table_rect);
                self.remote_branches_tab.render_help(rows);
            }
        }

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
