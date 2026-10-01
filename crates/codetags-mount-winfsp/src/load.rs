//! Finding and loading WinFsp at runtime (D3).
//!
//! The DLL is delay-loaded (`build.rs`), so a binary with this backend starts
//! without WinFsp; nothing here may call into WinFsp before [`load`] succeeds.
//! WinFsp's installer does not put its `bin` directory on `PATH`, and
//! `winfsp::winfsp_init` (without winfsp-rs's `system` feature, which needs
//! WinFsp on the build host) only searches the default DLL path. So [`load`]
//! reads the install directory from the registry, loads the DLL by full path,
//! and then calls `winfsp_init`, which finds the already-loaded module by name.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, w};

use crate::availability::{DISABLE_ENV, INSTALL_KEYS, Unavailable, Version, disabled_by, dll_name};

/// A loaded, initialised WinFsp. Holding one is the only way to mount.
#[derive(Debug, Clone)]
pub struct WinFsp {
    dll: PathBuf,
    version: Version,
}

impl WinFsp {
    /// The DLL that was loaded.
    pub fn dll(&self) -> &Path {
        &self.dll
    }

    /// The version the DLL reports.
    pub fn version(&self) -> Version {
        self.version
    }
}

/// Loads WinFsp once per process. Later calls return the first result.
pub fn load() -> Result<WinFsp, Unavailable> {
    static LOADED: OnceLock<Result<WinFsp, Unavailable>> = OnceLock::new();
    LOADED.get_or_init(load_uncached).clone()
}

fn load_uncached() -> Result<WinFsp, Unavailable> {
    if disabled_by(std::env::var_os(DISABLE_ENV).as_deref()) {
        return Err(Unavailable::Disabled);
    }
    let arch = std::env::consts::ARCH;
    let name = dll_name(arch).ok_or(Unavailable::UnsupportedArch(arch))?;
    let install_dir = INSTALL_KEYS
        .iter()
        .find_map(|key| install_dir(key))
        .ok_or(Unavailable::NotInstalled)?;
    let dll = install_dir.join("bin").join(name);
    let unloadable = |reason: String| Unavailable::NotLoadable {
        dll: dll.clone(),
        reason,
    };
    load_library(&dll).map_err(unloadable)?;
    winfsp::winfsp_init().map_err(|error| unloadable(format!("winfsp_init: {error:?}")))?;
    let version = version().map_err(unloadable)?;
    Ok(WinFsp { dll, version })
}

/// `InstallDir` under `HKEY_LOCAL_MACHINE\<key>`, if present.
#[expect(unsafe_code, reason = "Win32 registry read")]
fn install_dir(key: &str) -> Option<PathBuf> {
    const CAPACITY: usize = 1024;
    let mut buffer = [0u16; CAPACITY];
    let mut bytes = (CAPACITY * size_of::<u16>()) as u32;
    // SAFETY: `buffer` is valid for `bytes` bytes, and RegGetValueW writes at
    // most that many and updates `bytes` to the length written, including the
    // terminating NUL (RRF_RT_REG_SZ guarantees one).
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key),
            w!("InstallDir"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let chars = (bytes as usize / size_of::<u16>()).min(CAPACITY);
    let value = &buffer[..chars];
    let value = value.split(|&c| c == 0).next().unwrap_or_default();
    (!value.is_empty()).then(|| PathBuf::from(String::from_utf16_lossy(value)))
}

/// Loads `dll` by full path and keeps it loaded for the life of the process.
#[expect(unsafe_code, reason = "Win32 DLL load")]
fn load_library(dll: &Path) -> Result<(), String> {
    // SAFETY: WinFsp's DLL has no initialisation side effects beyond its own
    // state, and the handle is never freed, so no code is unloaded under us.
    unsafe { LoadLibraryW(&HSTRING::from(dll.as_os_str())) }
        .map(drop)
        .map_err(|error| error.to_string())
}

/// The loaded DLL's version.
#[expect(unsafe_code, reason = "FFI call into the loaded WinFsp DLL")]
fn version() -> Result<Version, String> {
    let mut packed = 0u32;
    // SAFETY: only called after `winfsp_init` succeeded, so the delay-loaded
    // import resolves; FspVersion writes one UINT32 through the pointer.
    let status = unsafe { winfsp_sys::FspVersion(&mut packed) };
    if status < 0 {
        return Err(format!("FspVersion: NTSTATUS {status:#x}"));
    }
    Ok(Version::from_packed(packed))
}
