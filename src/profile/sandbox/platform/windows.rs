// Windows sandbox — AppContainer process isolation.
//
// Uses Win32 Security AppContainer APIs for process isolation.
// Compile-gated with #[cfg(target_os = "windows")].

use crate::profile::sandbox::policy::SandboxPolicy;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, S_OK};
use windows_sys::Win32::Security::{
    FreeSid, PSID, SECURITY_CAPABILITIES,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile,
    DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTUPINFOEXW,
};

/// Encode a Rust string as a null-terminated wide (UTF-16) string.
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Spawn a sandboxed process using Windows AppContainer.
///
/// Creates an AppContainer profile, derives its SID, and launches the shell
/// process with `SECURITY_CAPABILITIES` attached via a proc-thread attribute
/// list. The AppContainer restricts file-system, network, and registry access
/// at the OS level.
pub fn spawn_sandboxed_windows(shell: &str, _policy: &SandboxPolicy) -> anyhow::Result<()> {
    let container_name = "cronymax_sandbox";
    let container_name_w = to_wide(container_name);
    let display_name_w = to_wide("Cronymax Sandbox");
    let description_w = to_wide("Cronymax terminal sandbox container");

    // ── 1. Create or reuse AppContainer profile ──────────────────────────
    let mut sid: PSID = ptr::null_mut();

    let hr = unsafe {
        CreateAppContainerProfile(
            container_name_w.as_ptr(),
            display_name_w.as_ptr(),
            description_w.as_ptr(),
            ptr::null_mut(), // no extra capabilities
            0,               // capability count
            &mut sid,
        )
    };

    if hr != S_OK {
        // Profile may already exist — try to derive the SID from the name.
        let hr2 = unsafe {
            DeriveAppContainerSidFromAppContainerName(container_name_w.as_ptr(), &mut sid)
        };
        if hr2 != S_OK {
            return Err(anyhow::anyhow!(
                "Failed to create or derive AppContainer profile (HRESULT: {:#X}, {:#X})",
                hr,
                hr2
            ));
        }
    }

    let result = spawn_in_app_container(shell, sid, container_name);

    // Always free the SID.
    if !sid.is_null() {
        unsafe { FreeSid(sid) };
    }

    result
}

/// Inner function that launches the process, allowing the caller to handle SID cleanup.
fn spawn_in_app_container(
    shell: &str,
    sid: PSID,
    container_name: &str,
) -> anyhow::Result<()> {
    // ── 2. Build SECURITY_CAPABILITIES ───────────────────────────────────
    let security_caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };

    // ── 3. Initialize proc-thread attribute list ─────────────────────────
    let mut attr_size: usize = 0;
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size);
    }

    let mut attr_buf = vec![0u8; attr_size];
    let attr_list = attr_buf.as_mut_ptr() as *mut _;

    let ok = unsafe { InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) };
    if ok == 0 {
        return Err(anyhow::anyhow!(
            "InitializeProcThreadAttributeList failed"
        ));
    }

    let ok = unsafe {
        UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            &security_caps as *const _ as *const _,
            std::mem::size_of::<SECURITY_CAPABILITIES>(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        return Err(anyhow::anyhow!("UpdateProcThreadAttribute failed"));
    }

    // ── 4. Launch the sandboxed process ──────────────────────────────────
    let shell_path = if shell.is_empty() {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        shell.to_string()
    };
    let mut cmd_line = to_wide(&shell_path);

    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = attr_list;

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            cmd_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0, // don't inherit handles
            EXTENDED_STARTUPINFO_PRESENT,
            ptr::null(),
            ptr::null(),
            &si.StartupInfo,
            &mut pi,
        )
    };

    // Always clean up the attribute list.
    unsafe { DeleteProcThreadAttributeList(attr_list) };

    if created == 0 {
        return Err(anyhow::anyhow!(
            "CreateProcessW in AppContainer failed"
        ));
    }

    unsafe {
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
    }

    log::info!(
        "Windows sandbox: launched '{}' in AppContainer '{}'",
        shell_path,
        container_name
    );
    Ok(())
}

/// Remove the AppContainer profile (cleanup helper).
#[allow(dead_code)]
pub fn cleanup_app_container() {
    let name = to_wide("cronymax_sandbox");
    unsafe {
        DeleteAppContainerProfile(name.as_ptr());
    }
}
