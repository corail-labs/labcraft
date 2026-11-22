//! The control channel of the IDE (ide.md 8; mcp.md 1; architecture 12.8): how `labcraft-cli mcp` finds the window
//! of its project and speaks to it.
//!
//! **The file.** A window that serves its project listens on `127.0.0.1`, on a port the system gives, and writes one
//! file in the folder of the channel ([`folder`]): `<hash of the canonical folder of the project>.json`, holding the
//! port, a token new at each launch, the process and the project ([`Entry`]). Only the user can read it: mode `0600` in
//! a folder `0700` on Unix; on Windows, the inherited rights removed and the user alone given them (`icacls`), before
//! the token is written. The window removes it as it closes, if it still holds its token.
//!
//! **The connection.** Each side proves that it holds the token, which never goes on the connection ([`proof`]): the
//! client sends [`Hello`] with a nonce; the window answers [`Welcome`] with its proof for that nonce and a nonce of its
//! own; the client checks it and sends its [`Proof`]; the window answers [`Welcome`], `ok` or why not. Then the
//! connection carries MCP as `labcraft-cli mcp` gets it on stdin, one message per line, both ways: the server runs in
//! the window, on the session of the window, and `labcraft-cli mcp` relays ([`relay`]). A file whose port takes no
//! connection is left over from a window that did not close: the server starts headless. What takes the connection
//! and does not prove it holds the token stops the server: it is not the window, or not one to trust.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The folder of the channel, in place of the one of the user: for the tests, so that they never find a window of the
/// user (architecture 12.8). Empty, there is no channel: a window serves none, `mcp` always starts headless.
pub const FOLDER_VARIABLE: &str = "LABCRAFT_ATTACH_DIR";

/// The version of [`Hello`].
pub const VERSION: u32 = 1;

/// How long the window waits for the client to prove it holds the token *(proposed)*.
pub const HELLO_WAIT: Duration = Duration::from_secs(5);

/// How long `mcp` waits for the window to take the connection and prove it holds the token *(proposed)*.
pub const CONNECT_WAIT: Duration = Duration::from_secs(2);

/// The longest line of [`Hello`], [`Welcome`] or [`Proof`].
const MAX_HELLO: u64 = 1024;

/// The longest file of the channel read.
const MAX_FILE: u64 = 64 * 1024;

/// How long `whoami` or `icacls` may take before the file is given up *(proposed)*: a hung one must not hold the window.
#[cfg(windows)]
const ICACLS_WAIT: Duration = Duration::from_secs(5);

/// What the file of a window says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    /// The canonical folder of the project, as the window opened it.
    pub project: String,
}

/// The first line of a connection, from the client.
#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub labcraft_attach: u32,
    pub nonce: String,
}

/// A line of the window: `ok`, or why not; the first one has its proof and its nonce.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Welcome {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// The client's proof that it holds the token, for the nonce of the window.
#[derive(Debug, Serialize, Deserialize)]
pub struct Proof {
    pub proof: String,
}

/// The folder of the channel: [`FOLDER_VARIABLE`], else `labcraft/attach` in the local data of the user; none when
/// there is no channel.
pub fn folder() -> Option<PathBuf> {
    match std::env::var_os(FOLDER_VARIABLE) {
        Some(given) if given.is_empty() => None,
        Some(given) => Some(PathBuf::from(given)),
        None => dirs::data_local_dir().map(|data| data.join("labcraft").join("attach")),
    }
}

/// The file of the window of the project at `root`, a canonical folder, in `folder`: named by the first 32 digits of
/// the SHA-256 of its path, as a revision gives it (commands.md 9.1).
pub fn file_of(folder: &Path, root: &Path) -> PathBuf {
    let revision = labcraft_engine::revision(root.to_string_lossy().as_bytes());
    let digits = revision.strip_prefix("sha256:").unwrap_or(&revision);
    folder.join(format!("{}.json", digits.get(..32).unwrap_or(digits)))
}

/// A token for one launch: 32 random bytes, in hexadecimal.
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| format!("no random bytes for the token: {error}"))?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut text, byte| {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
        text
    }))
}

/// `value` as one line of JSON, with its end.
fn line_of(value: &impl Serialize) -> String {
    let mut line = serde_json::to_string(value).unwrap_or_default();
    line.push('\n');
    line
}

/// Whether two tokens are the same, in a time that does not depend on where they differ.
pub fn tokens_match(given: &str, held: &str) -> bool {
    let (given, held) = (given.as_bytes(), held.as_bytes());
    given.len() == held.len() && given.iter().zip(held).fold(0_u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// The file of a window, written; removed as it is dropped, if it still holds this token.
pub struct Published {
    path: PathBuf,
    token: String,
}

impl Published {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for Published {
    fn drop(&mut self) {
        // Another window of the same project may have written it since: its file stays.
        if read_entry(&self.path).is_some_and(|entry| tokens_match(&entry.token, &self.token)) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Writes the file of the window of the project at `root` that listens on `port`, readable by the user only.
pub fn publish(folder: &Path, root: &Path, port: u16) -> Result<Published, String> {
    let token = new_token()?;
    let entry = Entry { port, token: token.clone(), pid: std::process::id(), project: root.to_string_lossy().into_owned() };
    let text = serde_json::to_string(&entry).map_err(|error| error.to_string())?;
    make_folder(folder)?;
    let path = file_of(folder, root);
    // A file left over goes: the new one is made by this process, with its rights, before the token is in it.
    let _ = std::fs::remove_file(&path);
    let mut file = create_private(&path)?;
    let published = Published { path, token };
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("`{}` cannot be written: {error}", published.path.display()))?;
    Ok(published)
}

fn make_folder(folder: &Path) -> Result<(), String> {
    let made = |error: std::io::Error| format!("`{}` cannot be made: {error}", folder.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(folder).map_err(made)?;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700)).map_err(made)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(folder).map_err(made)
    }
}

/// A new file that only the user can read, still empty.
fn create_private(path: &Path) -> Result<std::fs::File, String> {
    let failed = |error: std::io::Error| format!("`{}` cannot be made: {error}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(failed)
    }
    #[cfg(not(unix))]
    {
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(failed)?;
        #[cfg(windows)]
        if let Err(why) = user_only(path) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(format!("`{}` cannot be made readable by the user only: {why}", path.display()));
        }
        Ok(file)
    }
}

/// Takes the inherited rights off the file at `path` and gives the user alone full control, by `icacls`, the tool of
/// Windows for it, naming the user by the SID that `whoami` gives: the code stays without `unsafe`.
#[cfg(windows)]
fn user_only(path: &Path) -> Result<(), String> {
    let user = run_for_a_while(std::process::Command::new("whoami").args(["/user", "/fo", "csv", "/nh"]))?;
    // `"machine\user","S-1-5-21-…"`.
    let sid = user.trim().rsplit(',').next().map(|field| field.trim_matches('"')).filter(|sid| sid.starts_with("S-1-"));
    let Some(sid) = sid else { return Err(format!("whoami gave no SID: {}", user.trim())) };
    let mut icacls = std::process::Command::new("icacls");
    icacls.arg(path).args(["/inheritance:r", "/grant:r"]).arg(format!("*{sid}:F"));
    run_for_a_while(&mut icacls).map(|_| ())
}

/// Runs `command` with no window, at most [`ICACLS_WAIT`]: what it wrote, once it ended well.
#[cfg(windows)]
#[expect(clippy::disallowed_methods, reason = "Windows only, never WASM")]
fn run_for_a_while(command: &mut std::process::Command) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let name = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("{name}: {error}"))?;
    let deadline = std::time::Instant::now() + ICACLS_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut out);
                }
                return Ok(out);
            }
            Ok(Some(status)) => return Err(format!("{name} ended with {status}")),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} did not end in {} s", ICACLS_WAIT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(format!("{name}: {error}")),
        }
    }
}

/// At most this many files of the folder looked at, and projects named, by [`other_projects`] *(proposed)*.
const OTHERS_LOOKED_AT: usize = 64;
const OTHERS_NAMED: usize = 4;

/// The projects that the windows of LabCraft show, other than the one at `root`, as their files say, by name: what
/// `mcp` tells an agent that works on a project no window shows (mcp.md 1).
pub fn other_projects(root: &Path) -> Vec<String> {
    let Some(entries) = folder().and_then(|folder| std::fs::read_dir(folder).ok()) else { return Vec::new() };
    let mut projects: Vec<String> = entries
        .flatten()
        .take(OTHERS_LOOKED_AT)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "json"))
        .filter_map(|entry| read_entry(&entry.path()))
        .map(|entry| entry.project)
        .filter(|project| Path::new(project) != root)
        .collect();
    projects.sort();
    projects.dedup();
    projects.truncate(OTHERS_NAMED);
    projects
}

/// What the file at `path` says, when it reads.
pub fn read_entry(path: &Path) -> Option<Entry> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_FILE).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// The window of the project at `root`, a canonical folder, as its file says: none when there is no file, or one that
/// does not read, or one of another project.
pub fn find(root: &Path) -> Option<Entry> {
    let entry = read_entry(&file_of(&folder()?, root))?;
    (Path::new(&entry.project) == root).then_some(entry)
}

/// Reads one line from `stream`, a byte at a time so that nothing that follows it is taken, all of it before
/// `deadline`: at most [`MAX_HELLO`] bytes. Why not otherwise.
#[expect(clippy::disallowed_methods, reason = "the command line and the desktop app, never built for WASM")]
fn read_line_by(stream: &mut TcpStream, deadline: std::time::Instant) -> Result<String, String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err("no whole line in time".to_owned());
        }
        stream.set_read_timeout(Some(left)).map_err(|error| error.to_string())?;
        match stream.read(&mut byte) {
            Ok(1) => {
                let [read] = byte;
                if read == b'\n' {
                    return String::from_utf8(line).map_err(|_| "a line that is not UTF-8".to_owned());
                }
                line.push(read);
                if line.len() as u64 >= MAX_HELLO {
                    return Err("a line too long".to_owned());
                }
            }
            Ok(_) => return Err("the connection closed".to_owned()),
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn write_line(stream: &mut TcpStream, value: &impl Serialize) -> Result<(), String> {
    stream.write_all(line_of(value).as_bytes()).and_then(|()| stream.flush()).map_err(|error| error.to_string())
}

/// The proof that `side` (`window` or `client`) holds `token`, for the nonce of the other side: the hexadecimal digits
/// of the SHA-256 of the three, so that the token itself never goes on the connection.
pub fn proof(side: &str, nonce: &str, token: &str) -> String {
    let revision = labcraft_engine::revision(format!("labcraft-attach:{side}:{nonce}:{token}").as_bytes());
    revision.strip_prefix("sha256:").unwrap_or(&revision).to_owned()
}

/// The window's side of a connection it took (`stream`): [`Hello`], its proof and its nonce, the client's proof, then
/// [`Welcome`], `ok` once `admit` takes the client. All of it within [`HELLO_WAIT`]. Why not, which the client is told.
#[expect(clippy::disallowed_methods, reason = "the desktop app, never built for WASM")]
pub fn greet(stream: &mut TcpStream, token: &str, admit: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + HELLO_WAIT;
    let refuse = |stream: &mut TcpStream, why: String| {
        let _ = write_line(stream, &Welcome { ok: false, error: Some(why.clone()), ..Welcome::default() });
        Err(why)
    };
    let hello: Hello = match read_line_by(stream, deadline).and_then(|line| serde_json::from_str(&line).map_err(|_| String::new())) {
        Ok(hello) => hello,
        Err(_) => return refuse(stream, "the first line is not the hello of `labcraft-cli mcp`".to_owned()),
    };
    if hello.labcraft_attach != VERSION {
        return refuse(stream, format!("version {} of the channel, and this window speaks {VERSION}", hello.labcraft_attach));
    }
    let nonce = new_token()?;
    write_line(stream, &Welcome { ok: true, proof: Some(proof("window", &hello.nonce, token)), nonce: Some(nonce.clone()), ..Welcome::default() })?;
    let answer: Proof = match read_line_by(stream, deadline).and_then(|line| serde_json::from_str(&line).map_err(|_| String::new())) {
        Ok(answer) => answer,
        Err(_) => return refuse(stream, "no proof of the token".to_owned()),
    };
    if !tokens_match(&answer.proof, &proof("client", &nonce, token)) {
        return refuse(stream, "the token is not this window's".to_owned());
    }
    if let Err(why) = admit() {
        return refuse(stream, why);
    }
    write_line(stream, &Welcome { ok: true, ..Welcome::default() })?;
    stream.set_read_timeout(None).map_err(|error| error.to_string())
}

/// Why `mcp` did not attach to a window.
#[derive(Debug)]
pub enum NotAttached {
    /// No window takes the connection: its file is left over, and the server starts headless.
    NoWindow,
    /// What took the connection refused it, or did not prove that it is the window: the server does not start, since
    /// it would edit the window's project from a session of its own, or give the agent what another process says.
    Refused(String),
}

/// Connects to the window of `entry`: each side proves that it holds the token ([`proof`]), then the connection is
/// ready for MCP.
#[expect(clippy::disallowed_methods, reason = "the command line, never built for WASM")]
pub fn connect(entry: &Entry) -> Result<TcpStream, NotAttached> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, entry.port));
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_WAIT).map_err(|_| NotAttached::NoWindow)?;
    let refused = |why: String| NotAttached::Refused(why);
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    let nonce = new_token().map_err(refused)?;
    write_line(&mut stream, &Hello { labcraft_attach: VERSION, nonce: nonce.clone() }).map_err(refused)?;
    let read = |stream: &mut TcpStream| -> Result<Welcome, NotAttached> {
        let line = read_line_by(stream, deadline).map_err(|why| refused(format!("port {}: {why}", entry.port)))?;
        let welcome: Welcome = serde_json::from_str(&line).map_err(|_| refused(format!("port {} does not speak as a window", entry.port)))?;
        match welcome {
            Welcome { ok: false, error, .. } => Err(refused(error.unwrap_or_else(|| "refused".to_owned()))),
            welcome => Ok(welcome),
        }
    };
    let challenge = read(&mut stream)?;
    let window = challenge.proof.as_deref().unwrap_or_default();
    if !tokens_match(window, &proof("window", &nonce, &entry.token)) {
        return Err(refused(format!("the process on port {} does not hold the token of the window", entry.port)));
    }
    let theirs = challenge.nonce.unwrap_or_default();
    write_line(&mut stream, &Proof { proof: proof("client", &theirs, &entry.token) }).map_err(refused)?;
    read(&mut stream)?;
    stream.set_read_timeout(None).map_err(|error| refused(error.to_string()))?;
    Ok(stream)
}

/// Relays `input` to the window and the window to `output`, until the window closes the connection. When `input`
/// closes first, the window is told, and its last answers still come: `Ok`. When the window closes first, the agent
/// is left with no server: an error, and the thread that reads `input` is left behind, as the process ends.
pub fn relay(stream: TcpStream, mut input: Box<dyn Read + Send>, output: &mut dyn Write) -> std::io::Result<()> {
    let mut to_window = stream.try_clone()?;
    let mut from_window = stream;
    let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let input_ended = ended.clone();
    std::thread::Builder::new().name("mcp stdin".to_owned()).spawn(move || {
        let _ = std::io::copy(&mut input, &mut to_window);
        input_ended.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = to_window.shutdown(std::net::Shutdown::Write);
    })?;
    copy_lines(&mut from_window, output)?;
    if ended.load(std::sync::atomic::Ordering::SeqCst) { Ok(()) } else { Err(std::io::Error::other("the window closed the connection")) }
}

/// Copies what the window says to `output`, flushed at each line: an agent waits for each answer.
fn copy_lines(from: &mut TcpStream, output: &mut dyn Write) -> std::io::Result<()> {
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = from.read(&mut buffer)?;
        if read == 0 {
            return output.flush();
        }
        output.write_all(buffer.get(..read).unwrap_or_default())?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, empty, removed with it.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let folder = std::env::temp_dir().join(format!("labcraft-attach-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        Scratch(folder)
    }

    #[test]
    fn tokens_match_only_when_they_are_equal() {
        assert!(tokens_match("abc", "abc"));
        assert!(!tokens_match("abc", "abd"));
        assert!(!tokens_match("abc", "abcd"));
        assert!(!tokens_match("", "a"));
    }

    #[test]
    fn a_token_is_64_hexadecimal_digits_and_new_each_time() {
        let (one, two) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(one.len(), 64);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(one, two);
    }

    #[test]
    fn a_published_file_is_found_and_goes_with_its_window() {
        let scratch = scratch("found");
        let root = scratch.0.join("project");
        let published = publish(&scratch.0, &root, 4321).unwrap();
        let entry = read_entry(published.path()).unwrap();
        assert_eq!((entry.port, entry.pid, entry.project.as_str()), (4321, std::process::id(), &*root.to_string_lossy()));
        assert_eq!(entry.token, published.token());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(published.path()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let path = published.path().to_path_buf();
        drop(published);
        assert!(!path.exists());
    }

    #[test]
    fn a_window_that_closes_leaves_the_file_of_a_newer_one() {
        let scratch = scratch("newer");
        let root = scratch.0.join("project");
        let older = publish(&scratch.0, &root, 1).unwrap();
        let newer = publish(&scratch.0, &root, 2).unwrap();
        drop(older);
        assert_eq!(read_entry(newer.path()).unwrap().port, 2);
    }

    #[test]
    fn each_project_has_its_file() {
        let folder = Path::new("/channel");
        assert_ne!(file_of(folder, Path::new("/a")), file_of(folder, Path::new("/b")));
        assert_eq!(file_of(folder, Path::new("/a")), file_of(folder, Path::new("/a")));
    }
}
