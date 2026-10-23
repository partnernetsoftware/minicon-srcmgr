//! Installed Git preference. Storage is passive; executable validation and
//! execution authority belong to the application, never a profile read.
use srcmgr_domain::NativePath;
use crate::{ProfileStore, StoreError};
use crate::recent::{decode_path, encode_path};

const GIT_EXECUTABLE: &str = "git.executable.v1";

impl ProfileStore {
    pub fn git_executable(&self) -> Result<Option<NativePath>, StoreError> {
        let Some(value) = self.get_setting_bounded(GIT_EXECUTABLE, 132_000)? else { return Ok(None); };
        let (tag, payload) = value.split_once('\t').ok_or(StoreError::InvalidMetadata)?;
        let path = NativePath::from_encoded(decode_path(tag, payload)?).map_err(|_| StoreError::InvalidMetadata)?;
        Ok(Some(path))
    }

    pub fn set_git_executable(&mut self, path: Option<&NativePath>) -> Result<(), StoreError> {
        match path {
            Some(path) => self.put_setting(GIT_EXECUTABLE, &encode_path(&path.encoded())?),
            None => self.delete_setting(GIT_EXECUTABLE),
        }
    }
}
