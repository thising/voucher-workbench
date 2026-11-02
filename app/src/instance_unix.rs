use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

pub(super) struct Endpoint {
    path: PathBuf,
    uid: u32,
}

pub(super) struct Listener {
    listener: UnixListener,
    path: PathBuf,
    uid: u32,
    identity: (u64, u64),
}

fn private_directory(path: &Path, uid: u32) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("无法创建当前用户的本地通信目录"),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        bail!("本地通信目录的所有者或访问权限不符合要求。请检查当前账号的临时目录权限；不会降低权限要求或删除现有文件");
    }
    Ok(())
}

impl Endpoint {
    pub(super) fn for_state(directory: &Path) -> Result<Self> {
        // SAFETY: geteuid has no preconditions and returns this process's uid.
        let uid = unsafe { libc::geteuid() };
        // macOS per-user temporary paths plus a long state path can exceed the
        // Unix-domain socket limit. This short directory is private and checked
        // rather than inheriting the world-accessible /tmp permissions.
        let private = PathBuf::from(format!("/tmp/voucher-workbench-{uid}"));
        private_directory(&private, uid)?;
        let mut hash = Sha256::new();
        hash.update(uid.to_le_bytes());
        hash.update(directory.as_os_str().as_bytes());
        let key = hex::encode(hash.finalize());
        Ok(Self {
            path: private.join(format!("{}.sock", &key[..32])),
            uid,
        })
    }

    pub(super) fn bind(&self) -> Result<Listener> {
        // The caller has already acquired the record-directory file lock. Only
        // an owned socket may be cleaned after an abnormal prior process exit.
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) => {
                if !metadata.file_type().is_socket() || metadata.uid() != self.uid {
                    bail!("本地通信位置已被其他文件或用户占用，拒绝删除");
                }
                fs::remove_file(&self.path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = UnixListener::bind(&self.path)?;
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&self.path)?;
        Ok(Listener {
            listener,
            path: self.path.clone(),
            uid: self.uid,
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    pub(super) async fn connect(&self) -> Result<UnixStream> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != self.uid
            || metadata.permissions().mode() & 0o077 != 0
        {
            bail!("本地通信通道的所有者或访问权限无效");
        }
        Ok(UnixStream::connect(&self.path).await?)
    }
}

impl Listener {
    pub(super) async fn accept(&mut self) -> Result<UnixStream> {
        let (connection, _) = self.listener.accept().await?;
        if connection.peer_cred()?.uid() != self.uid {
            bail!("本地通信请求不属于当前用户");
        }
        Ok(connection)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // A late Drop from an old listener must not unlink a replacement socket.
        if let Ok(metadata) = fs::symlink_metadata(&self.path) {
            if metadata.file_type().is_socket()
                && metadata.uid() == self.uid
                && (metadata.dev(), metadata.ino()) == self.identity
            {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn socket_is_owner_only_and_long_record_paths_are_supported() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("a".repeat(180));
        fs::create_dir(&directory).unwrap();
        let endpoint = Endpoint::for_state(&directory).unwrap();
        assert!(endpoint.path.as_os_str().as_bytes().len() < 104);
        let listener = endpoint.bind().unwrap();
        let metadata = fs::metadata(&endpoint.path).unwrap();
        assert_eq!(metadata.uid(), endpoint.uid);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        drop(listener);
        assert!(!endpoint.path.exists());
    }

    #[test]
    fn insecure_or_linked_private_directories_are_never_repaired_silently() {
        let temp = tempfile::tempdir().unwrap();
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        let directory = temp.path().join("insecure");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_directory(&directory, uid).is_err());
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&directory, &link).unwrap();
        assert!(private_directory(&link, uid).is_err());
    }
}
