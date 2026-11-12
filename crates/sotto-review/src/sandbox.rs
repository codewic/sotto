//! Run the renderer under bubblewrap with no network, no home directory, an
//! empty /etc (fonts and LibreOffice config only), memory and time limits.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ReviewError;

pub fn available() -> bool {
    Command::new("bwrap").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

fn root_dir_args(args: &mut Vec<String>, dir: &str, target: &str) {
    let p = Path::new(dir);
    if p.is_symlink() {
        args.extend(["--symlink".into(), target.into(), dir.into()]);
    } else if p.exists() {
        args.extend(["--ro-bind".into(), dir.into(), dir.into()]);
    }
}

/// Numeric uid and gid of this process, from /proc (no libc needed).
fn ids() -> (String, String) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|v| v.split_whitespace().next())
            .unwrap_or("65534")
            .to_string()
    };
    (field("Uid:"), field("Gid:"))
}

/// A synthetic passwd/group pair so programs that look up the current user (LibreOffice) work,
/// without exposing the real account database.
fn write_identity(dir: &Path) -> std::io::Result<()> {
    let (uid, gid) = ids();
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("passwd"), format!("sotto:x:{uid}:{gid}:sotto:/tmp:/usr/sbin/nologin\n"))?;
    std::fs::write(dir.join("group"), format!("sotto:x:{gid}:\n"))
}

/// Build the full command line (exposed for tests and the audit document).
/// `identity` holds the synthetic passwd and group files.
pub fn command(render: &Path, input: &Path, out: &Path, identity: &Path, mode: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = vec!["prlimit".into(), "--as=4294967296".into(), "--fsize=2147483648".into(), "--".into(), "bwrap".into()];
    a.extend(
        ["--unshare-all", "--die-with-parent", "--new-session", "--cap-drop", "ALL", "--clearenv", "--ro-bind", "/usr", "/usr"]
            .into_iter()
            .map(String::from),
    );
    root_dir_args(&mut a, "/lib", "usr/lib");
    root_dir_args(&mut a, "/lib64", "usr/lib64");
    root_dir_args(&mut a, "/bin", "usr/bin");
    root_dir_args(&mut a, "/sbin", "usr/sbin");
    for etc in ["/etc/fonts", "/etc/libreoffice", "/etc/ld.so.cache", "/etc/alternatives"] {
        a.extend(["--ro-bind-try".into(), etc.into(), etc.into()]);
    }
    for f in ["passwd", "group"] {
        a.extend(["--ro-bind-try".into(), identity.join(f).display().to_string(), format!("/etc/{f}")]);
    }
    a.extend(
        ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--setenv", "HOME", "/tmp", "--setenv", "PATH", "/usr/bin:/bin", "--setenv", "LANG", "C.UTF-8"]
            .into_iter()
            .map(String::from),
    );
    a.extend(["--ro-bind".into(), input.display().to_string(), "/in/file".into()]);
    a.extend(["--bind".into(), out.display().to_string(), "/out".into()]);
    a.extend(["--ro-bind".into(), render.display().to_string(), "/sotto-render".into()]);
    a.extend(["--chdir".into(), "/tmp".into(), "--".into(), "/sotto-render".into(), "/in/file".into(), "/out".into()]);
    a.extend(mode.map(String::from));
    a
}

/// Run the renderer. `out` must be an empty directory the caller owns.
pub fn run(render: &Path, input: &Path, out: &Path, timeout: Duration) -> Result<String, ReviewError> {
    run_mode(render, input, out, timeout, None)
}

/// Run the renderer in a mode (`photo`: one clean, upright image instead of pages).
pub fn run_mode(render: &Path, input: &Path, out: &Path, timeout: Duration, mode: Option<&str>) -> Result<String, ReviewError> {
    if !available() {
        return Err(ReviewError::NoSandbox);
    }
    let identity = out.with_extension("id");
    write_identity(&identity).map_err(|_| ReviewError::Io)?;
    let argv = command(render, input, out, &identity, mode);
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ReviewError::NoSandbox)?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ReviewError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err(ReviewError::NoSandbox),
        }
    }
    let _ = std::fs::remove_dir_all(&identity);
    let status = std::fs::read_to_string(out.join("status.txt")).unwrap_or_default();
    let status: String = status.chars().filter(|c| !c.is_control()).take(200).collect();
    match status.strip_prefix("ok ") {
        Some(kind) if ["pdf", "image", "office", "text", "photo"].contains(&kind) => Ok(kind.to_string()),
        _ => Err(ReviewError::Render(status.strip_prefix("error ").unwrap_or("the file could not be opened safely").to_string())),
    }
}

/// Where `sotto-render` lives: next to the running binary, unless overridden.
pub fn default_render_path() -> PathBuf {
    if let Ok(p) = std::env::var("SOTTO_RENDER") {
        return PathBuf::from(p);
    }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("sotto-render"))).unwrap_or_else(|| PathBuf::from("sotto-render"))
}
