use libloading::Library;
use std::ffi::CStr;
use std::fs::{self};
use std::io;
use std::mem::ManuallyDrop;
use std::os::raw::{c_char, c_int, c_long, c_short, c_uint, c_ulong, c_void};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use tracing::{debug, info};

pub type CurlCode = c_int;
pub type CurlOption = c_uint;
pub type CurlMCode = c_int;
pub type CurlMOption = c_int;

#[cfg(windows)]
pub type CurlSocket = usize;
#[cfg(not(windows))]
pub type CurlSocket = c_int;

pub type CurlMultiSocketCallback =
    unsafe extern "C" fn(*mut Curl, CurlSocket, c_int, *mut c_void, *mut c_void) -> c_int;
pub type CurlMultiTimerCallback =
    unsafe extern "C" fn(*mut CurlMulti, c_long, *mut c_void) -> c_int;

#[repr(C)]
pub struct Curl {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CurlMulti {
    _private: [u8; 0],
}

#[repr(C)]
pub struct CurlSlist {
    pub data: *mut c_char,
    pub next: *mut CurlSlist,
}

#[repr(C)]
pub struct CurlWsFrame {
    pub age: c_int,
    pub flags: c_int,
    pub offset: i64,
    pub bytesleft: i64,
    pub len: usize,
}

#[repr(C)]
pub union CurlMessageData {
    pub whatever: *mut c_void,
    pub result: CurlCode,
}

#[repr(C)]
pub struct CurlMessage {
    pub msg: c_int,
    pub easy_handle: *mut Curl,
    pub data: CurlMessageData,
}

#[repr(C)]
pub struct CurlWaitFd {
    pub fd: CurlSocket,
    pub events: c_short,
    pub revents: c_short,
}

pub const CURLE_OK: CurlCode = 0;
pub const CURLE_AGAIN: CurlCode = 81;
pub const CURL_GLOBAL_DEFAULT: c_ulong = 3;
pub const CURLM_OK: CurlMCode = 0;
pub const CURLMSG_DONE: c_int = 1;

#[cfg(windows)]
pub const CURL_SOCKET_TIMEOUT: CurlSocket = usize::MAX;
#[cfg(not(windows))]
pub const CURL_SOCKET_TIMEOUT: CurlSocket = -1;

pub const CURL_CSELECT_IN: c_int = 0x01;
pub const CURL_CSELECT_OUT: c_int = 0x02;
pub const CURL_CSELECT_ERR: c_int = 0x04;

pub const CURL_POLL_NONE: c_int = 0;
pub const CURL_POLL_IN: c_int = 1;
pub const CURL_POLL_OUT: c_int = 2;
pub const CURL_POLL_INOUT: c_int = 3;
pub const CURL_POLL_REMOVE: c_int = 4;

pub const CURLMOPT_SOCKETFUNCTION: CurlMOption = 20001;
pub const CURLMOPT_SOCKETDATA: CurlMOption = 10002;
pub const CURLMOPT_TIMERFUNCTION: CurlMOption = 20004;
pub const CURLMOPT_TIMERDATA: CurlMOption = 10005;

pub const CURLOPT_URL: CurlOption = 10002;
pub const CURLOPT_HTTPHEADER: CurlOption = 10023;
pub const CURLOPT_HTTP_VERSION: CurlOption = 84;
pub const CURLOPT_CONNECT_ONLY: CurlOption = 141;
pub const CURLOPT_VERBOSE: CurlOption = 41;
pub const CURLOPT_PROXY: CurlOption = 10004;
pub const CURLOPT_CAINFO: CurlOption = 10065;
pub const CURLINFO_RESPONSE_CODE: c_uint = 0x200002;

pub const CURL_HTTP_VERSION_1_1: c_long = 2;
pub const CURLWS_TEXT: c_uint = 1;

#[derive(Debug, thiserror::Error)]
pub enum SysError {
    #[error("failed to load dynamic library {path}: {source}")]
    LoadLibrary {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    #[error("missing symbol {name}: {source}")]
    MissingSymbol {
        name: String,
        #[source]
        source: libloading::Error,
    },
    #[error("CURL_IMPERSONATE_LIB points to a missing file: {0}")]
    MissingEnvPath(PathBuf),
    #[error("failed to locate libcurl-impersonate. searched: {0:?}")]
    LibraryNotFound(Vec<PathBuf>),
    #[error(
        "failed to locate libcurl-impersonate after auto-fetch attempt. searched: {searched:?}; auto-fetch error: {auto_fetch_error}"
    )]
    LibraryNotFoundAfterAutoFetch {
        searched: Vec<PathBuf>,
        auto_fetch_error: String,
    },
    #[error("auto-fetch is not supported on target: {0}")]
    AutoFetchUnsupportedTarget(String),
    #[error("auto-fetch needs cache directory but HOME and IMPCURL_LIB_DIR are not set")]
    AutoFetchCacheDirUnavailable,
    #[error("failed to run downloader command {command}: {source}")]
    AutoFetchCommandSpawn { command: String, source: io::Error },
    #[error("downloader command {command} failed with status {status:?}: {stderr}")]
    AutoFetchCommandFailed {
        command: String,
        status: Option<i32>,
        stderr: String,
    },
    #[error("I/O error during auto-fetch: {0}")]
    AutoFetchIo(#[from] io::Error),
    #[error("no standalone libcurl-impersonate shared library was found in {cache_dir}")]
    AutoFetchNoStandaloneRuntime { cache_dir: PathBuf },
    #[error("no libcurl-impersonate asset naming rule for target: {0}")]
    AutoFetchRuntimeUnsupportedTarget(String),
}

pub struct CurlApi {
    // Keep the dynamic library loaded for process lifetime. Unloading can crash
    // with libcurl-impersonate on process teardown in some environments.
    _lib: ManuallyDrop<Library>,
    pub global_init: unsafe extern "C" fn(c_ulong) -> CurlCode,
    pub global_cleanup: unsafe extern "C" fn(),
    pub easy_init: unsafe extern "C" fn() -> *mut Curl,
    pub easy_cleanup: unsafe extern "C" fn(*mut Curl),
    pub easy_perform: unsafe extern "C" fn(*mut Curl) -> CurlCode,
    pub easy_setopt: unsafe extern "C" fn(*mut Curl, CurlOption, ...) -> CurlCode,
    pub easy_getinfo: unsafe extern "C" fn(*mut Curl, c_uint, ...) -> CurlCode,
    pub easy_strerror: unsafe extern "C" fn(CurlCode) -> *const c_char,
    pub easy_impersonate: unsafe extern "C" fn(*mut Curl, *const c_char, c_int) -> CurlCode,
    pub slist_append: unsafe extern "C" fn(*mut CurlSlist, *const c_char) -> *mut CurlSlist,
    pub slist_free_all: unsafe extern "C" fn(*mut CurlSlist),
    pub ws_send:
        unsafe extern "C" fn(*mut Curl, *const c_void, usize, *mut usize, i64, c_uint) -> CurlCode,
    pub ws_recv: unsafe extern "C" fn(
        *mut Curl,
        *mut c_void,
        usize,
        *mut usize,
        *mut *const CurlWsFrame,
    ) -> CurlCode,
    pub multi_init: unsafe extern "C" fn() -> *mut CurlMulti,
    pub multi_cleanup: unsafe extern "C" fn(*mut CurlMulti) -> CurlMCode,
    pub multi_setopt: unsafe extern "C" fn(*mut CurlMulti, CurlMOption, ...) -> CurlMCode,
    pub multi_add_handle: unsafe extern "C" fn(*mut CurlMulti, *mut Curl) -> CurlMCode,
    pub multi_remove_handle: unsafe extern "C" fn(*mut CurlMulti, *mut Curl) -> CurlMCode,
    pub multi_fdset: unsafe extern "C" fn(
        *mut CurlMulti,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_int,
    ) -> CurlMCode,
    pub multi_timeout: unsafe extern "C" fn(*mut CurlMulti, *mut c_long) -> CurlMCode,
    pub multi_perform: unsafe extern "C" fn(*mut CurlMulti, *mut c_int) -> CurlMCode,
    pub multi_poll: unsafe extern "C" fn(
        *mut CurlMulti,
        *mut CurlWaitFd,
        c_uint,
        c_int,
        *mut c_int,
    ) -> CurlMCode,
    pub multi_socket_action:
        unsafe extern "C" fn(*mut CurlMulti, CurlSocket, c_int, *mut c_int) -> CurlMCode,
    pub multi_info_read: unsafe extern "C" fn(*mut CurlMulti, *mut c_int) -> *mut CurlMessage,
    pub multi_strerror: unsafe extern "C" fn(CurlMCode) -> *const c_char,
}

impl CurlApi {
    /// # Safety
    /// Caller must ensure the loaded library is ABI-compatible with the symbols used.
    pub unsafe fn load(path: &Path) -> Result<Self, SysError> {
        debug!(path = %path.display(), "loading curl-impersonate library");
        let lib = unsafe { Library::new(path) }.map_err(|source| SysError::LoadLibrary {
            path: path.to_path_buf(),
            source,
        })?;

        let global_init = unsafe {
            load_symbol::<unsafe extern "C" fn(c_ulong) -> CurlCode>(&lib, b"curl_global_init\0")?
        };
        let global_cleanup =
            unsafe { load_symbol::<unsafe extern "C" fn()>(&lib, b"curl_global_cleanup\0")? };
        let easy_init = unsafe {
            load_symbol::<unsafe extern "C" fn() -> *mut Curl>(&lib, b"curl_easy_init\0")?
        };
        let easy_cleanup = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut Curl)>(&lib, b"curl_easy_cleanup\0")?
        };
        let easy_perform = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut Curl) -> CurlCode>(
                &lib,
                b"curl_easy_perform\0",
            )?
        };
        let easy_setopt = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut Curl, CurlOption, ...) -> CurlCode>(
                &lib,
                b"curl_easy_setopt\0",
            )?
        };
        let easy_getinfo = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut Curl, c_uint, ...) -> CurlCode>(
                &lib,
                b"curl_easy_getinfo\0",
            )?
        };
        let easy_strerror = unsafe {
            load_symbol::<unsafe extern "C" fn(CurlCode) -> *const c_char>(
                &lib,
                b"curl_easy_strerror\0",
            )?
        };
        let easy_impersonate = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut Curl, *const c_char, c_int) -> CurlCode>(
                &lib,
                b"curl_easy_impersonate\0",
            )?
        };
        let slist_append = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlSlist, *const c_char) -> *mut CurlSlist>(
                &lib,
                b"curl_slist_append\0",
            )?
        };
        let slist_free_all = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlSlist)>(&lib, b"curl_slist_free_all\0")?
        };
        let ws_send = unsafe {
            load_symbol::<
                unsafe extern "C" fn(
                    *mut Curl,
                    *const c_void,
                    usize,
                    *mut usize,
                    i64,
                    c_uint,
                ) -> CurlCode,
            >(&lib, b"curl_ws_send\0")?
        };
        let ws_recv = unsafe {
            load_symbol::<
                unsafe extern "C" fn(
                    *mut Curl,
                    *mut c_void,
                    usize,
                    *mut usize,
                    *mut *const CurlWsFrame,
                ) -> CurlCode,
            >(&lib, b"curl_ws_recv\0")?
        };
        let multi_init = unsafe {
            load_symbol::<unsafe extern "C" fn() -> *mut CurlMulti>(&lib, b"curl_multi_init\0")?
        };
        let multi_cleanup = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti) -> CurlMCode>(
                &lib,
                b"curl_multi_cleanup\0",
            )?
        };
        let multi_setopt = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, CurlMOption, ...) -> CurlMCode>(
                &lib,
                b"curl_multi_setopt\0",
            )?
        };
        let multi_add_handle = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, *mut Curl) -> CurlMCode>(
                &lib,
                b"curl_multi_add_handle\0",
            )?
        };
        let multi_remove_handle = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, *mut Curl) -> CurlMCode>(
                &lib,
                b"curl_multi_remove_handle\0",
            )?
        };
        let multi_fdset = unsafe {
            load_symbol::<
                unsafe extern "C" fn(
                    *mut CurlMulti,
                    *mut c_void,
                    *mut c_void,
                    *mut c_void,
                    *mut c_int,
                ) -> CurlMCode,
            >(&lib, b"curl_multi_fdset\0")?
        };
        let multi_timeout = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, *mut c_long) -> CurlMCode>(
                &lib,
                b"curl_multi_timeout\0",
            )?
        };
        let multi_perform = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, *mut c_int) -> CurlMCode>(
                &lib,
                b"curl_multi_perform\0",
            )?
        };
        let multi_poll = unsafe {
            load_symbol::<
                unsafe extern "C" fn(
                    *mut CurlMulti,
                    *mut CurlWaitFd,
                    c_uint,
                    c_int,
                    *mut c_int,
                ) -> CurlMCode,
            >(&lib, b"curl_multi_poll\0")?
        };
        let multi_socket_action = unsafe {
            load_symbol::<
                unsafe extern "C" fn(*mut CurlMulti, CurlSocket, c_int, *mut c_int) -> CurlMCode,
            >(&lib, b"curl_multi_socket_action\0")?
        };
        let multi_info_read = unsafe {
            load_symbol::<unsafe extern "C" fn(*mut CurlMulti, *mut c_int) -> *mut CurlMessage>(
                &lib,
                b"curl_multi_info_read\0",
            )?
        };
        let multi_strerror = unsafe {
            load_symbol::<unsafe extern "C" fn(CurlMCode) -> *const c_char>(
                &lib,
                b"curl_multi_strerror\0",
            )?
        };

        info!(path = %path.display(), "curl-impersonate library loaded");
        Ok(Self {
            _lib: ManuallyDrop::new(lib),
            global_init,
            global_cleanup,
            easy_init,
            easy_cleanup,
            easy_perform,
            easy_setopt,
            easy_getinfo,
            easy_strerror,
            easy_impersonate,
            slist_append,
            slist_free_all,
            ws_send,
            ws_recv,
            multi_init,
            multi_cleanup,
            multi_setopt,
            multi_add_handle,
            multi_remove_handle,
            multi_fdset,
            multi_timeout,
            multi_perform,
            multi_poll,
            multi_socket_action,
            multi_info_read,
            multi_strerror,
        })
    }

    pub fn error_text(&self, code: CurlCode) -> String {
        unsafe {
            let ptr = (self.easy_strerror)(code);
            if ptr.is_null() {
                return format!("CURLcode {}", code);
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }

    pub fn multi_error_text(&self, code: CurlMCode) -> String {
        unsafe {
            let ptr = (self.multi_strerror)(code);
            if ptr.is_null() {
                return format!("CURLMcode {}", code);
            }
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    }
}

// CurlApi only contains function pointers and a ManuallyDrop<Library> (never unloaded).
// All function pointers are process-global symbols — safe to share across threads.
unsafe impl Send for CurlApi {}
unsafe impl Sync for CurlApi {}

static SHARED_API: OnceLock<Arc<CurlApi>> = OnceLock::new();

/// Get or initialize the process-wide shared CurlApi instance.
/// First call loads the library; subsequent calls return the cached Arc.
pub fn shared_curl_api(lib_path: &Path) -> Result<Arc<CurlApi>, SysError> {
    if let Some(api) = SHARED_API.get() {
        return Ok(Arc::clone(api));
    }
    let api = unsafe { CurlApi::load(lib_path) }?;
    let arc = Arc::new(api);
    // Race is fine — loser's CurlApi uses ManuallyDrop so no resource leak.
    let _ = SHARED_API.set(Arc::clone(&arc));
    Ok(SHARED_API.get().map(Arc::clone).unwrap_or(arc))
}

impl CurlMessage {
    /// # Safety
    /// Caller must only use this for messages where `msg == CURLMSG_DONE`.
    pub unsafe fn done_result(&self) -> CurlCode {
        unsafe { self.data.result }
    }
}

unsafe fn load_symbol<T: Copy>(lib: &Library, name: &[u8]) -> Result<T, SysError> {
    let symbol = unsafe { lib.get::<T>(name) }.map_err(|source| SysError::MissingSymbol {
        name: String::from_utf8_lossy(name)
            .trim_end_matches('\0')
            .to_owned(),
        source,
    })?;
    Ok(*symbol)
}

pub fn platform_library_names() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["libcurl-impersonate.4.dylib", "libcurl-impersonate.dylib"]
    } else if cfg!(target_os = "linux") {
        &["libcurl-impersonate.so.4", "libcurl-impersonate.so"]
    } else if cfg!(target_os = "windows") {
        &[
            "curl-impersonate.dll",
            "libcurl-impersonate.dll",
            "libcurl.dll",
        ]
    } else {
        &[
            "libcurl-impersonate.4.dylib",
            "libcurl-impersonate.so.4",
            "curl-impersonate.dll",
        ]
    }
}

pub fn find_near_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    for name in platform_library_names() {
        let in_lib = exe_dir.join("..").join("lib").join(name);
        if in_lib.exists() {
            return Some(in_lib);
        }
        let side_by_side = exe_dir.join(name);
        if side_by_side.exists() {
            return Some(side_by_side);
        }
    }
    None
}

fn probe_library_dir(dir: &Path, searched: &mut Vec<PathBuf>) -> Option<PathBuf> {
    for name in platform_library_names() {
        let candidate = dir.join(name);
        searched.push(candidate.clone());
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn default_library_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(dir) = std::env::var("IMPCURL_LIB_DIR") {
        roots.push(PathBuf::from(dir));
    }

    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(&home).join(".impcurl/lib"));
        roots.push(PathBuf::from(home).join(".cuimp/binaries"));
    }

    roots
}

/// Resolve a usable CA bundle file path for TLS verification.
///
/// Resolution order:
/// 1. `CURL_CA_BUNDLE`
/// 2. `SSL_CERT_FILE`
/// 3. platform defaults (`/etc/ssl/certs/ca-certificates.crt` first on Linux)
pub fn resolve_ca_bundle_path() -> Option<PathBuf> {
    for key in ["CURL_CA_BUNDLE", "SSL_CERT_FILE"] {
        if let Ok(value) = std::env::var(key) {
            let candidate = PathBuf::from(value);
            if candidate.is_file() {
                debug!(path = %candidate.display(), env = key, "resolved CA bundle from env");
                return Some(candidate);
            }
        }
    }

    for candidate in default_ca_bundle_candidates() {
        if candidate.is_file() {
            debug!(path = %candidate.display(), "resolved CA bundle from platform defaults");
            return Some(candidate);
        }
    }

    None
}

fn default_ca_bundle_candidates() -> Vec<PathBuf> {
    if cfg!(target_os = "linux") {
        vec![
            PathBuf::from("/etc/ssl/certs/ca-certificates.crt"),
            PathBuf::from("/etc/pki/tls/certs/ca-bundle.crt"),
            PathBuf::from("/etc/ssl/cert.pem"),
            PathBuf::from("/etc/pki/tls/cert.pem"),
            PathBuf::from("/etc/ssl/ca-bundle.pem"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/etc/ssl/cert.pem")]
    } else {
        Vec::new()
    }
}

fn auto_fetch_enabled() -> bool {
    match std::env::var("IMPCURL_AUTO_FETCH") {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

fn auto_fetch_cache_dir() -> Result<PathBuf, SysError> {
    if let Ok(dir) = std::env::var("IMPCURL_AUTO_FETCH_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(dir) = std::env::var("IMPCURL_LIB_DIR") {
        return Ok(PathBuf::from(dir));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join(".impcurl/lib"));
    }
    Err(SysError::AutoFetchCacheDirUnavailable)
}

fn current_target_triple() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu"
    )) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86",
        target_env = "gnu"
    )) {
        "i686-unknown-linux-gnu"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "gnu"
    )) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "musl"
    )) {
        "x86_64-unknown-linux-musl"
    } else if cfg!(all(
        target_os = "linux",
        target_arch = "aarch64",
        target_env = "musl"
    )) {
        "aarch64-unknown-linux-musl"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "x86")) {
        "i686-pc-windows-msvc"
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        "aarch64-pc-windows-msvc"
    } else {
        "unknown"
    }
}

fn asset_target_for_triple(target: &str) -> Option<&'static str> {
    match target {
        "x86_64-apple-darwin" => Some("x86_64-apple-darwin"),
        "aarch64-apple-darwin" => Some("aarch64-apple-darwin"),
        "x86_64-unknown-linux-gnu" => Some("x86_64-unknown-linux-gnu"),
        "aarch64-unknown-linux-gnu" => Some("aarch64-unknown-linux-gnu"),
        "x86_64-unknown-linux-musl" => Some("x86_64-unknown-linux-musl"),
        "aarch64-unknown-linux-musl" => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

fn asset_version() -> String {
    std::env::var("IMPCURL_LIBCURL_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned())
}

fn asset_repo() -> String {
    std::env::var("IMPCURL_LIBCURL_REPO").unwrap_or_else(|_| "tuchg/impcurl".to_owned())
}

fn asset_release_tag(version: &str) -> String {
    format!("impcurl-libcurl-impersonate-v{version}")
}

fn asset_name(version: &str, target: &str) -> String {
    format!("impcurl-libcurl-impersonate-v{version}-{target}.tar.gz")
}

fn asset_cache_dir(base: &Path, version: &str, target: &str) -> PathBuf {
    base.join("libcurl-impersonate-assets")
        .join(version)
        .join(target)
}

fn asset_url(repo: &str, tag: &str, asset_name: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset_name}")
}

fn run_download_command(command: &mut Command, command_label: &str) -> Result<Vec<u8>, SysError> {
    let output = command
        .output()
        .map_err(|source| SysError::AutoFetchCommandSpawn {
            command: command_label.to_owned(),
            source,
        })?;

    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Err(SysError::AutoFetchCommandFailed {
        command: command_label.to_owned(),
        status: output.status.code(),
        stderr,
    })
}

fn fetch_url_to_file(url: &str, output_path: &Path) -> Result<(), SysError> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let output_str = output_path.to_string_lossy().to_string();

    let mut curl_cmd = Command::new("curl");
    curl_cmd
        .arg("-fL")
        .arg("-o")
        .arg(&output_str)
        .arg("-H")
        .arg("User-Agent: impcurl-sys")
        .arg(url);
    match run_download_command(&mut curl_cmd, "curl") {
        Ok(_) => return Ok(()),
        Err(SysError::AutoFetchCommandSpawn { .. }) => {}
        Err(err) => return Err(err),
    }

    let mut wget_cmd = Command::new("wget");
    wget_cmd.arg("-O").arg(&output_str).arg(url);
    run_download_command(&mut wget_cmd, "wget")?;
    Ok(())
}

fn extract_tar_gz_archive(archive_path: &Path, output_dir: &Path) -> Result<(), SysError> {
    fs::create_dir_all(output_dir)?;
    let mut tar_cmd = Command::new("tar");
    tar_cmd
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(output_dir);
    let _ = run_download_command(&mut tar_cmd, "tar")?;
    Ok(())
}

fn auto_fetch_from_assets(cache_dir: &Path) -> Result<PathBuf, SysError> {
    let version = asset_version();
    let target_triple = current_target_triple().to_owned();
    let target = asset_target_for_triple(&target_triple)
        .ok_or_else(|| SysError::AutoFetchRuntimeUnsupportedTarget(target_triple.clone()))?;
    let repo = asset_repo();
    let output_dir = asset_cache_dir(cache_dir, &version, target);
    let tag = asset_release_tag(&version);
    let asset = asset_name(&version, target);
    let url = asset_url(&repo, &tag, &asset);
    let archive_path = cache_dir.join(format!(
        ".libcurl-impersonate-asset-{version}-{target}-{}.tar.gz",
        std::process::id()
    ));

    info!(
        cache_dir = %cache_dir.display(),
        output_dir = %output_dir.display(),
        url = %url,
        "auto-fetching libcurl-impersonate from asset release"
    );

    fetch_url_to_file(&url, &archive_path)?;
    extract_tar_gz_archive(&archive_path, &output_dir)?;
    let _ = fs::remove_file(&archive_path);

    let mut searched = Vec::new();
    probe_library_dir(&output_dir, &mut searched).ok_or_else(|| {
        SysError::AutoFetchNoStandaloneRuntime {
            cache_dir: output_dir,
        }
    })
}

pub fn resolve_impersonate_lib_path(extra_search_roots: &[PathBuf]) -> Result<PathBuf, SysError> {
    if let Ok(path) = std::env::var("CURL_IMPERSONATE_LIB") {
        let resolved = PathBuf::from(path);
        if resolved.exists() {
            debug!(path = %resolved.display(), "found via CURL_IMPERSONATE_LIB");
            return Ok(resolved);
        }
        return Err(SysError::MissingEnvPath(resolved));
    }

    if let Some(packaged) = find_near_executable() {
        debug!(path = %packaged.display(), "found near executable");
        return Ok(packaged);
    }

    let mut searched = Vec::new();
    for root in extra_search_roots {
        if let Some(found) = probe_library_dir(root, &mut searched) {
            return Ok(found);
        }
    }

    for root in default_library_search_roots() {
        if let Some(found) = probe_library_dir(&root, &mut searched) {
            return Ok(found);
        }
    }

    if auto_fetch_enabled() {
        let auto_fetch_result = (|| -> Result<PathBuf, SysError> {
            let cache_dir = auto_fetch_cache_dir()?;
            let target_triple = current_target_triple().to_owned();
            let target = asset_target_for_triple(&target_triple).ok_or_else(|| {
                SysError::AutoFetchRuntimeUnsupportedTarget(target_triple.clone())
            })?;
            let version = asset_version();
            let asset_dir = asset_cache_dir(&cache_dir, &version, target);
            if let Some(found) = probe_library_dir(&asset_dir, &mut searched) {
                return Ok(found);
            }
            if let Some(found) = probe_library_dir(&cache_dir, &mut searched) {
                return Ok(found);
            }

            auto_fetch_from_assets(&cache_dir)?;
            if let Some(found) = probe_library_dir(&asset_dir, &mut searched) {
                return Ok(found);
            }
            probe_library_dir(&cache_dir, &mut searched).ok_or_else(|| {
                SysError::AutoFetchNoStandaloneRuntime {
                    cache_dir: cache_dir.to_path_buf(),
                }
            })
        })();

        return match auto_fetch_result {
            Ok(found) => Ok(found),
            Err(err) => Err(SysError::LibraryNotFoundAfterAutoFetch {
                searched,
                auto_fetch_error: err.to_string(),
            }),
        };
    }

    Err(SysError::LibraryNotFound(searched))
}

#[cfg(test)]
mod tests {
    use super::{resolve_ca_bundle_path, asset_name, asset_release_tag};
    use std::{env, ffi::OsString};

    #[test]
    fn prefers_curl_ca_bundle_env_when_file_exists() {
        let fixture = env::current_exe().expect("current executable path should exist");
        let _guard_ssl = EnvGuard::set("SSL_CERT_FILE", None);
        let _guard_curl = EnvGuard::set("CURL_CA_BUNDLE", Some(fixture.as_os_str()));

        let resolved = resolve_ca_bundle_path().expect("expected env CA bundle to resolve");
        assert_eq!(resolved, fixture);
    }

    #[test]
    fn asset_naming_is_versioned() {
        assert_eq!(
            asset_release_tag("1.2.3"),
            "impcurl-libcurl-impersonate-v1.2.3"
        );
        assert_eq!(
            asset_name("1.2.3", "x86_64-unknown-linux-gnu"),
            "impcurl-libcurl-impersonate-v1.2.3-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, new: Option<&std::ffi::OsStr>) -> Self {
            let old = env::var_os(key);
            unsafe {
                match new {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.old.as_ref() {
                    Some(value) => env::set_var(self.key, value),
                    None => env::remove_var(self.key),
                }
            }
        }
    }
}
