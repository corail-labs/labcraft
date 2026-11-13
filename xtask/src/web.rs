//! `cargo xtask web` builds the web viewer (`docs/architecture.md`, 11.5) with Trunk into `target/web`, after clippy for
//! WASM; `cargo xtask web-smoke` serves that folder on the loopback and opens it in headless Chrome, whose page says how
//! it went (`?smoke`): it drew a function, trapped its worker, started a new one and drew again; `cargo xtask web-serve`
//! serves it, to try it in a browser. None is part of `ci`, which needs neither Trunk nor a browser: the job `web` of the
//! workflow `checks` runs the first two.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The version of Trunk that the job `web` installs, as SolidCraft and GridCraft pin it.
pub const TRUNK: &str = "0.21.14";

/// What the page must give, besides itself.
const BUILT: [&str; 5] = ["index.html", "labcraft-web_bg.wasm", "labcraft-web.js", "labcraft-worker_bg.wasm", "labcraft-worker_loader.js"];

/// The longest line of a request, and the most headers, that the server reads.
const LINE: u64 = 8 * 1024;
const HEADERS: usize = 64;

/// How long the smoke test waits for the page to report.
const PATIENCE: Duration = Duration::from_secs(180);

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
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(report) = answer(stream, &site, &mut served) {
                    println!("web-smoke: {report} ({served} files served)");
                    return if report.starts_with("ok:") { Ok(()) } else { Err(format!("web-smoke: {report}")) };
                }
            }
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

/// Answers one request: a file of the site, or the report of the page, which it returns.
fn answer(stream: TcpStream, site: &Path, served: &mut usize) -> Option<String> {
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
    let path = target.split('?').next().unwrap_or("/");
    let mut stream = reader.get_ref().try_clone().ok()?;
    if method == "POST" && path == "/smoke" {
        let mut body = vec![0; length.min(64 * 1024)];
        reader.read_exact(&mut body).ok()?;
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        return Some(String::from_utf8_lossy(&body).into_owned());
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
}
