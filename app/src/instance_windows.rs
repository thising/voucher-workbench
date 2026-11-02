use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{ffi::c_void, io, mem, path::Path, ptr};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) struct Endpoint {
    name: String,
    user_sid: String,
}

pub(super) struct Listener {
    endpoint: Endpoint,
    pending: NamedPipeServer,
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper exclusively owns an OpenProcessToken handle.
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: the Windows conversion APIs allocate these buffers with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

fn current_user_sid() -> Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: token points to initialized writable storage; the pseudo process
    // handle is valid and only read access to the current process token is used.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error()).context("无法读取当前Windows用户身份");
    }
    let token = OwnedHandle(token);
    let mut bytes = 0;
    // SAFETY: a null buffer and zero length request only the required buffer size.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut bytes) };
    if bytes == 0 {
        return Err(io::Error::last_os_error()).context("无法读取当前Windows用户SID长度");
    }
    // usize storage guarantees TOKEN_USER alignment as well as sufficient size.
    let mut storage = vec![0usize; (bytes as usize).div_ceil(mem::size_of::<usize>())];
    // SAFETY: the aligned allocation has at least the requested byte capacity.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("无法读取当前Windows用户SID");
    }
    // SAFETY: GetTokenInformation filled a valid TOKEN_USER in aligned storage.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut converted = ptr::null_mut();
    // SAFETY: the SID belongs to the live TOKEN_USER allocation; output is writable.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut converted) } == 0 {
        return Err(io::Error::last_os_error()).context("无法转换当前Windows用户SID");
    }
    let converted_owner = LocalAllocation(converted.cast());
    let mut length = 0;
    // SAFETY: the successful conversion returns a null-terminated UTF-16 string.
    while unsafe { *converted.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: length was found within the OS-owned terminated allocation.
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(converted, length) })?;
    drop(converted_owner);
    Ok(sid)
}

impl Endpoint {
    pub(super) fn for_state(directory: &Path) -> Result<Self> {
        let user_sid = current_user_sid()?;
        let identity = directory.to_string_lossy().to_lowercase();
        let mut digest = Sha256::new();
        digest.update(user_sid.as_bytes());
        digest.update([0]);
        digest.update(identity.as_bytes());
        Ok(Self {
            name: format!(
                r"\\.\pipe\voucher-workbench-{}",
                hex::encode(digest.finalize())
            ),
            user_sid,
        })
    }

    fn create(&self, first: bool) -> Result<NamedPipeServer> {
        // A protected DACL with exactly one allow ACE. Never use the permissive
        // default pipe descriptor (which also grants Everyone/Anonymous access).
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", self.user_sid)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: sddl is terminated UTF-16; descriptor receives an owned LocalAlloc buffer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error())
                .context("无法建立仅当前用户可访问的命名管道权限");
        }
        let descriptor_owner = LocalAllocation(descriptor);
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = unsafe {
            // SAFETY: SECURITY_ATTRIBUTES and its valid descriptor remain alive
            // throughout CreateNamedPipe; the returned handle is not inherited.
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    &self.name,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                )
        };
        drop(descriptor_owner);
        Ok(result?)
    }

    pub(super) fn bind(self) -> Result<Listener> {
        let pending = self.create(true)?;
        Ok(Listener {
            endpoint: self,
            pending,
        })
    }

    pub(super) async fn connect(&self) -> Result<NamedPipeClient> {
        Ok(ClientOptions::new().open(&self.name)?)
    }
}

impl Listener {
    pub(super) async fn accept(&mut self) -> Result<NamedPipeServer> {
        self.pending.connect().await?;
        // Keep the next protected instance present before handing off this
        // connection, avoiding a pipe-not-found gap between activations.
        let next = self.endpoint.create(false)?;
        Ok(mem::replace(&mut self.pending, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Security::{
        Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT},
        GetAce, ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION,
    };

    #[tokio::test]
    async fn actual_pipe_descriptor_has_one_allow_ace_for_the_current_user() {
        let temp = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::for_state(&temp.path().canonicalize().unwrap()).unwrap();
        let expected = endpoint.user_sid.clone();
        let listener = endpoint.bind().unwrap();
        let mut descriptor = ptr::null_mut();
        let mut dacl = ptr::null_mut();
        // SAFETY: a live pipe handle is queried; outputs are writable OS pointers.
        let result = unsafe {
            GetSecurityInfo(
                listener.pending.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(result, 0);
        let owner = LocalAllocation(descriptor);
        assert!(!dacl.is_null());
        // SAFETY: GetSecurityInfo returned this allocated ACL.
        assert_eq!(unsafe { (*dacl).AceCount }, 1);
        let mut ace = ptr::null_mut();
        // SAFETY: the sole ACE is within the returned ACL.
        assert_ne!(unsafe { GetAce(dacl, 0, &mut ace) }, 0);
        // SAFETY: the descriptor was constructed with one ACCESS_ALLOWED_ACE.
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Header.AceType, 0);
        let sid = (&ace.SidStart as *const u32).cast_mut().cast();
        let mut converted = ptr::null_mut();
        // SAFETY: SidStart is the in-place SID of the verified allow ACE.
        assert_ne!(unsafe { ConvertSidToStringSidW(sid, &mut converted) }, 0);
        let converted_owner = LocalAllocation(converted.cast());
        let mut length = 0;
        // SAFETY: successful conversion returns terminated UTF-16.
        while unsafe { *converted.add(length) } != 0 {
            length += 1;
        }
        let actual =
            String::from_utf16(unsafe { std::slice::from_raw_parts(converted, length) }).unwrap();
        assert_eq!(actual, expected);
        drop(converted_owner);
        drop(owner);
    }
}
