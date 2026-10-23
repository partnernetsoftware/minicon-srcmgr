//! Optional user tools. Arguments are discrete values, never shell templates.
use crate::NativePath;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolKind { Editor, Terminal }

impl ToolKind {
    pub const fn name(self) -> &'static str { match self { Self::Editor => "Editor", Self::Terminal => "Terminal" } }
    pub const fn setting(self) -> &'static str { match self { Self::Editor => "tools.editor.v1", Self::Terminal => "tools.terminal.v1" } }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalTool {
    pub executable: NativePath,
    pub arguments: Vec<String>,
}

impl ExternalTool {
    pub fn validate(&self, kind: ToolKind) -> Result<(), &'static str> {
        if self.arguments.len() > 32 || self.arguments.iter().any(|a| a.len() > 4096 || a.contains(['\0', '\r', '\n'])) || self.arguments.iter().map(String::len).sum::<usize>() > 16 * 1024 {
            return Err("Use at most 32 arguments, 4096 bytes each and 16 KiB total, without NUL or line breaks");
        }
        if kind == ToolKind::Editor && !self.arguments.iter().any(|a| a == "{file}") { return Err("Editor arguments need a separate {file} argument"); }
        if kind == ToolKind::Terminal && self.arguments.iter().any(|a| a == "{file}") { return Err("Terminal arguments use {repo}, not {file}"); }
        Ok(())
    }
}
