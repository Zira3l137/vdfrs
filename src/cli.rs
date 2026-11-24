//! The command line. Without a command it opens archives in the explorer; the commands are what
//! Windows Explorer's context menu runs, and work from a terminal just as well.

use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;

#[derive(Debug, Parser)]
#[command(
    name = "vdfrs",
    version,
    about = "Browse, edit and pack the ZenGin's .vdf and .mod archives"
)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Cli {
    /// Archives to open in the explorer.
    pub archives: Vec<PathBuf>,

    /// Join the other processes Explorer starts for the same selection, to open them as tabs of
    /// one window.
    #[arg(long, hide = true)]
    pub gather: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Extract every file of archives, each into a folder named after it unless told otherwise.
    Extract {
        #[arg(required = true)]
        archives: Vec<PathBuf>,
        /// Into the folder each archive is in.
        #[arg(long, conflicts_with_all = ["to", "pick"])]
        here: bool,
        /// Into this folder.
        #[arg(long, value_name = "FOLDER", conflicts_with = "pick")]
        to: Option<PathBuf>,
        /// Into a folder picked in a dialog.
        #[arg(long)]
        pick: bool,
        /// Join the other processes Explorer starts for the same selection.
        #[arg(long, hide = true)]
        gather: bool,
    },
    /// Pack files and folders into a new archive.
    Pack {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// The archive to write; by default one beside them, named after them.
        #[arg(long, short, value_name = "ARCHIVE")]
        out: Option<PathBuf>,
        #[arg(long, hide = true)]
        gather: bool,
    },
    /// Open a new, unsaved archive holding files and folders in the explorer.
    New {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(long, hide = true)]
        gather: bool,
    },
    /// Add vdfrs to Windows Explorer, or take it out again.
    Shell {
        #[command(subcommand)]
        action: ShellAction,
    },
}

#[derive(Debug, Clone, Copy, Subcommand)]
pub enum ShellAction {
    /// Add the vdfrs menu to Explorer's context menu, for this user only.
    Install {
        /// Also make vdfrs open .vdf and .mod files on a double click.
        #[arg(long)]
        associate: bool,
    },
    /// Remove everything `install` added.
    Uninstall,
    /// Say whether vdfrs is in Explorer, and for which program.
    Status,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn parses_what_the_context_menu_runs() {
        Cli::command().debug_assert();
        let cli = Cli::parse_from(["vdfrs", "extract", "--here", "--gather", r"C:\Data\Anims.vdf"]);
        assert!(matches!(cli.command, Some(Command::Extract { here: true, gather: true, .. })));
        let cli = Cli::parse_from(["vdfrs", "a.vdf", "b.mod"]);
        assert_eq!((cli.archives.len(), cli.command.is_none()), (2, true));
        assert!(Cli::try_parse_from(["vdfrs", "extract", "--here", "--pick", "a.vdf"]).is_err());
    }
}
