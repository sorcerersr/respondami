//! Configuration and model context state.
//!
//! Holds `Config`, `model_name`, `context_window`, display modes (thinking, tool
//! output, hooks), and runtime context: `cwd`, `rtk_state`, `skills`,
//! `agents_md_path`, and `hook_registry`.

use std::path::PathBuf;

use super::hook_display::HookDisplay;
use super::thinking_display::ThinkingDisplay;
use crate::config::Config;
use crate::hooks::HookRegistry;
use crate::skills::Skill;
use crate::tools::rtk::RtkState;

/// Configuration and model context state.
#[derive(Debug)]
pub struct ConfigState {
    pub config: Config,
    pub model_name: String,
    pub context_window: u32,
    pub thinking_display: ThinkingDisplay,
    pub tool_output_expanded: bool,
    pub hook_display: HookDisplay,
    pub cwd: PathBuf,
    pub rtk_state: RtkState,
    pub skills: Vec<Skill>,
    pub agents_md_path: Option<PathBuf>,
    pub hook_registry: HookRegistry,
}
