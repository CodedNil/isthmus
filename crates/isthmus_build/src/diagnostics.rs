//! Relates errors in the extracted shader crate to the original Rust item.
use anyhow::Result;
use proc_macro2::LineColumn;
use rustix::{fd::OwnedFd, io::dup, stdio::dup2_stderr};
use std::{
    collections::BTreeSet,
    fs,
    io::{Seek as _, Write as _, read_to_string, stderr},
    path::Path,
};
use syn::spanned::Spanned as _;

const ORIGIN: &str = "Isthmus origin: ";

pub fn origin(path: &Path, position: LineColumn, label: &str) -> String {
    format!("{ORIGIN}{}:{}:{} ({label})", path.display(), position.line, position.column + 1)
}

struct RestoreStderr(OwnedFd);

impl Drop for RestoreStderr {
    fn drop(&mut self) {
        let _restored = dup2_stderr(&self.0);
    }
}

pub fn compile<T>(source: &Path, build: impl FnOnce() -> Result<T>) -> Result<T> {
    let mut capture = tempfile::tempfile()?;
    let original = RestoreStderr(dup(stderr())?);
    dup2_stderr(&capture)?;
    let result = build();
    drop(original);

    capture.rewind()?;
    let log = read_to_string(capture)?;
    stderr().write_all(log.as_bytes())?;
    if result.is_err() {
        let mut seen = BTreeSet::new();
        for line in log.lines() {
            if let Some(origin) = source_location(source, line)
                && seen.insert(origin.clone())
            {
                writeln!(stderr(), "  = Original Rust: {origin}")?;
            }
        }
    }
    result
}

fn source_location(root: &Path, line: &str) -> Option<String> {
    let position = line.trim_start().strip_prefix("--> ").or_else(|| line.trim_start().strip_prefix("::: "))?;
    let (position, _) = position.rsplit_once(':')?;
    let (path, row) = position.rsplit_once(':')?;
    let file = root.join(path).canonicalize().ok()?;
    if !file.starts_with(root.canonicalize().ok()?) {
        return None;
    }
    let row = row.trim().parse::<usize>().ok()?;
    let text = fs::read_to_string(file).ok()?;
    let ast = syn::parse_file(&text).ok()?;
    let start = ast.items.iter().find_map(|item| {
        let span = item.span();
        let start = span.start().line;
        (start <= row && row <= span.end().line).then_some(start)
    })?;
    text.lines()
        .take(row)
        .skip(start.saturating_sub(1))
        .filter_map(|line| line.trim_start().strip_prefix("///")?.trim_start().strip_prefix(ORIGIN))
        .last()
        .map(str::to_owned)
}
