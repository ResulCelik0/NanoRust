//! Reading and writing Word `.docx` packages (no terminal, no file I/O).
//!
//! A [`Document`] is the text (one line per paragraph), a [`Rich`] layer (per-char
//! styles + image table), and everything else needed to avoid degrading the package:
//! unknown parts (`parts`, copied byte-for-byte) and package metadata (`package`:
//! non-image relationships and content-type entries, so styles/numbering/headers stay wired up).
//!
//! Reading conventions:
//! - every `w:p` is one line; paragraphs inside tables are simply lines in document order
//!   (cell and row structure is not kept; tables are flattened to text).
//! - `w:tab` becomes `'\t'`. `w:br`/`w:cr` (a soft line break) **starts a new line**, i.e. the
//!   paragraph is split in two; page/column breaks (`w:type="page"|"column"`) are dropped.
//! - `w:b`/`w:i`/`w:u` in a run's `w:rPr` set styles; `w:val` of `0`, `false`, `off` (and `none`
//!   for `w:u`) turns the style off. Other run properties are not kept.
//! - inline images (`w:drawing` → `a:blip r:embed`, or legacy `v:imagedata r:id`) become one
//!   placeholder char each; the same media part always maps to the same char. Other drawing
//!   content (text boxes) and `mc:Fallback` branches are ignored.
//!
//! Writing regenerates `word/document.xml`; the original paragraph/section properties
//! (`w:pPr`, `w:sectPr`) are not preserved. Chars that are illegal in XML are dropped,
//! and placeholder chars with no image in the table are dropped.

use crate::rich::{ImageTable, Rich, Style};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashSet;
use std::fmt;
use std::io::{Cursor, Read, Write};

const DOC_PATH: &str = "word/document.xml";
const RELS_PATH: &str = "word/_rels/document.xml.rels";
const CT_PATH: &str = "[Content_Types].xml";
const ROOT_RELS_PATH: &str = "_rels/.rels";
const IMAGE_REL_TYPE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const ROOT_RELS: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
    r#"</Relationships>"#
);
const DOC_CT: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";

#[derive(Debug)]
pub enum DocxError {
    /// Not a readable zip archive, or zip writing failed.
    Zip(String),
    /// Malformed XML in a package part.
    Xml(String),
    /// A required package part is absent.
    MissingPart(String),
}

impl fmt::Display for DocxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocxError::Zip(m) => write!(f, "invalid docx (zip): {m}"),
            DocxError::Xml(m) => write!(f, "invalid docx (xml): {m}"),
            DocxError::MissingPart(p) => write!(f, "invalid docx: missing {p}"),
        }
    }
}

impl std::error::Error for DocxError {}

impl From<DocxError> for std::io::Error {
    fn from(e: DocxError) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
    }
}

/// A package part carried through verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub data: Vec<u8>,
}

/// A relationship of `word/document.xml` that is not one of our images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rel {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub target_mode: Option<String>,
}

/// Package metadata kept so passthrough parts stay referenced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    /// `(extension, content type)` defaults, excluding those the writer generates.
    pub defaults: Vec<(String, String)>,
    /// `(part name with leading '/', content type)` overrides, excluding the main document.
    pub overrides: Vec<(String, String)>,
    pub rels: Vec<Rel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// One entry per paragraph; never empty.
    pub lines: Vec<String>,
    /// Styles aligned with `lines` and the image table.
    pub rich: Rich,
    /// Unknown package parts, written back byte-for-byte.
    pub parts: Vec<Part>,
    pub package: Package,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document (one empty paragraph).
    pub fn new() -> Self {
        Document { lines: vec![String::new()], rich: Rich::new(), parts: Vec::new(), package: Package::default() }
    }
}

fn xml_err(e: impl fmt::Display) -> DocxError {
    DocxError::Xml(e.to_string())
}

fn attr(e: &BytesStart, local: &str) -> Result<Option<String>, DocxError> {
    for a in e.attributes() {
        let a = a.map_err(xml_err)?;
        if a.key.local_name().as_ref() == local.as_bytes() {
            return Ok(Some(a.unescape_value().map_err(xml_err)?.into_owned()));
        }
    }
    Ok(None)
}

fn norm_ext(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    }
}

fn builtin_type(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        _ => "application/octet-stream",
    }
}

fn builtin_ext(ct: &str) -> &'static str {
    match ct {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/svg+xml" => "svg",
        "image/x-emf" => "emf",
        "image/x-wmf" => "wmf",
        _ => "bin",
    }
}

/// Resolves a relationship target (relative to `word/`) to a package part name.
fn resolve_target(target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { Vec::new() } else { vec!["word"] };
    for seg in target.trim_start_matches('/').split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn parse_rels(data: &[u8]) -> Result<Vec<Rel>, DocxError> {
    let mut reader = Reader::from_reader(data);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(xml_err)? {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"Relationship" => {
                out.push(Rel {
                    id: attr(&e, "Id")?.unwrap_or_default(),
                    rel_type: attr(&e, "Type")?.unwrap_or_default(),
                    target: attr(&e, "Target")?.unwrap_or_default(),
                    target_mode: attr(&e, "TargetMode")?,
                });
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

type Pairs = Vec<(String, String)>;

fn parse_content_types(data: &[u8]) -> Result<(Pairs, Pairs), DocxError> {
    let mut reader = Reader::from_reader(data);
    let (mut defaults, mut overrides) = (Vec::new(), Vec::new());
    loop {
        match reader.read_event().map_err(xml_err)? {
            Event::Start(e) | Event::Empty(e) => {
                let ct = attr(&e, "ContentType")?.unwrap_or_default();
                match e.local_name().as_ref() {
                    b"Default" => defaults.push((attr(&e, "Extension")?.unwrap_or_default(), ct)),
                    b"Override" => overrides.push((attr(&e, "PartName")?.unwrap_or_default(), ct)),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((defaults, overrides))
}

struct Media<'a> {
    parts: &'a [Part],
    rels: &'a [Rel],
    defaults: &'a Pairs,
    overrides: &'a Pairs,
}

impl Media<'_> {
    fn content_type(&self, part: &str) -> String {
        let slash = format!("/{part}");
        if let Some((_, ct)) = self.overrides.iter().find(|(n, _)| *n == slash) {
            return ct.clone();
        }
        let ext = norm_ext(part);
        if let Some((_, ct)) = self.defaults.iter().find(|(e, _)| e.eq_ignore_ascii_case(&ext)) {
            return ct.clone();
        }
        builtin_type(&ext).to_string()
    }
}

#[derive(Default)]
struct Reading {
    lines: Vec<String>,
    styles: Vec<Vec<Style>>,
    line: String,
    line_styles: Vec<Style>,
    in_run: bool,
    in_rpr: bool,
    in_t: bool,
    skip: usize,
    run: Style,
    images: ImageTable,
    /// media part name -> placeholder char
    loaded: Vec<(String, char)>,
}

impl Reading {
    fn push_char(&mut self, c: char) {
        self.line.push(c);
        self.line_styles.push(self.run);
    }

    fn finish_line(&mut self) {
        self.lines.push(std::mem::take(&mut self.line));
        self.styles.push(std::mem::take(&mut self.line_styles));
    }

    fn embed(&mut self, media: &Media, id: &str) {
        let Some(rel) = media.rels.iter().find(|r| r.id == id && r.target_mode.as_deref() != Some("External")) else {
            return;
        };
        let part = resolve_target(&rel.target);
        let c = if let Some((_, c)) = self.loaded.iter().find(|(p, _)| *p == part) {
            *c
        } else {
            let Some(p) = media.parts.iter().find(|p| p.name == part) else { return };
            let name = part.rsplit('/').next().unwrap_or(&part).to_string();
            let Some(c) = self.images.add_image(&name, p.data.clone(), &media.content_type(&part)) else {
                return;
            };
            self.loaded.push((part, c));
            c
        };
        self.push_char(c);
    }

    fn start(&mut self, media: &Media, e: &BytesStart) -> Result<(), DocxError> {
        let name = e.local_name();
        let name = name.as_ref();
        match name {
            b"blip" => {
                if self.in_run {
                    if let Some(id) = attr(e, "embed")? {
                        self.embed(media, &id);
                    }
                }
            }
            b"imagedata" => {
                if self.in_run {
                    if let Some(id) = attr(e, "id")? {
                        self.embed(media, &id);
                    }
                }
            }
            _ => {}
        }
        if matches!(name, b"drawing" | b"pict" | b"Fallback" | b"rPrChange") {
            self.skip += 1;
            return Ok(());
        }
        if self.skip > 0 {
            return Ok(());
        }
        match name {
            b"r" => {
                self.in_run = true;
                self.run = Style::NONE;
            }
            b"rPr" if self.in_run => self.in_rpr = true,
            b"b" | b"i" | b"u" if self.in_rpr => {
                let flag = match name {
                    b"b" => Style::BOLD,
                    b"i" => Style::ITALIC,
                    _ => Style::UNDERLINE,
                };
                let off = attr(e, "val")?.is_some_and(|v| {
                    matches!(v.as_str(), "0" | "false" | "off") || (name == b"u" && v == "none")
                });
                self.run = if off { self.run.without(flag) } else { self.run.with(flag) };
            }
            b"t" if self.in_run => self.in_t = true,
            b"tab" if self.in_run && !self.in_rpr => self.push_char('\t'),
            b"br" | b"cr" if self.in_run && !self.in_rpr => {
                let kind = attr(e, "type")?;
                if !matches!(kind.as_deref(), Some("page") | Some("column")) {
                    self.finish_line();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, name: &[u8]) {
        if matches!(name, b"drawing" | b"pict" | b"Fallback" | b"rPrChange") {
            self.skip = self.skip.saturating_sub(1);
            return;
        }
        if self.skip > 0 {
            return;
        }
        match name {
            b"p" => self.finish_line(),
            b"r" => {
                self.in_run = false;
                self.in_rpr = false;
                self.in_t = false;
            }
            b"rPr" => self.in_rpr = false,
            b"t" => self.in_t = false,
            _ => {}
        }
    }

    fn text(&mut self, s: &str) {
        if self.in_t && self.skip == 0 {
            for c in s.chars() {
                self.push_char(c);
            }
        }
    }
}

/// Parses a `.docx` package.
pub fn read(bytes: &[u8]) -> Result<Document, DocxError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| DocxError::Zip(e.to_string()))?;
    let mut all: Vec<Part> = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| DocxError::Zip(e.to_string()))?;
        if f.is_dir() {
            continue;
        }
        let name = f.name().to_string();
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| DocxError::Zip(e.to_string()))?;
        all.push(Part { name, data });
    }
    let find = |n: &str| all.iter().find(|p| p.name == n).map(|p| p.data.as_slice());
    let doc_xml = find(DOC_PATH).ok_or_else(|| DocxError::MissingPart(DOC_PATH.to_string()))?;
    let rels = match find(RELS_PATH) {
        Some(d) => parse_rels(d)?,
        None => Vec::new(),
    };
    let (defaults, overrides) = match find(CT_PATH) {
        Some(d) => parse_content_types(d)?,
        None => (Vec::new(), Vec::new()),
    };

    let media = Media { parts: &all, rels: &rels, defaults: &defaults, overrides: &overrides };
    let mut st = Reading::default();
    let mut reader = Reader::from_reader(doc_xml);
    loop {
        match reader.read_event().map_err(xml_err)? {
            Event::Start(e) => st.start(&media, &e)?,
            Event::Empty(e) => {
                st.start(&media, &e)?;
                st.end(e.local_name().as_ref());
            }
            Event::End(e) => st.end(e.local_name().as_ref()),
            Event::Text(t) => {
                let s = t.unescape().map_err(xml_err)?;
                st.text(&s);
            }
            Event::CData(t) => {
                let s = String::from_utf8_lossy(&t).into_owned();
                st.text(&s);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if st.lines.is_empty() {
        st.lines.push(String::new());
        st.styles.push(Vec::new());
    }

    let loaded = std::mem::take(&mut st.loaded);
    let is_loaded = |n: &str| loaded.iter().any(|(p, _)| p == n);
    let mut parts = Vec::new();
    for p in &all {
        let skip = matches!(p.name.as_str(), DOC_PATH | RELS_PATH | CT_PATH)
            || is_loaded(&p.name)
            || (p.name == ROOT_RELS_PATH && p.data == ROOT_RELS.as_bytes());
        if !skip {
            parts.push(p.clone());
        }
    }
    let image_of = |ext: &str, ct: &str| {
        loaded.iter().any(|(p, c)| {
            norm_ext(p) == ext.to_ascii_lowercase() && st.images.get(*c).is_some_and(|i| i.content_type == ct)
        })
    };
    let package = Package {
        defaults: defaults
            .iter()
            .filter(|(e, ct)| !matches!(e.to_ascii_lowercase().as_str(), "rels" | "xml") || *ct != xml_default_type(e))
            .filter(|(e, ct)| !image_of(e, ct))
            .cloned()
            .collect(),
        overrides: overrides
            .iter()
            .filter(|(n, ct)| {
                let part = n.trim_start_matches('/');
                part != DOC_PATH && !(is_loaded(part) && st.images.get(loaded.iter().find(|(p, _)| p == part).map_or('\0', |l| l.1)).is_some_and(|i| i.content_type == *ct))
            })
            .cloned()
            .collect(),
        rels: rels
            .iter()
            .filter(|r| !(r.target_mode.as_deref() != Some("External") && is_loaded(&resolve_target(&r.target))))
            .cloned()
            .collect(),
    };

    let text = st.lines.join("\n");
    let mut rich = Rich::from_text(&text);
    for (i, s) in st.styles.into_iter().enumerate() {
        rich.set_line_styles(i, s);
    }
    rich.images = st.images;
    Ok(Document { lines: st.lines, rich, parts, package })
}

fn xml_default_type(ext: &str) -> &'static str {
    if ext.eq_ignore_ascii_case("rels") {
        "application/vnd.openxmlformats-package.relationships+xml"
    } else {
        "application/xml"
    }
}

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
}

fn xml_legal(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

/// Pixel size of a PNG/GIF/JPEG, if recognisable.
fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() >= 24 && b.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_be_bytes(b[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(b[20..24].try_into().ok()?);
        return Some((w, h));
    }
    if b.len() >= 10 && b.starts_with(b"GIF8") {
        return Some((u16::from_le_bytes([b[6], b[7]]) as u32, u16::from_le_bytes([b[8], b[9]]) as u32));
    }
    if b.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = b[i + 1];
            if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                return Some((w, h));
            }
            if m == 0xFF || m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
                i += if m == 0xFF { 1 } else { 2 };
                continue;
            }
            i += 2 + u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        }
    }
    None
}

fn extent(bytes: &[u8]) -> (u64, u64) {
    const PX: u64 = 9525;
    const MAX_W: u64 = 5_760_000;
    match image_size(bytes) {
        Some((w, h)) if w > 0 && h > 0 => {
            let (mut cx, mut cy) = (w as u64 * PX, h as u64 * PX);
            if cx > MAX_W {
                cy = cy * MAX_W / cx;
                cx = MAX_W;
            }
            (cx, cy.max(1))
        }
        _ => (1_905_000, 1_905_000),
    }
}

struct MediaOut {
    part: String,
    rel_id: String,
}

fn clean_name(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('.') && !n.contains(['/', '\\']) && !n.chars().any(|c| c.is_control())
}

fn run_props(s: Style, out: &mut String) {
    if s.is_empty() {
        return;
    }
    out.push_str("<w:rPr>");
    if s.contains(Style::BOLD) {
        out.push_str("<w:b/>");
    }
    if s.contains(Style::ITALIC) {
        out.push_str("<w:i/>");
    }
    if s.contains(Style::UNDERLINE) {
        out.push_str(r#"<w:u w:val="single"/>"#);
    }
    out.push_str("</w:rPr>");
}

#[derive(PartialEq)]
enum Piece {
    Text(char),
    Image(usize),
}

fn paragraph(line: &str, styles: &[Style], rich: &Rich, media: &[MediaOut], out: &mut String) {
    out.push_str("<w:p>");
    let mut pieces: Vec<(Piece, Style)> = Vec::new();
    for (i, c) in line.chars().enumerate() {
        let st = styles.get(i).copied().unwrap_or_default();
        if let Some(idx) = ImageTable::index_of(c) {
            if idx < rich.images.len() {
                pieces.push((Piece::Image(idx), st));
            }
        } else if xml_legal(c) && c != '\n' && c != '\r' {
            pieces.push((Piece::Text(c), st));
        }
    }
    let mut i = 0;
    while i < pieces.len() {
        let style = pieces[i].1;
        out.push_str("<w:r>");
        run_props(style, out);
        match pieces[i].0 {
            Piece::Image(idx) => {
                drawing(idx, rich, &media[idx], out);
                i += 1;
            }
            Piece::Text(_) => {
                let mut buf = String::new();
                let flush = |buf: &mut String, out: &mut String| {
                    if !buf.is_empty() {
                        out.push_str(r#"<w:t xml:space="preserve">"#);
                        esc(buf, out);
                        out.push_str("</w:t>");
                        buf.clear();
                    }
                };
                while i < pieces.len() && pieces[i].1 == style {
                    match pieces[i].0 {
                        Piece::Text('\t') => {
                            flush(&mut buf, out);
                            out.push_str("<w:tab/>");
                        }
                        Piece::Text(c) => buf.push(c),
                        Piece::Image(_) => break,
                    }
                    i += 1;
                }
                flush(&mut buf, out);
            }
        }
        out.push_str("</w:r>");
    }
    out.push_str("</w:p>");
}

fn drawing(idx: usize, rich: &Rich, m: &MediaOut, out: &mut String) {
    let img = &rich.images.images()[idx];
    let (cx, cy) = extent(&img.bytes);
    let mut name = String::new();
    esc(&img.name, &mut name);
    let id = idx + 1;
    out.push_str(&format!(
        concat!(
            r#"<w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">"#,
            r#"<wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="{name}"/>"#,
            r#"<wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">"#,
            r#"<pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr>"#,
            r#"<pic:blipFill><a:blip r:embed="{rid}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>"#,
            r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>"#,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>"#,
            r#"</a:graphicData></a:graphic></wp:inline></w:drawing>"#
        ),
        cx = cx,
        cy = cy,
        id = id,
        name = name,
        rid = m.rel_id
    ));
}

/// Serialises a document to a `.docx` package.
pub fn write(doc: &Document) -> Result<Vec<u8>, DocxError> {
    let zerr = |e: zip::result::ZipError| DocxError::Zip(e.to_string());
    let ioerr = |e: std::io::Error| DocxError::Zip(e.to_string());

    // Reserved names: passthrough parts, so generated media never collide.
    let part_names: HashSet<String> = doc.parts.iter().map(|p| p.name.to_ascii_lowercase()).collect();
    let used_ids: HashSet<&str> = doc.package.rels.iter().map(|r| r.id.as_str()).collect();
    let mut taken: HashSet<String> = HashSet::new();
    let mut media = Vec::new();
    for (n, img) in doc.rich.images.images().iter().enumerate() {
        let free = |name: &str, taken: &HashSet<String>| {
            let p = format!("word/media/{name}").to_ascii_lowercase();
            !taken.contains(&p) && !part_names.contains(&p)
        };
        let mut name = img.name.clone();
        if !clean_name(&name) || !free(&name, &taken) {
            let ext = builtin_ext(&img.content_type);
            let mut k = n + 1;
            loop {
                name = format!("image{k}.{ext}");
                if free(&name, &taken) {
                    break;
                }
                k += 1;
            }
        }
        let part = format!("word/media/{name}");
        taken.insert(part.to_ascii_lowercase());
        let mut k = n + 1;
        let mut rel_id = format!("rIdImg{k}");
        while used_ids.contains(rel_id.as_str()) {
            k += 1000;
            rel_id = format!("rIdImg{k}");
        }
        media.push(MediaOut { part, rel_id });
    }

    let mut body = String::new();
    body.push_str(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
        r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
        r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
        r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
        r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>"#
    ));
    let empty: Vec<Style> = Vec::new();
    let line_count = doc.lines.len().max(1);
    for i in 0..line_count {
        let line = doc.lines.get(i).map_or("", String::as_str);
        let styles = doc.rich.lines().get(i).unwrap_or(&empty);
        paragraph(line, styles, &doc.rich, &media, &mut body);
    }
    body.push_str("</w:body></w:document>");

    // Relationships.
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for r in &doc.package.rels {
        rels.push_str("<Relationship Id=\"");
        esc(&r.id, &mut rels);
        rels.push_str("\" Type=\"");
        esc(&r.rel_type, &mut rels);
        rels.push_str("\" Target=\"");
        esc(&r.target, &mut rels);
        rels.push('"');
        if let Some(m) = &r.target_mode {
            rels.push_str(" TargetMode=\"");
            esc(m, &mut rels);
            rels.push('"');
        }
        rels.push_str("/>");
    }
    for m in &media {
        rels.push_str(&format!(
            r#"<Relationship Id="{}" Type="{}" Target="media/"#,
            m.rel_id, IMAGE_REL_TYPE
        ));
        esc(m.part.trim_start_matches("word/media/"), &mut rels);
        rels.push_str("\"/>");
    }
    rels.push_str("</Relationships>");

    // Content types.
    let mut defaults: Pairs = vec![
        ("rels".into(), xml_default_type("rels").into()),
        ("xml".into(), xml_default_type("xml").into()),
    ];
    for (e, ct) in &doc.package.defaults {
        if !defaults.iter().any(|(x, _)| x.eq_ignore_ascii_case(e)) {
            defaults.push((e.clone(), ct.clone()));
        }
    }
    let mut overrides: Pairs = Vec::new();
    for (i, img) in doc.rich.images.images().iter().enumerate() {
        let part = &media[i].part;
        let ext = norm_ext(part);
        match defaults.iter().find(|(x, _)| x.eq_ignore_ascii_case(&ext)) {
            Some((_, ct)) if *ct == img.content_type => {}
            None if !ext.is_empty() => defaults.push((ext, img.content_type.clone())),
            _ => overrides.push((format!("/{part}"), img.content_type.clone())),
        }
    }
    overrides.push((format!("/{DOC_PATH}"), DOC_CT.to_string()));
    for (n, ct) in &doc.package.overrides {
        let p = n.trim_start_matches('/');
        if part_names.contains(&p.to_ascii_lowercase()) && !overrides.iter().any(|(x, _)| x == n) {
            overrides.push((n.clone(), ct.clone()));
        }
    }
    let mut cts = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
    );
    for (e, ct) in &defaults {
        cts.push_str("<Default Extension=\"");
        esc(e, &mut cts);
        cts.push_str("\" ContentType=\"");
        esc(ct, &mut cts);
        cts.push_str("\"/>");
    }
    for (n, ct) in &overrides {
        cts.push_str("<Override PartName=\"");
        esc(n, &mut cts);
        cts.push_str("\" ContentType=\"");
        esc(ct, &mut cts);
        cts.push_str("\"/>");
    }
    cts.push_str("</Types>");

    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::FileOptions::default();
    let mut put = |name: &str, data: &[u8]| -> Result<(), DocxError> {
        zw.start_file(name, opts).map_err(zerr)?;
        zw.write_all(data).map_err(ioerr)
    };
    put(CT_PATH, cts.as_bytes())?;
    if !doc.parts.iter().any(|p| p.name == ROOT_RELS_PATH) {
        put(ROOT_RELS_PATH, ROOT_RELS.as_bytes())?;
    }
    put(DOC_PATH, body.as_bytes())?;
    put(RELS_PATH, rels.as_bytes())?;
    for (i, img) in doc.rich.images.images().iter().enumerate() {
        put(&media[i].part, &img.bytes)?;
    }
    for p in &doc.parts {
        if matches!(p.name.as_str(), CT_PATH | DOC_PATH | RELS_PATH) {
            continue;
        }
        put(&p.name, &p.data)?;
    }
    Ok(zw.finish().map_err(zerr)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d) in entries {
            zw.start_file(*n, zip::write::FileOptions::default()).unwrap();
            zw.write_all(d).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

    fn fixture(body: &str) -> Vec<u8> {
        let doc = format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
</Relationships>"#;
        let ct = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
        build(&[
            ("[Content_Types].xml", ct.as_bytes()),
            ("_rels/.rels", b"<Relationships/>"),
            ("word/document.xml", doc.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/styles.xml", b"<w:styles/>\x00\xff"),
            ("word/media/image1.png", b"\x89PNGfake"),
            ("word/media/other.bin", b"unused"),
        ])
    }

    #[test]
    fn reads_text_styles_and_image() {
        let body = r#"<w:p><w:r><w:rPr><w:b/><w:u w:val="single"/></w:rPr><w:t>Bo</w:t></w:r><w:r><w:rPr><w:i w:val="0"/><w:b w:val="false"/></w:rPr><w:t xml:space="preserve"> x &amp; y</w:t><w:tab/><w:t>z</w:t></w:r></w:p>
<w:p/>
<w:p><w:pPr><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:drawing><wp:inline xmlns:wp="x"><a:graphic><a:blip r:embed="rId5"/></a:graphic></wp:inline></w:drawing></w:r><w:r><w:t>a</w:t><w:br/><w:t>b</w:t><w:br w:type="page"/></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        let d = read(&fixture(body)).unwrap();
        let img = d.rich.images.get('\u{E000}').unwrap();
        assert_eq!(img.name, "image1.png");
        assert_eq!(img.content_type, "image/png");
        assert_eq!(img.bytes, b"\x89PNGfake");
        assert_eq!(d.lines, vec!["Bo x & y\tz", "", "\u{E000}a", "b", "cell"]);
        let bu = Style::BOLD.with(Style::UNDERLINE);
        assert_eq!(d.rich.lines()[0][..2], [bu, bu]);
        assert!(d.rich.lines()[0][2..].iter().all(|s| s.is_empty()));
        assert_eq!(d.rich.lines()[2], vec![Style::NONE, Style::NONE]);
        // unknown parts pass through; the image media does not (it's in the table)
        let names: Vec<&str> = d.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["_rels/.rels", "word/styles.xml", "word/media/other.bin"]);
        assert_eq!(d.package.rels.len(), 1);
    }

    #[test]
    fn round_trip_identity() {
        let mut d = Document::new();
        d.lines = vec!["héllo 日本語 😀".into(), "".into(), "<a href=\"x\">&amp; 'q'</a>".into(), "  sp\ttab  ".into(), "".into()];
        let mut rich = Rich::from_text(&d.lines.join("\n"));
        let n0 = d.lines[0].chars().count();
        let mut s0 = vec![Style::NONE; n0];
        s0[0] = Style::BOLD;
        s0[1] = Style::BOLD.with(Style::ITALIC);
        s0[8] = Style::UNDERLINE;
        rich.set_line_styles(0, s0);
        let c = rich.images.add_image("pic.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x03rest".to_vec(), "image/png").unwrap();
        let c2 = rich.images.add_image("p.jpg", vec![1, 2, 3], "image/jpeg").unwrap();
        d.lines[3] = format!("  sp\t{c}{c2}{c}");
        rich.set_line_styles(3, vec![Style::ITALIC; 8]);
        d.rich = Rich::from_text(&d.lines.join("\n"));
        for (i, l) in rich.lines().iter().enumerate() {
            if i != 3 {
                d.rich.set_line_styles(i, l.clone());
            }
        }
        d.rich.set_line_styles(3, vec![Style::ITALIC; 8]);
        d.rich.images = rich.images.clone();
        d.parts.push(Part { name: "word/styles.xml".into(), data: vec![9, 8, 7] });
        d.package.rels.push(Rel {
            id: "rId1".into(),
            rel_type: "http://x/styles".into(),
            target: "styles.xml".into(),
            target_mode: None,
        });
        d.package.overrides.push(("/word/styles.xml".into(), "application/x-styles".into()));
        let bytes = write(&d).unwrap();
        let back = read(&bytes).unwrap();
        assert_eq!(back, d);
        assert_eq!(write(&back).unwrap().len(), bytes.len());
    }

    #[test]
    fn writes_valid_package() {
        let mut d = Document::new();
        d.lines = vec!["a".into()];
        d.rich = Rich::from_text("a");
        d.rich.images.add_image("x.png", vec![1], "image/png");
        let bytes = write(&d).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for n in ["[Content_Types].xml", "_rels/.rels", "word/document.xml", "word/_rels/document.xml.rels", "word/media/x.png"] {
            z.by_name(n).unwrap();
        }
        let mut s = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        assert!(s.contains(r#"<w:t xml:space="preserve">a</w:t>"#));
    }

    #[test]
    fn unknown_part_byte_identical_and_unknown_image_dropped() {
        let d = read(&fixture("<w:p><w:r><w:t>hi</w:t></w:r></w:p>")).unwrap();
        let mut d2 = d.clone();
        d2.lines = vec!["a\u{E005}b\u{1}c".into()];
        d2.rich = Rich::from_text(&d2.lines[0]);
        let back = read(&write(&d2).unwrap()).unwrap();
        assert_eq!(back.lines, vec!["abc"]);
        let get = |d: &Document, n: &str| d.parts.iter().find(|p| p.name == n).map(|p| p.data.clone());
        assert_eq!(get(&back, "word/styles.xml").unwrap(), b"<w:styles/>\x00\xff");
        assert_eq!(get(&back, "word/media/other.bin").unwrap(), b"unused");
        assert_eq!(back.package, d.package);
    }

    #[test]
    fn errors() {
        assert!(matches!(read(b"definitely not a zip"), Err(DocxError::Zip(_))));
        assert!(matches!(read(&[]), Err(DocxError::Zip(_))));
        let no_doc = build(&[("word/styles.xml", b"x")]);
        assert!(matches!(read(&no_doc), Err(DocxError::MissingPart(_))));
        let bad_xml = build(&[("word/document.xml", b"<w:document><w:p></w:document>")]);
        assert!(matches!(read(&bad_xml), Err(DocxError::Xml(_))));
        let mut truncated = fixture("<w:p/>");
        truncated.truncate(truncated.len() / 2);
        assert!(read(&truncated).is_err());
    }
}
