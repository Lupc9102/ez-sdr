//! CLI parsing and command dispatch - translated from src-cli/main.cpp and core/cli/cli.cpp

use clap::{ArgMatches, Command};
use std::sync::Arc;

pub struct CommandHandler {
    app: Command,
    cmd_handlers: Vec<Arc<dyn CmdHandler>>,
}

pub trait CmdHandler: Send + Sync {
    fn cmd(&self) -> &'static str;
    fn reg(&self, app: &mut Command);
    fn run(&self, matches: &ArgMatches, is_gui: bool) -> anyhow::Result<()>;
}

impl CommandHandler {
    pub fn new(_is_gui: bool) -> Self {
        let app = Command::new("satdump")
            .version("0.1.0")
            .about("Satellite data processor core in Rust")
            .subcommand_required(true);
        Self { app, cmd_handlers: Vec::new() }
    }

    pub fn add_handler(&mut self, handler: Arc<dyn CmdHandler>) {
        handler.reg(&mut self.app);
        self.cmd_handlers.push(handler);
    }

    pub fn parse(&mut self, args: &[String]) -> anyhow::Result<ArgMatches> {
        let cmd = self.app.clone().try_get_matches_from(args)?;
        Ok(cmd)
    }

    pub fn run(&self, matches: &ArgMatches) -> anyhow::Result<()> {
        if let Some((name, sub_matches)) = matches.subcommand() {
            for h in &self.cmd_handlers {
                if h.cmd() == name {
                    return h.run(sub_matches, false);
                }
            }
        }
        Ok(())
    }
}

pub fn check_verbose(args: &[String]) -> bool {
    args.iter().any(|a| a == "-v" || a == "--verbose")
}
