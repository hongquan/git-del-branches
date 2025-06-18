use clap::Parser;
use color_eyre::Result;
use console::{Emoji, style};
use eyre::Context;
use git2::{Branch, BranchType, PushOptions, Remote, RemoteCallbacks, Repository};
use git2_credentials::CredentialHandler;
use inquire::error::InquireError;
use inquire::ui::{RenderConfig, Styled};
use inquire::{Confirm, MultiSelect};

const EXCLUDES: &[&str] = &["master", "main", "develop", "development"];

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {}

fn get_branches(repo: &Repository, names: Vec<String>) -> Vec<Branch> {
    names
        .into_iter()
        .filter_map(|n| repo.find_branch(&n, BranchType::Local).ok())
        .collect::<Vec<Branch>>()
}

fn show_list_of_branches(branch_pairs: &Vec<(Branch, Option<Branch>)>) {
    let lines: Vec<String> = branch_pairs
        .iter()
        .filter_map(|(lb, rb)| {
            let local_name = lb.name().ok()??;
            let upstream_name = rb.as_ref().and_then(|n| n.name().ok()).flatten();
            let line = match upstream_name {
                Some(name) => format!(" {local_name} ({name})"),
                None => format!(" {local_name}"),
            };
            Some(line)
        })
        .collect();
    eprintln!("{}", lines.join("\n"));
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
    branch.delete().ok()
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
    let branches = repo.branches(Some(BranchType::Local))?;
    let staying_in_branch = repo.head().ok().map(|r| r.is_branch()).unwrap_or(false);
    let names: Vec<String> = branches
        .flatten()
        .filter_map(|(branch, _type)| {
            if branch.is_head() {
                return None;
            }
            let n = branch.name().ok()??;
            if EXCLUDES.contains(&n) {
                None
            } else {
                Some(n.to_string())
            }
        })
        .collect();
    if names.is_empty() {
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
    let ans_branches = match MultiSelect::new("Select branches to delete", names).prompt() {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let ans_up = match Confirm::new("Do you want to delete the upstream branches also")
        .with_default(false)
        .prompt()
    {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let msg = if ans_up {
        "To delete these branches and their upstream:"
    } else {
        "To delete these branches:"
    };
    eprintln!("{}", style(msg).blue());
    let local_branches = get_branches(&repo, ans_branches);
    let branch_pairs: Vec<(Branch, Option<Branch>)> = local_branches
        .into_iter()
        .map(|b| {
            let upstream = b.upstream().ok();
            (b, upstream)
        })
        .collect();
    show_list_of_branches(&branch_pairs);
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
    for (mut lb, rb) in branch_pairs {
        lb.delete().ok();
        if let Some((orig, branch)) = origin.as_mut().zip(rb) {
            delete_upstream_branch(branch, orig, &mut opts);
        };
    }
    eprintln!("{} {}", Emoji("🎉", "v"), style("Done!").bright().green());
    Ok(())
}
