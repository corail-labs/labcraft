//! `cargo xtask web` builds the web page (`docs/architecture.md`, 12.9) with Trunk into `target/web`, after clippy for
//! WASM; `cargo xtask web-smoke` serves that folder on the loopback and opens it in headless Chrome, whose page says how
//! it went (`?smoke`): it drew a function, trapped its worker, started a new one and drew again; then (scenario 8 of
//! `docs/acceptance-m1.md`) it added a node and wired it, undid it to the bytes before and redid it, set a setting of
//! the user and one of the project, and posted the zip of `Download`, which the server unpacks and checks; the page then
//! loads itself again, and says whether the setting of the user stayed. `cargo xtask web-serve` serves it, to try it in a
//! browser. None is part of `ci`, which needs neither Trunk nor a browser: the job `web` of the workflow `checks` runs
//! the first two.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The version of Trunk that the job `web` installs, as SolidCraft and GridCraft pin it.
pub const TRUNK: &str = "0.21.14";

/// What the page must give, besides itself.
const BUILT: [&str; 5] = ["index.html", "labcraft-web_bg.wasm", "labcraft-web.js", "labcraft-worker_bg.wasm", "labcraft-worker_loader.js"];

/// The longest line of a request, and the most headers, that the server reads.
const LINE: u64 = 8 * 1024;
const HEADERS: usize = 64;

/// How long the smoke test waits for the page to report, both of its loads.
const PATIENCE: Duration = Duration::from_secs(300);

/// The most bytes of a report of the page, and of its zip.
const REPORT: usize = 64 * 1024;
const ZIP: usize = 64 * 1024 * 1024;

/// The setting of the project that the page sets (`apps/labcraft-web/src/page/smoke.rs`), which its zip must hold: its
/// table, its key, its value.
const PROJECT_KEY: (&str, &str, bool) = ("editor", "drag_pans", true);
/// The call that the page adds, and the value it wires to it (`apps/labcraft-web/src/page/smoke.rs`).
const CALL: (&str, &str) = ("core.not", "in_range");

fn loopback(error: std::io::Error) -> String {
    format!("the loopback: {error}")
}

/// The site built, or what to run first.
fn built(root: &Path) -> Result<PathBuf, String> {
    let site = root.join("target").join("web");
    if site.join("index.html").is_file() { Ok(site) } else { Err(format!("{} has no page: run `cargo xtask web` first", site.display())) }
}

pub fn build(root: &Path) -> Result<(), String> {
    let found = Command::new("trunk").arg("--version").stdout(Stdio::piped()).stderr(Stdio::null()).output();
    let version = match found {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        _ => {
            return Err(format!(
                "trunk is missing; install it with
  cargo install trunk --version {TRUNK} --locked"
            ));
        }
    };
    if !version.ends_with(TRUNK) {
        eprintln!("web: {version}, where the job `web` installs trunk {TRUNK}");
    }
    super::run(&[
        "clippy",
        "--locked",
        "--target",
        super::WASM_TARGET,
        "--package",
        "labcraft-web",
        "--all-targets",
        "--no-deps",
        "--",
        "-D",
        "warnings",
    ])?;
    eprintln!("$ trunk build --release --locked (apps/labcraft-web)");
    let status = Command::new("trunk")
        .args(["build", "--release", "--locked"])
        .current_dir(root.join("apps").join("labcraft-web"))
        .status()
        .map_err(|error| format!("trunk: {error}"))?;
    if !status.success() {
        return Err(format!("trunk build: {status}"));
    }
    let site = built(root)?;
    for name in BUILT {
        let path = site.join(name);
        let size = std::fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?.len();
        println!("web: {name}, {} KiB", size / 1024);
    }
    Ok(())
}

/// Chrome or Chromium: `CHROME` when it is set, then the usual names and places.
fn chrome() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CHROME") {
        return Ok(PathBuf::from(path));
    }
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"];
    for name in names {
        if Command::new(name).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success()) {
            return Ok(PathBuf::from(name));
        }
    }
    let places = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ];
    places.iter().map(PathBuf::from).find(|path| path.is_file()).ok_or_else(|| "no Chrome or Chromium found: set CHROME to its path".to_owned())
}

/// Ends the browser however the test ends.
struct Browser(Child);

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn smoke(root: &Path) -> Result<(), String> {
    // As a browser opens it, then with WebGL2 asked for: a runner has no GPU, so the first falls back to WebGL2 too.
    smoke_once(root, "?smoke")?;
    smoke_once(root, "?smoke&webgl")
}

#[expect(clippy::disallowed_methods, reason = "xtask runs natively, where the clock does not trap")]
fn smoke_once(root: &Path, query: &str) -> Result<(), String> {
    let site = built(root)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(loopback)?;
    listener.set_nonblocking(true).map_err(loopback)?;
    let port = listener.local_addr().map_err(loopback)?.port();
    let profile = root.join("target").join("web-smoke-profile");
    let _ = std::fs::remove_dir_all(&profile);
    let url = format!("http://127.0.0.1:{port}/{query}");
    let browser = chrome()?;
    eprintln!("$ {} --headless=new … {url}", browser.display());
    let child = Command::new(&browser)
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            // WebGL2 on SwiftShader, which every runner has.
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{}: {error}", browser.display()))?;
    let _browser = Browser(child);
    let deadline = Instant::now() + PATIENCE;
    let mut served = 0usize;
    let mut zip_checked = false;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => match answer(stream, &site, &mut served) {
                Some(Said::Report(report)) => {
                    println!("web-smoke: {report} ({served} files served)");
                    return match (report.starts_with("ok:"), zip_checked) {
                        (true, true) => Ok(()),
                        (true, false) => Err("web-smoke: the page reported before the server had its zip".to_owned()),
                        (false, _) => Err(format!("web-smoke: {report}")),
                    };
                }
                Some(Said::Step(step)) => println!("web-smoke: {step}"),
                Some(Said::Zip { bytes, name, file, node }) => {
                    let checked = check_zip(root, &bytes, &name, &file, &node).map_err(|why| format!("web-smoke: the zip of `Download`: {why}"))?;
                    println!("web-smoke: the zip of `Download`: {checked}");
                    zip_checked = true;
                }
                None => {}
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => return Err(format!("the loopback: {error}")),
        }
    }
    Err(format!("web-smoke: the page said nothing within {} s ({served} files served)", PATIENCE.as_secs()))
}

/// Serves `target/web` on the loopback until the process is stopped, to try the viewer in a browser.
pub fn serve(root: &Path) -> Result<(), String> {
    let site = built(root)?;
    let listener = TcpListener::bind("127.0.0.1:8770").or_else(|_| TcpListener::bind("127.0.0.1:0")).map_err(loopback)?;
    let port = listener.local_addr().map_err(loopback)?.port();
    println!("web-serve: http://127.0.0.1:{port}/ (?webgl for WebGL2); Ctrl+C stops it");
    let mut served = 0usize;
    for stream in listener.incoming().flatten() {
        let _ = answer(stream, &site, &mut served);
    }
    Ok(())
}

/// The file of the site that a path of a request names: only names of the site, no `..`, no drive or root (`C:`, `\\`),
/// nothing that leaves `target/web`.
fn site_file<'a>(site: &Path, path: &'a str) -> (&'a str, Option<PathBuf>) {
    let name = if path == "/" { "index.html" } else { path.strip_prefix('/').unwrap_or(path) };
    let plain = |part: &str| !part.is_empty() && part != "." && part != ".." && !part.contains([':', '\\']);
    (name, name.split('/').all(plain).then(|| site.join(name)))
}

/// What the page posts to the server that served it.
#[derive(Debug, PartialEq)]
enum Said {
    /// How the smoke test went: it ends there.
    Report(String),
    /// What it did before it loads the page again.
    Step(String),
    /// The zip of `Download`, with the name the page saves it under, the file the test edited and the name of the node
    /// it added.
    Zip { bytes: Vec<u8>, name: String, file: String, node: String },
}

/// Answers one request: a file of the site, or what the page says, which it returns.
fn answer(stream: TcpStream, site: &Path, served: &mut usize) -> Option<Said> {
    let _ = stream.set_nonblocking(false);
    // One request at a time: a client that says nothing holds the server for this long at most.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    (&mut reader).take(LINE).read_line(&mut request).ok()?;
    let mut length = 0usize;
    for _ in 0..HEADERS {
        let mut header = String::new();
        if (&mut reader).take(LINE).read_line(&mut header).ok()? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut parts = request.split_whitespace();
    let (method, target) = (parts.next()?, parts.next()?);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut stream = reader.get_ref().try_clone().ok()?;
    if method == "POST" && ["/smoke", "/smoke-step", "/smoke-zip"].contains(&path) {
        let most = if path == "/smoke-zip" { ZIP } else { REPORT };
        if length > most {
            let _ = stream.write_all(b"HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return Some(Said::Report(format!("fail: the page posted {length} bytes to {path}, more than {most}")));
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        let text = || String::from_utf8_lossy(&body).into_owned();
        return Some(match path {
            "/smoke" => Said::Report(text()),
            "/smoke-step" => Said::Step(text()),
            _ => Said::Zip { bytes: body, name: parameter(query, "name"), file: parameter(query, "file"), node: parameter(query, "node") },
        });
    }
    let (name, file) = site_file(site, path);
    match file.and_then(|file| std::fs::read(file).ok()) {
        Some(bytes) => {
            *served += 1;
            let kind = match name.rsplit('.').next() {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("wasm") => "application/wasm",
                _ => "application/octet-stream",
            };
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(&bytes));
        }
        None => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    }
    None
}

/// The value of a parameter of the part of an address after `?`, percent-decoded, an escape that does not read made `?`;
/// empty when it is not there.
fn parameter(query: &str, name: &str) -> String {
    let Some(value) = query.split('&').find_map(|part| part.strip_prefix(name)?.strip_prefix('=')) else { return String::new() };
    let mut bytes = Vec::new();
    let mut rest = value.bytes();
    while let Some(byte) = rest.next() {
        let hex = |digit: Option<u8>| char::from(digit.unwrap_or(b'x')).to_digit(16).and_then(|digit| u8::try_from(digit).ok());
        match byte {
            b'%' => match (hex(rest.next()), hex(rest.next())) {
                (Some(high), Some(low)) => bytes.push(high * 16 + low),
                _ => bytes.push(b'?'),
            },
            byte => bytes.push(byte),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The most bytes that the zip may unpack to.
const UNPACKED: u64 = 256 * 1024 * 1024;

/// The line of `file` that binds `node` to `CALL` with the wire from its value, as the spec writes a wire (5.1: a named
/// value, an argument of the call, given by position or by the name of its input, `a` of `core.not`).
fn wired(text: &str, node: &str) -> bool {
    let (call, from) = CALL;
    let lines = [format!("{node} = {call}({from})"), format!("{node} = {call}(a: {from})")];
    text.lines().any(|line| lines.iter().any(|wired| line.trim() == wired))
}

/// The name the page saves the zip of a folder under (ide.md 12, `NAME.zip`, `NAME` the name of the folder), for a name
/// with no character that a file name refuses and that names no device of Windows, as `examples` is.
fn zip_name(folder: &Path) -> String {
    format!("{}.zip", folder.file_name().map(|name| name.to_string_lossy()).unwrap_or_default())
}

/// Unpacks the zip of `Download` in memory, refusing an entry whose path would leave the folder it unpacks into, and
/// checks it against `examples/` on the disk: its name is that of the folder; every program file passes `check`, as
/// `labcraft-cli check` checks a folder; the file edited differs from its original and binds the node added to the
/// wire from `in_range`; every other program file of `examples/` is there, byte for byte, and no other;
/// `.labcraft/config.toml`, read as TOML, holds the setting of the project. What it found, or why it fails.
fn check_zip(root: &Path, bytes: &[u8], name: &str, file: &str, node: &str) -> Result<String, String> {
    let expected = zip_name(&root.join("examples"));
    if name != expected {
        return Err(format!("the page saves it as `{name}`, where the folder `examples` gives `{expected}`"));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|error| format!("it does not read: {error}"))?;
    let mut unpacked = BTreeMap::new();
    let mut total = 0u64;
    for at in 0..archive.len() {
        let mut entry = archive.by_index(at).map_err(|error| format!("entry {at} does not read: {error}"))?;
        let name = entry.name().map(|name| name.into_owned()).unwrap_or_default();
        let Some(relative) = entry.enclosed_name() else { return Err(format!("the entry `{name}` leaves the folder")) };
        if entry.is_dir() {
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > UNPACKED {
            return Err(format!("it unpacks to more than {UNPACKED} bytes"));
        }
        let mut data = Vec::new();
        (&mut entry).take(UNPACKED).read_to_end(&mut data).map_err(|error| format!("`{name}` does not read: {error}"))?;
        unpacked.insert(slashed(&relative), data);
    }
    let errors = check_folder(&unpacked);
    if !errors.is_empty() {
        return Err(format!("`check` gives {} errors: {}", errors.len(), errors.join("; ")));
    }
    let examples = examples(&root.join("examples"))?;
    let held: BTreeMap<&str, &Vec<u8>> = unpacked.iter().filter(|(path, _)| is_program(path)).map(|(path, bytes)| (path.as_str(), bytes)).collect();
    let (Some(edited), Some(original)) = (held.get(file), examples.get(file)) else {
        return Err(format!("the file edited, `{file}`, is not both in the zip and in examples/"));
    };
    if *edited == original {
        return Err(format!("`{file}` is the one of examples/: the edit is not there"));
    }
    if node.is_empty() || !wired(&String::from_utf8_lossy(edited), node) {
        return Err(format!("`{file}` has no line `{node} = {}({})`: the node added, wired from `{}`", CALL.0, CALL.1, CALL.1));
    }
    for (path, bytes) in examples.iter().filter(|(path, _)| *path != file) {
        match held.get(path.as_str()) {
            None => return Err(format!("`{path}` of examples/ is not in the zip")),
            Some(held) if *held != bytes => return Err(format!("`{path}` differs from the one of examples/, which the test did not edit")),
            Some(_) => {}
        }
    }
    if let Some(extra) = held.keys().find(|path| !examples.contains_key(**path)) {
        return Err(format!("`{extra}` is in the zip, not in examples/"));
    }
    let config = unpacked.get(".labcraft/config.toml").ok_or("it has no `.labcraft/config.toml`")?;
    let toml = String::from_utf8_lossy(config)
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("`.labcraft/config.toml` does not read: {error}"))?;
    let (table, key, value) = PROJECT_KEY;
    let held = toml.get(table).and_then(|table| table.get(key)).and_then(toml_edit::Item::as_bool);
    if held != Some(value) {
        return Err(format!("`.labcraft/config.toml` does not hold `{key} = {value}` in `[{table}]`: {toml}"));
    }
    Ok(format!(
        "`{name}`, {} entries unpacked in memory; `check` gives no error; `{file}` binds `{node}` to `{}` wired from `{}`; the {} other program files are those of examples/; `.labcraft/config.toml` holds `{key} = {value}` in `[{table}]`",
        archive.len(),
        CALL.0,
        CALL.1,
        examples.len().saturating_sub(1)
    ))
}

/// A `.lcf` or a `.lct` file.
fn is_program(path: &str) -> bool {
    path.ends_with(".lcf") || path.ends_with(".lct")
}

/// A path from the root of the zip, with `/`.
fn slashed(path: &Path) -> String {
    path.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// The program files under `base`, by their paths from it with `/`, as the page bundles a folder: no hidden folder.
fn examples(base: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut found = BTreeMap::new();
    let mut folders = vec![base.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let entries = std::fs::read_dir(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        for entry in entries {
            let path = entry.map_err(|error| format!("{}: {error}", folder.display()))?.path();
            if path.is_dir() {
                if !path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with('.')) {
                    folders.push(path);
                }
                continue;
            }
            let relative = slashed(path.strip_prefix(base).map_err(|_| format!("{} is not under {}", path.display(), base.display()))?);
            if is_program(&relative) {
                let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
                found.insert(relative, bytes);
            }
        }
    }
    Ok(found)
}

/// The errors that `check` gives for a folder of files, with the libraries shipped, in memory: one line each.
fn check_folder(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let memory = labcraft_engine::Memory::new();
    let project = PathBuf::from("/project");
    memory.put_folder(&project);
    memory.set_current(&project);
    for (path, bytes) in files {
        memory.put(project.join(path), bytes.clone());
    }
    let workspace: Arc<dyn labcraft_engine::workspace::Workspace> = Arc::new(memory);
    let folders = [project.clone()];
    let limits = labcraft_types::Limits::default();
    let given = labcraft_engine::folders::Folders { folders: &folders, lib_paths: &[], current: Some(project), limits: &limits };
    let (mut report, _) = labcraft_engine::folders::check_folders(&*workspace, &labcraft_libs::shipped(), &given);
    report.finish();
    report
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.severity() == labcraft_types::Severity::Error)
        .map(|diagnostic| format!("{} {} in {}", diagnostic.code().as_str(), diagnostic.message, diagnostic.file))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_serves_only_the_files_of_the_site() {
        let site = Path::new("site");
        assert_eq!(site_file(site, "/"), ("index.html", Some(site.join("index.html"))));
        assert_eq!(site_file(site, "/snippets/a/b.js").1, Some(site.join("snippets/a/b.js")));
        for path in ["/../Cargo.toml", "/C:/Windows/win.ini", "/a/../../b", "//etc/passwd", "/a\\..\\b", "/./index.html"] {
            assert_eq!(site_file(site, path).1, None, "{path}");
        }
    }

    /// A zip whose entry would land outside the folder it is unpacked into is refused.
    #[test]
    fn an_entry_that_leaves_the_folder_is_refused() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("../outside.lcf", options).unwrap();
        writer.write_all(b"labcraft 1\n").unwrap();
        writer.finish().unwrap();
        let refused = check_zip(Path::new("nowhere"), bytes.get_ref(), "examples.zip", "check_voltage.lcf", "not").unwrap_err();
        assert!(refused.contains("leaves the folder"), "{refused}");
    }

    /// A wire as the spec writes it (5.1): the value as an argument of the call, by position or by the name of the input.
    #[test]
    fn the_wire_added_is_read_as_the_spec_writes_it() {
        assert!(wired("    x = 1\n    not = core.not(in_range)\n", "not"));
        assert!(wired("not = core.not(a: in_range)", "not"));
        assert!(!wired("not = core.not(above)", "not"));
        assert!(!wired("not2 = core.not(in_range)", "not"));
        assert!(!wired("not = core.not(in_range, err: e)", "not"));
        assert_eq!(zip_name(Path::new("/r/examples")), "examples.zip");
    }

    #[test]
    fn the_parameters_of_the_zip_are_read_from_its_address() {
        let query = "file=sub/check%20voltage.lcf&node=not";
        assert_eq!((parameter(query, "file"), parameter(query, "node")), ("sub/check voltage.lcf".to_owned(), "not".to_owned()));
        assert_eq!(parameter(query, "nod"), "");
        assert_eq!(parameter("file=%zz", "file"), "?");
    }
}
