//! EPUB opening, metadata, and chapter text extraction.

use crate::text::html_to_text;
use anyhow::{Context, Result, bail};
use epub::doc::{EpubDoc, NavPoint};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub struct Book {
    doc: EpubDoc<BufReader<File>>,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ChapterInfo {
    pub index: usize,
    pub id: String,
    pub title: Option<String>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct BookMeta {
    pub title: Option<String>,
    pub creators: Vec<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub chapter_count: usize,
}

impl Book {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let doc = EpubDoc::new(&path)
            .with_context(|| format!("failed to open EPUB: {}", path.display()))?;
        Ok(Self { doc, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn meta(&self) -> BookMeta {
        BookMeta {
            title: self.doc.get_title(),
            creators: self.metadata_values("creator"),
            language: self.metadata_first("language"),
            publisher: self.metadata_first("publisher"),
            identifier: self
                .doc
                .unique_identifier
                .clone()
                .or_else(|| self.metadata_first("identifier")),
            description: self.metadata_first("description"),
            chapter_count: self.doc.get_num_chapters(),
        }
    }

    fn metadata_first(&self, property: &str) -> Option<String> {
        self.doc.mdata(property).map(|m| m.value.clone())
    }

    fn metadata_values(&self, property: &str) -> Vec<String> {
        self.doc
            .metadata
            .iter()
            .filter(|m| m.property == property)
            .map(|m| m.value.clone())
            .collect()
    }

    pub fn chapters(&self) -> Vec<ChapterInfo> {
        self.doc
            .spine
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let id = item.idref.clone();
                let path = self.doc.resources.get(&id).map(|r| r.path.clone());
                let title = path
                    .as_ref()
                    .and_then(|p| find_toc_title(&self.doc.toc, p));

                ChapterInfo {
                    index,
                    id,
                    title,
                    path,
                }
            })
            .collect()
    }

    /// Extract plain text for a spine chapter (0-based index).
    pub fn chapter_text(&mut self, index: usize) -> Result<String> {
        let n = self.doc.get_num_chapters();
        if index >= n {
            bail!("chapter index {index} out of range (0..{n})");
        }
        if !self.doc.set_current_chapter(index) {
            bail!("failed to select chapter {index}");
        }
        let (xhtml, _mime) = self
            .doc
            .get_current_str()
            .with_context(|| format!("missing content for chapter {index}"))?;
        Ok(html_to_text(&xhtml))
    }
}

fn find_toc_title(toc: &[NavPoint], path: &Path) -> Option<String> {
    let mut found = None;
    walk_toc(toc, &mut |nav| {
        if found.is_some() {
            return;
        }
        let nav_path = strip_fragment(&nav.content);
        if paths_match(&nav_path, path) {
            found = Some(nav.label.clone());
        }
    });
    found
}

fn strip_fragment(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    PathBuf::from(s.split('#').next().unwrap_or(&s))
}

fn paths_match(a: &Path, b: &Path) -> bool {
    a == b || a.ends_with(b) || b.ends_with(a)
}

fn walk_toc(toc: &[NavPoint], f: &mut dyn FnMut(&NavPoint)) {
    for nav in toc {
        f(nav);
        walk_toc(&nav.children, f);
    }
}
