//! Passive user-tool definitions; reading or saving never starts a program.
use srcmgr_domain::{ExternalTool, ToolKind, NativePath};
use crate::{ProfileStore, StoreError};
use crate::recent::{decode_path, encode_path};

impl ProfileStore {
    pub fn external_tool(&self, kind: ToolKind) -> Result<Option<ExternalTool>, StoreError> {
        let Some(value) = self.get_setting_bounded(kind.setting(), 180_000)? else { return Ok(None); };
        let fields: Vec<_> = value.split('\n').collect();
        if fields.len() < 3 || fields[0] != "1" || fields.len() > 35 { return Err(StoreError::InvalidMetadata); }
        let (tag, payload) = fields[1].split_once('\t').ok_or(StoreError::InvalidMetadata)?;
        let executable = NativePath::from_encoded(decode_path(tag, payload)?).map_err(|_| StoreError::InvalidMetadata)?;
        let count = fields[2].parse::<usize>().map_err(|_| StoreError::InvalidMetadata)?;
        if count != fields.len() - 3 { return Err(StoreError::InvalidMetadata); }
        let arguments = fields[3..].iter().map(|s| {
            if s.len() > 8192 || s.len() % 2 != 0 { return Err(StoreError::InvalidMetadata); }
            let bytes = s.as_bytes().chunks_exact(2).map(|pair| {
                let text = std::str::from_utf8(pair).map_err(|_| StoreError::InvalidMetadata)?;
                u8::from_str_radix(text, 16).map_err(|_| StoreError::InvalidMetadata)
            }).collect::<Result<Vec<_>,_>>()?;
            String::from_utf8(bytes).map_err(|_| StoreError::InvalidMetadata)
        }).collect::<Result<Vec<_>,_>>()?;
        let definition = ExternalTool { executable, arguments };
        definition.validate(kind).map_err(|_| StoreError::InvalidMetadata)?;
        Ok(Some(definition))
    }

    pub fn set_external_tool(&mut self, kind: ToolKind, definition: Option<&ExternalTool>) -> Result<(), StoreError> {
        let Some(definition) = definition else { return self.delete_setting(kind.setting()); };
        definition.validate(kind).map_err(|_| StoreError::InvalidMetadata)?;
        let mut fields = vec!["1".into(), encode_path(&definition.executable.encoded())?, definition.arguments.len().to_string()];
        fields.extend(definition.arguments.iter().map(|a| a.as_bytes().iter().map(|b| format!("{b:02x}")).collect::<String>()));
        let value = fields.join("\n");
        if value.len() > 180_000 { return Err(StoreError::InvalidMetadata); }
        self.put_setting(kind.setting(), &value)
    }
}
