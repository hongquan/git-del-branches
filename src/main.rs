use std::fmt;
use std::time::{Duration, SystemTime};

use clap::Parser;
use color_eyre::Result;
use console::{Emoji, style};
use eyre::Context;
use git2::{Branch, BranchType, Error, PushOptions, Remote, RemoteCallbacks, Repository};
use git2_credentials::CredentialHandler;
use human_units::FormatDuration;
use inquire::error::InquireError;
use inquire::list_option::ListOption;
use inquire::ui::{RenderConfig, Styled};
use inquire::{Confirm, MultiSelect};
use verynicetable::Table;

const EXCLUDES: &[&str] = &["master", "main", "develop", "development"];

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {}

struct BranchChoice<'repo> {
    local: Branch<'repo>,
    upstream: Option<Branch<'repo>>,
    branch_name: String,
    author_name: Option<String>,
    commit_time: SystemTime,
}

impl<'repo> fmt::Display for BranchChoice<'repo> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let upstream = if self.upstream.is_some() {
            " (🔭)"
        } else {
            ""
        };
        let author = self.author_name.as_deref().unwrap_or("no-name");
        let dur = SystemTime::now()
            .duration_since(self.commit_time)
            .unwrap_or_default();
        let ago = human_units::Duration(dur).format_duration();
        write!(
            f,
            "{}{} 🧒 {} ⏰ {} ago",
            self.branch_name, upstream, author, ago
        )
    }
}

fn format_final_answers(opts: &[ListOption<&BranchChoice>]) -> String {
    let data: Vec<_> = opts
        .iter()
        .map(|o| {
            let c = o.value;
            let remote_name = c
                .upstream
                .as_ref()
                .and_then(|b| b.name().ok())
                .flatten()
                .unwrap_or_default();
            let author = c.author_name.as_deref().unwrap_or_default();
            vec![c.branch_name.as_str(), author, remote_name]
        })
        .collect();
    let mut table = Table::new();
    table.headers(&["Local", "Author", "Remote"]).data(&data);
    format!("\n{table}")
}

fn get_branch_choices(repo: &Repository) -> Result<Vec<BranchChoice>, Error> {
    let branches = repo.branches(Some(BranchType::Local))?;
    let mut choices: Vec<_> = branches
        .flatten()
        .filter_map(|(branch, _t)| {
            if branch.is_head() {
                return None;
            }
            let branch_name = branch.name().ok().flatten()?;
            if EXCLUDES.contains(&branch_name) {
                return None;
            }
            let branch_name = branch_name.to_string();
            let upstream = branch.upstream().ok();
            let commit = branch.get().peel_to_commit().ok()?;
            let secs = u64::try_from(commit.time().seconds()).unwrap_or_default();
            let commit_time = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))?;
            let author = commit.author();
            let author_name = author
                .name()
                .or_else(|| author.email().and_then(|s| s.split('@').next()))
                .map(|s| s.to_string());
            Some(BranchChoice {
                local: branch,
                upstream,
                branch_name,
                author_name,
                commit_time,
            })
        })
        .collect();
    choices.sort_unstable_by_key(|c| c.commit_time);
    Ok(choices)
}

fn get_local_name<'a>(branch: &'a Branch) -> Option<&'a str> {
    let name = branch.name().ok().flatten()?;
    name.strip_prefix("origin/").or(Some(name))
}

fn delete_upstream_branch(
    mut branch: Branch,
    origin: &mut Remote,
    opts: &mut PushOptions,
) -> Option<()> {
    let branch_name = get_local_name(&branch)?;
    let refspec = format!(":refs/heads/{}", branch_name);
    let result = origin.push(&[&refspec], Some(opts));
    if let Err(e) = result {
        eprintln!("  {}", style(e.message()).dim());
        let msg = format!("Failed to delete upstream branch {}", branch_name);
        eprintln!("{} {}", Emoji("⚠️", "!"), style(msg).yellow());
    }
    branch.delete().map_err(|e| eprintln!("{e}")).ok()
}

fn get_render_config() -> RenderConfig<'static> {
    RenderConfig {
        scroll_up_prefix: Styled::new("▲"),
        scroll_down_prefix: Styled::new("▼"),
        ..Default::default()
    }
}

fn main() -> Result<()> {
    color_eyre::install()?;
    Cli::parse();
    inquire::set_global_render_config(get_render_config());
    let repo = Repository::discover(".").wrap_err("Not a Git working folder")?;
    let staying_in_branch = repo.head().ok().map(|r| r.is_branch()).unwrap_or(false);
    let branch_choices = get_branch_choices(&repo)?;
    if branch_choices.is_empty() {
        eprintln!("No branches eligible to delete.");
        if staying_in_branch {
            eprintln!(
                "{}",
                style(
                    "You can not delete the branch to are staying in. Please switch to another one."
                )
                .yellow()
            );
        }
        return Ok(());
    }
    let ans_branches = match MultiSelect::new("Select branches to delete", branch_choices)
        .with_formatter(&format_final_answers)
        .prompt()
    {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let ans_up = match Confirm::new("Do you want to delete the upstream branches also?")
        .with_default(false)
        .prompt()
    {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let ans_again = match Confirm::new("Ready to delete?")
        .with_default(false)
        .prompt()
    {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if !ans_again {
        return Ok(());
    }
    let mut remote_callback = RemoteCallbacks::new();
    let git_config = git2::Config::open_default()?;
    let mut credential_handler = CredentialHandler::new(git_config);
    remote_callback.credentials(move |url, username, allowed| {
        let msg = if let Some(name) = username {
            format!(
                "Try authenticating with \"{}\" username for {}...",
                name, url
            )
        } else {
            format!("Try authenticating for {}, without username...", url)
        };
        eprintln!("  {}", style(msg).dim());
        credential_handler.try_next_credential(url, username, allowed)
    });
    let mut origin = repo.find_remote("origin").ok();
    let mut opts = PushOptions::new();
    opts.remote_callbacks(remote_callback);
    for mut c in ans_branches {
        c.local
            .delete()
            .map_err(|e| eprintln!("{e}"))
            .unwrap_or_default();
        if !ans_up {
            continue;
        }
        if let Some((orig, branch)) = origin.as_mut().zip(c.upstream) {
            delete_upstream_branch(branch, orig, &mut opts);
        };
    }
    eprintln!("{} {}", Emoji("🎉", "v"), style("Done!").bright().green());
    Ok(())
}
