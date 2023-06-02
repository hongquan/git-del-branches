
use eyre::Context;
use git2::{Repository, BranchType, Branch};
use inquire::{MultiSelect, Confirm};
use inquire::error::InquireError;
use color_eyre::Result;
use console::{style, Emoji};


const EXCLUDES: &[&str] = &["master", "main", "develop", "development"];


fn get_branches(repo: &Repository, names: Vec<String>) -> Vec<Branch> {
    names.into_iter().filter_map(|n| {
        repo.find_branch(&n, BranchType::Local).ok()
    }).collect::<Vec<Branch>>()
}

fn show_list_of_branches(branch_pairs: &Vec<(Branch, Option<Branch>)>) {
    let lines: Vec<String> = branch_pairs.iter().filter_map(|(lb, rb)| {
        let local_name = lb.name().ok()??;
        let upstream_name = rb.as_ref().map(|b| b.name().ok()).flatten().flatten();
        let line = match upstream_name {
            Some(name) => format!(" {local_name} ({name})"),
            None => format!(" {local_name}")
        };
        Some(line)
    }).collect();
    eprintln!("{}", lines.join("\n"));
}


fn main() -> Result<()>{
    color_eyre::install()?;
    let repo = Repository::discover(".").wrap_err("Not a Git working folder")?;
    let branches = repo.branches(Some(BranchType::Local))?;
    let names: Vec<String> = branches.filter_map(|b| {
        let branch = b.ok().map(|x| x.0)?;
        let n = branch.name().ok()??;
        if branch.is_head() || EXCLUDES.contains(&n) {
            None
        } else {
            Some(n.to_string())
        }
    }).collect();
    if names.is_empty() {
        eprintln!("No branches found");
        return Ok(());
    }
    let ans_branches = match MultiSelect::new("Select branches to delete", names).prompt() {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into())
    };
    let ans_up = match Confirm::new("Do you want to delete the upstream branches also").with_default(false).prompt() {
        Ok(ans) => ans,
        Err(InquireError::OperationCanceled) => return Ok(()),
        Err(e) => return Err(e.into())
    };
    let msg = if ans_up {
        "To delete these branches and their upstream:"
    } else {
        "To delete these branches:"
    };
    eprintln!("{}", style(msg).blue());
    let local_branches = get_branches(&repo, ans_branches);
    let branch_pairs: Vec<(Branch, Option<Branch>)> = local_branches.into_iter().map(|b| {
        let upstream = b.upstream().ok();
        (b, upstream)
    }).collect();
    show_list_of_branches(&branch_pairs);
    for (mut lb, mut rb) in branch_pairs {
        lb.delete().ok();
        match rb.as_mut() {
            Some(b) => b.delete().ok(),
            None => Some(())
        };
    }
    println!("{} {}", Emoji("🎉", "v"), style("Done!").bright().green());
    Ok(())
}
