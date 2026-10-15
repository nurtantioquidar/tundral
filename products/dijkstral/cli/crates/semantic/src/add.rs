//! `dijkstral add <url>`: fetch a page into the corpus (`docs/graphify-semantic.md` §11).
//!
//! Safety, as Graphify's: only `http` and `https`; the host is resolved first and the fetch is
//! refused when any address is loopback, private, link-local, unspecified, multicast or otherwise
//! not globally routable. The connection is made to the addresses that were checked (the
//! resolver below is the only one the HTTP agent uses), so a second lookup cannot redirect it,
//! and every redirect hop goes through the same check.
//!
//! The saved file (Graphify's name and format are unobserved, §12.2 item 8) is dijkstral's own:
//! `<dir>/<host>_<path slug>.<ext>`. Text is saved as Markdown (`.md`, HTML as `.html`) with a
//! front-matter block naming the source URL, capture time, author and contributor, which the
//! semantic pass reads; PDFs and images are saved as fetched. The graph is not rebuilt: the next
//! `dijkstral extract` sends the new file to the model.

use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

/// Largest body saved (bytes).
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    /// `--dir` (default `raw` under the working directory).
    pub dir: Option<PathBuf>,
    pub author: Option<String>,
    pub contributor: Option<String>,
    pub timeout: Option<Duration>,
}

/// Refuse anything but `http(s)://host…`; returns `(scheme, host, port)`.
pub fn check_url(url: &str) -> Result<(String, String, u16)> {
    let (scheme, rest) = url
        .split_once("://")
        .map(|(s, r)| (s.to_ascii_lowercase(), r))
        .or_else(|| {
            url.split_once(':')
                .map(|(s, r)| (s.to_ascii_lowercase(), r))
        })
        .context("not a URL")?;
    if scheme != "http" && scheme != "https" {
        bail!("Blocked URL scheme '{scheme}' - only http and https are allowed.");
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']').context("bad IPv6 host")?;
        let port = after.strip_prefix(':').map(str::parse).transpose()?;
        (h.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse()?)),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() {
        bail!("URL has no host");
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Ok((scheme, host, port))
}

/// Not reachable from the public internet: loopback, private, link-local, CGNAT, unspecified,
/// multicast, documentation, unique-local IPv6 and IPv4-mapped forms of those.
pub fn is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked(IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] == 0x2001 && s[1] == 0x0db8)
        }
    }
}

/// Resolve `host:port`, refusing it when any address is blocked.
pub fn resolve_public(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolving {host}"))?
        .collect();
    if addrs.is_empty() {
        bail!("{host} did not resolve");
    }
    if let Some(bad) = addrs.iter().find(|a| is_blocked(a.ip())) {
        bail!(
            "Blocked private/internal IP {} (resolved from '{host}').",
            bad.ip()
        );
    }
    Ok(addrs)
}

/// Fetch `url` and save it under the target directory; returns the saved path.
pub fn add(url: &str, opts: &AddOptions) -> Result<PathBuf> {
    let (_, host, port) = check_url(url)?;
    resolve_public(&host, port)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(opts.timeout.unwrap_or(Duration::from_secs(60)))
        .redirects(5)
        .resolver(|netloc: &str| -> std::io::Result<Vec<SocketAddr>> {
            let (h, p) = netloc
                .rsplit_once(':')
                .and_then(|(h, p)| Some((h.trim_matches(['[', ']']), p.parse().ok()?)))
                .ok_or_else(|| std::io::Error::other("bad address"))?;
            resolve_public(h, p).map_err(|e| std::io::Error::other(e.to_string()))
        })
        .build();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| anyhow::anyhow!("fetching {url}: {e}"))?;
    let final_url = resp.get_url().to_string();
    check_url(&final_url)?;
    let mime = resp
        .content_type()
        .to_ascii_lowercase()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    let mut body = Vec::new();
    resp.into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_BYTES {
        bail!("{url} is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let dir = opts.dir.clone().unwrap_or_else(|| PathBuf::from("raw"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let (ext, text) = match mime.as_str() {
        "application/pdf" => ("pdf", false),
        "image/png" => ("png", false),
        "image/jpeg" => ("jpg", false),
        "image/gif" => ("gif", false),
        "image/webp" => ("webp", false),
        "image/svg+xml" => ("svg", false),
        "text/html" | "application/xhtml+xml" => ("html", true),
        _ => ("md", true),
    };
    let path = dir.join(format!("{}.{ext}", file_stem(&final_url)));
    let bytes = if text && ext == "md" {
        let mut out = front_matter(&final_url, opts).into_bytes();
        out.extend_from_slice(&body);
        out
    } else {
        body
    };
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// `host_path` slugged to `[a-z0-9_]`, at most 100 characters.
pub fn file_stem(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut s = String::new();
    for c in rest.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    let s: String = s.trim_matches('_').chars().take(100).collect();
    if s.is_empty() {
        "page".into()
    } else {
        s
    }
}

fn front_matter(url: &str, opts: &AddOptions) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut fm = format!("---\nsource_url: {url}\ncaptured_at: {now}\n");
    for (k, v) in [("author", &opts.author), ("contributor", &opts.contributor)] {
        if let Some(v) = v {
            fm.push_str(&format!("{k}: {}\n", v.replace('\n', " ")));
        }
    }
    fm.push_str("---\n\n");
    fm
}

/// Where `add` saves by default for a corpus at `root`.
pub fn default_dir(root: &Path) -> PathBuf {
    root.join("raw")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_other_schemes() {
        for url in [
            "file:///etc/passwd",
            "ftp://x.org/a",
            "javascript:alert(1)",
            "FILE:///x",
        ] {
            let e = check_url(url).unwrap_err().to_string();
            assert!(e.contains("Blocked URL scheme"), "{url}: {e}");
        }
        assert_eq!(
            check_url("https://example.org:8443/a?b").unwrap(),
            ("https".into(), "example.org".into(), 8443)
        );
        assert_eq!(
            check_url("http://user@[2001:db8::1]/x").unwrap().1,
            "2001:db8::1"
        );
    }

    #[test]
    fn refuses_private_and_loopback_addresses() {
        for host in [
            "127.0.0.1",
            "localhost",
            "::1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.169.254",
        ] {
            let e = resolve_public(host, 80).unwrap_err().to_string();
            assert!(e.contains("Blocked private/internal IP"), "{host}: {e}");
        }
        for ip in [
            "100.64.0.1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "0.0.0.0",
            "224.0.0.1",
        ] {
            assert!(is_blocked(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "2606:2800:220:1::1"] {
            assert!(!is_blocked(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn add_refuses_before_any_request() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = AddOptions {
            dir: Some(tmp.path().join("raw")),
            ..Default::default()
        };
        assert!(add("http://127.0.0.1:9/x", &opts).is_err());
        assert!(add("file:///etc/hosts", &opts).is_err());
        assert!(!tmp.path().join("raw").exists());
    }

    #[test]
    fn names_files_after_the_url() {
        assert_eq!(
            file_stem("https://Example.org/docs/Guide.html?x=1#y"),
            "example_org_docs_guide_html"
        );
    }
}
